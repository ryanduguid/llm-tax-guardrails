"""URL collection and outcome classification are tested offline with a
monkeypatched urllib - no live network calls. Liveness is CI/local-only."""
import socket
import threading
import urllib.error
import urllib.request

from drdebits_build import linkcheck
from drdebits_build.build import write_outputs
from drdebits_build.linkcheck import collect_urls

from tests.test_build import make_repo


def test_collect_urls_unique_ordered(tmp_path):
    root = make_repo(tmp_path)
    (root / "README.md").write_text(
        "See https://x.invalid/a and https://x.invalid/a again\n", encoding="utf-8", newline="\n")
    (root / "MAINTENANCE.md").write_text("None here\n", encoding="utf-8", newline="\n")
    write_outputs(root, root)
    urls = collect_urls(root)
    assert urls.count("https://x.invalid/a") == 1


class _FakeResponse:
    def __init__(self, status):
        self.status = status

    def geturl(self):
        return "https://x.invalid/"

    def __enter__(self):
        return self

    def __exit__(self, *exc_info):
        return False


def test_check_404_is_dead_and_not_retried(monkeypatch):
    calls = []

    def fake_urlopen(req, timeout):
        calls.append(1)
        raise urllib.error.HTTPError(req.full_url, 404, "Not Found", None, None)

    monkeypatch.setattr(urllib.request, "urlopen", fake_urlopen)
    kind, detail = linkcheck.check("https://x.invalid/gone", 5)
    assert (kind, detail) == ("dead", "404")
    assert len(calls) == 1  # definitive death: no retry


def test_check_403_is_unreachable_after_one_retry(monkeypatch):
    calls = []

    def fake_urlopen(req, timeout):
        calls.append(1)
        raise urllib.error.HTTPError(req.full_url, 403, "Forbidden", None, None)

    monkeypatch.setattr(urllib.request, "urlopen", fake_urlopen)
    kind, detail = linkcheck.check("https://x.invalid/blocked", 5)
    assert (kind, detail) == ("unreachable", "403")
    assert len(calls) == 2  # one retry attempted before classification stuck


def test_check_timeout_then_success_is_ok(monkeypatch):
    calls = []

    def fake_urlopen(req, timeout):
        calls.append(1)
        if len(calls) == 1:
            raise TimeoutError("read operation timed out")
        return _FakeResponse(200)

    monkeypatch.setattr(urllib.request, "urlopen", fake_urlopen)
    kind, detail = linkcheck.check("https://x.invalid/flaky", 5)
    assert (kind, detail) == ("ok", "200")
    assert len(calls) == 2  # retry succeeded


def test_check_dns_failure_is_dead(monkeypatch):
    calls = []

    def fake_urlopen(req, timeout):
        calls.append(1)
        raise urllib.error.URLError(
            socket.gaierror(socket.EAI_NONAME, "Name or service not known"))

    monkeypatch.setattr(urllib.request, "urlopen", fake_urlopen)
    kind, detail = linkcheck.check("https://x.invalid/nxdomain", 5)
    assert (kind, detail) == ("dead", "gaierror")
    assert len(calls) == 1  # definitive death: no retry


def test_check_transient_dns_failure_is_unreachable_and_retried(monkeypatch):
    """EAI_AGAIN is a resolver hiccup, not link rot: it must land in the
    unreachable bucket (exit 0, no SOURCE CURRENCY issue) and get the retry."""
    calls = []

    def fake_urlopen(req, timeout):
        calls.append(1)
        raise urllib.error.URLError(
            socket.gaierror(socket.EAI_AGAIN, "Temporary failure in name resolution"))

    monkeypatch.setattr(urllib.request, "urlopen", fake_urlopen)
    kind, detail = linkcheck.check("https://x.invalid/flaky-dns", 5)
    assert (kind, detail) == ("unreachable", "gaierror")
    assert len(calls) == 2  # one retry attempted before classification stuck


def test_non_https_url_is_never_fetched(monkeypatch):
    """Scheme guard: anything but https:// must classify without touching
    urlopen, so a collector change can never make this fetch local schemes."""
    def fake_urlopen(req, timeout):
        raise AssertionError("urlopen must not be called for non-https URLs")

    monkeypatch.setattr(urllib.request, "urlopen", fake_urlopen)
    kind, detail = linkcheck.check("file:///C:/Windows/win.ini", 5)
    assert (kind, detail) == ("unreachable", "non-https")


