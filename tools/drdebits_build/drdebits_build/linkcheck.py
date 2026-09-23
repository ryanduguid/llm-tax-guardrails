"""Live link checking. Report-only: prints findings, never edits anything."""
from __future__ import annotations

import argparse
import http.client
import ipaddress
import re
import socket
import urllib.error
import urllib.parse
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


class _PinnedHTTPSConnection(http.client.HTTPSConnection):
    """Connect to the approved address while retaining the URL hostname."""

    def __init__(self, host, pinned_ip, **kwargs):
        super().__init__(host, **kwargs)
        self._pinned_ip = pinned_ip

    def connect(self):
        self.sock = socket.create_connection((self._pinned_ip, self.port), self.timeout)
        self.sock = self._context.wrap_socket(self.sock, server_hostname=self.host)


class _PinnedHTTPSHandler(urllib.request.HTTPSHandler):
    def https_open(self, req):
        return self.do_open(_PinnedHTTPSConnection, req, context=self._context)

    def do_open(self, http_class, req, **http_conn_args):
        pinned_ip = getattr(req, "_pinned_ip", None)
        if pinned_ip is None:
            raise _BlockedDestination("unvalidated-destination")
        return super().do_open(
            lambda host, **kwargs: http_class(host, pinned_ip, **kwargs),
            req,
            **http_conn_args,
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
    and so on can echo attacker- or server-controlled text and must not flow into
    issue bodies).
    """
    if isinstance(exc, _BlockedDestination):
        return "unreachable", exc.detail
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


class _BlockedDestination(urllib.error.URLError):
    """A redirect target this checker refused to request."""

    def __init__(self, detail: str) -> None:
        super().__init__(detail)
        self.detail = detail


def _destination_problem(url: str) -> str | None:
    """Why `url` must not be requested, or None when it may be.

    Only public https destinations are allowed. Anything resolving to a
    loopback, private, link-local, shared or otherwise reserved address is
    refused, so a linked site cannot point this checker at a service on the
    runner or inside its network. A name that does not resolve is left to the
    request itself, because _classify_exception distinguishes definitive name
    rot from a transient resolver failure and this must not pre-empt that.
    """
    parts = urllib.parse.urlsplit(url)
    if parts.scheme != "https":
        return "non-https"
    try:
        host = parts.hostname
        port = parts.port or 443
    except ValueError:
        return "unparsable-url"
    if not host:
        return "no-host"
    try:
        infos = socket.getaddrinfo(host, port, type=socket.SOCK_STREAM)
    except (socket.gaierror, UnicodeError):
        return "unresolvable"
    if any(not ipaddress.ip_address(info[4][0]).is_global for info in infos):
        return "non-public-address"
    return None


def _validated_ip(url: str) -> str | None:
    parts = urllib.parse.urlsplit(url)
    host = parts.hostname
    port = parts.port or 443
    infos = socket.getaddrinfo(host, port, type=socket.SOCK_STREAM)
    for info in infos:
        address = info[4][0]
        if ipaddress.ip_address(address).is_global:
            return address
    return None


class _ValidatingRedirectHandler(urllib.request.HTTPRedirectHandler):
    """Check a redirect target before the request that would follow it.

    urlopen follows redirects itself, so checking resp.geturl() afterwards
    checks a request that has already gone out. This runs first instead.
    """

    def redirect_request(self, req, fp, code, msg, headers, newurl):  # noqa: ANN001, ANN201
        problem = _destination_problem(newurl)
        if problem is not None:
            raise _BlockedDestination(problem)
        redirected = super().redirect_request(req, fp, code, msg, headers, newurl)
        redirected._pinned_ip = _validated_ip(newurl)
        return redirected


_OPENER = urllib.request.build_opener(_ValidatingRedirectHandler, _PinnedHTTPSHandler)


def _open(req, timeout):
    """The only place this module issues a request. The offline tests stub it."""
    return _OPENER.open(req, timeout=timeout)


def _attempt(url, timeout):
    # collect_urls only yields https:// URLs, but keep the fetch pinned to a
    # public https destination here too, so a future collector change cannot
    # make this reach file://, a local service or a private address.
    problem = _destination_problem(url)
    if problem is not None:
        return "unreachable", problem
    req = urllib.request.Request(url, headers={"User-Agent": UA}, method="GET")
    req._pinned_ip = _validated_ip(url)
    try:
        with _open(req, timeout) as resp:
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
