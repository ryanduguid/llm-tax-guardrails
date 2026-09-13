"""Live link checking. Report-only: prints findings, never edits anything."""
from __future__ import annotations

import argparse
import re
import socket
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from functools import partial
from pathlib import Path

from .build import GENERATED, find_root, load_sources

URL_RE = re.compile(r"https://[^\s)\"<>]+")
UA = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) drdebits-linkcheck"

# HTTP statuses that indicate definitive link rot rather than a network/CDN
# hiccup. Everything else (403/429/5xx, timeouts, TLS errors, connection
# resets, DNS hiccups that aren't outright NXDOMAIN, ...) is merely
# unreachable *from this network* and must not be reported as dead.
DEAD_HTTP_STATUSES = {404, 410}

# getaddrinfo errnos that indicate definitive name rot (NXDOMAIN / no address
# records). Every other gaierror - EAI_AGAIN's "temporary failure in name
# resolution" being the common one on CI runners - is a transient resolver
# condition, not link rot. EAI_NODATA is missing from some platforms' socket
# modules, hence the hasattr guard.
DEAD_GAI_ERRNOS = frozenset(
    getattr(socket, name) for name in ("EAI_NONAME", "EAI_NODATA")
    if hasattr(socket, name)
)


def collect_urls(root):
    root = Path(root)
    s = load_sources(root)
    texts = [fn(s) for fn in GENERATED.values()]
    for rel in ("README.md", "MAINTENANCE.md"):
        p = root / rel
        if p.is_file():
            texts.append(p.read_text(encoding="utf-8"))
    seen, out = set(), []
    for text in texts:
        for m in URL_RE.finditer(text):
            url = m.group(0).rstrip(".,;")
            if url not in seen:
                seen.add(url)
                out.append(url)
    return out


def _classify_exception(exc):
    """Map an exception to (kind, detail).

    detail is always an HTTP status code or an exception class name - never
    a server-supplied reason phrase (HTTPError.msg, URLError.reason strings,
    etc. can echo attacker- or server-controlled text and must not flow into
    issue bodies).
    """
    if isinstance(exc, urllib.error.HTTPError):
        detail = str(exc.code)
        if exc.code in DEAD_HTTP_STATUSES:
            return "dead", detail
        return "unreachable", detail
    if isinstance(exc, urllib.error.URLError):
        reason = exc.reason
        if isinstance(reason, socket.gaierror):
            if reason.errno in DEAD_GAI_ERRNOS:
                return "dead", "gaierror"
            return "unreachable", "gaierror"
        if isinstance(reason, TimeoutError):
            return "unreachable", "timeout"
        if isinstance(reason, BaseException):
            return "unreachable", type(reason).__name__
        return "unreachable", type(exc).__name__
    if isinstance(exc, TimeoutError):
        return "unreachable", "timeout"
    return "unreachable", type(exc).__name__


def _attempt(url, timeout):
    # collect_urls only yields https:// URLs, but keep urlopen pinned to that
    # scheme here too so a future collector change cannot make this fetch
    # file:// or other local schemes.
    if not url.startswith("https://"):
        return "unreachable", "non-https"
    req = urllib.request.Request(url, headers={"User-Agent": UA}, method="GET")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            status = resp.status
            final = resp.geturl()
        if not str(final).startswith("https://"):
            return "unreachable", "non-https-redirect"
    except Exception as exc:  # any failure is a finding, described not raised
        return _classify_exception(exc)
    if 200 <= status < 400:
        return "ok", str(status)
    return "unreachable", str(status)


def check(url, timeout):
    """Classify a URL as ("ok" | "dead" | "unreachable", detail).

    Unreachable results (timeouts, connection refused/reset, HTTP
    403/429/5xx, TLS errors, and anything else that isn't a definitive
    404/410 or DNS NXDOMAIN) get one retry with the same timeout before the
    classification sticks, since a single blocked probe from this network
    must not be reported as link rot.
    """
    kind, detail = _attempt(url, timeout)
    if kind == "unreachable":
        kind, detail = _attempt(url, timeout)
    return kind, detail


def main(argv=None):
    parser = argparse.ArgumentParser(prog="drdebits_build.linkcheck")
    parser.add_argument("--root", default=None)
    parser.add_argument("--timeout", type=int, default=20)
    args = parser.parse_args(argv)
    root = Path(args.root) if args.root else find_root(Path.cwd())
    urls = collect_urls(root)
    ok = dead = unreachable = 0
    with ThreadPoolExecutor(max_workers=10) as executor:
        results = executor.map(partial(check, timeout=args.timeout), urls)
        for url, (kind, detail) in zip(urls, results):
            if kind == "ok":
                ok += 1
            elif kind == "dead":
                dead += 1
                print(f"DEAD {detail} {url}")
            else:
                unreachable += 1
                print(f"UNREACHABLE {detail} {url}")
    print(f"checked {ok + dead + unreachable}: ok {ok}, dead {dead}, unreachable {unreachable}")
    return 1 if dead else 0


if __name__ == "__main__":
    raise SystemExit(main())