def test_check_url_error_timeout_reason_is_unreachable(monkeypatch):
    def fake_urlopen(req, timeout):
        raise urllib.error.URLError(TimeoutError("timed out"))

    monkeypatch.setattr(urllib.request, "urlopen", fake_urlopen)
    kind, detail = linkcheck.check("https://x.invalid/slow", 5)
    assert (kind, detail) == ("unreachable", "timeout")


def test_check_connection_reset_is_unreachable_by_class_name(monkeypatch):
    def fake_urlopen(req, timeout):
        raise ConnectionResetError("connection reset by peer")

    monkeypatch.setattr(urllib.request, "urlopen", fake_urlopen)
    kind, detail = linkcheck.check("https://x.invalid/reset", 5)
    assert (kind, detail) == ("unreachable", "ConnectionResetError")


def test_check_500_is_unreachable(monkeypatch):
    def fake_urlopen(req, timeout):
        raise urllib.error.HTTPError(req.full_url, 503, "Service Unavailable", None, None)

    monkeypatch.setattr(urllib.request, "urlopen", fake_urlopen)
    kind, detail = linkcheck.check("https://x.invalid/down", 5)
    assert (kind, detail) == ("unreachable", "503")


def test_check_detail_never_carries_server_reason_text(monkeypatch):
    """The final review flagged remote text flowing into issue bodies -
    detail must be a status code or exception class name only."""

    def fake_urlopen(req, timeout):
        raise urllib.error.HTTPError(
            req.full_url, 404, "<script>evil server text</script>", None, None)

    monkeypatch.setattr(urllib.request, "urlopen", fake_urlopen)
    _, detail = linkcheck.check("https://x.invalid/gone", 5)
    assert detail == "404"
    assert "script" not in detail


def test_main_reports_dead_and_unreachable_and_exits_1_on_dead(tmp_path, monkeypatch, capsys):
    root = make_repo(tmp_path)
    (root / "README.md").write_text(
        "See https://x.invalid/dead and https://x.invalid/blocked\n",
        encoding="utf-8", newline="\n")
    (root / "MAINTENANCE.md").write_text("None here\n", encoding="utf-8", newline="\n")
    write_outputs(root, root)

    def fake_check(url, timeout):
        if url.endswith("/dead"):
            return "dead", "404"
        return "unreachable", "timeout"

    monkeypatch.setattr(linkcheck, "check", fake_check)
    rc = linkcheck.main(["--root", str(root)])
    out = capsys.readouterr().out
    assert rc == 1
    assert "DEAD 404 https://x.invalid/dead" in out
    assert "UNREACHABLE timeout https://x.invalid/blocked" in out
    # make_repo's own fixture source contributes one more URL (x.invalid/a),
    # which also lands in the unreachable bucket via fake_check's else branch.
    assert "checked 3: ok 0, dead 1, unreachable 2" in out


def test_main_exits_0_when_only_unreachable(tmp_path, monkeypatch, capsys):
    root = make_repo(tmp_path)
    (root / "README.md").write_text(
        "See https://x.invalid/blocked\n", encoding="utf-8", newline="\n")
    (root / "MAINTENANCE.md").write_text("None here\n", encoding="utf-8", newline="\n")
    write_outputs(root, root)

    monkeypatch.setattr(linkcheck, "check", lambda url, timeout: ("unreachable", "403"))
    rc = linkcheck.main(["--root", str(root)])
    out = capsys.readouterr().out
    assert rc == 0
    assert "UNREACHABLE 403 https://x.invalid/blocked" in out
    # make_repo's own fixture source contributes one more URL (x.invalid/a).
    assert "checked 2: ok 0, dead 0, unreachable 2" in out


def test_main_probes_urls_concurrently(tmp_path, monkeypatch, capsys):
    """A serial probe loop can exceed the workflow's 15-minute job limit."""
    rendezvous = threading.Barrier(2)
    monkeypatch.setattr(
        linkcheck,
        "collect_urls",
        lambda root: ["https://x.invalid/one", "https://x.invalid/two"],
    )

    def fake_check(url, timeout):
        try:
            rendezvous.wait(timeout=0.25)
        except threading.BrokenBarrierError:
            return "unreachable", "serial"
        return "ok", "200"

    monkeypatch.setattr(linkcheck, "check", fake_check)

    rc = linkcheck.main(["--root", str(tmp_path)])
    out = capsys.readouterr().out

    assert rc == 0
    assert out == "checked 2: ok 2, dead 0, unreachable 0\n"
