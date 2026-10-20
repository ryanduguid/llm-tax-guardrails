//! Live link checking. Report-only: prints findings, never edits anything.

use std::net::{IpAddr, SocketAddr};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use regex::Regex;

use crate::builder::{self, render_generated};
use crate::error::Fail;
use crate::net::{is_global, port_number, urlsplit};
use crate::py::{self, PyPath};

static URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r#"https://[^{})"<>]+"#,
        py::class_body(crate::pyunicode::SPACE)
    ))
    .expect("pattern")
});
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) drdebits-linkcheck";

// HTTP statuses that indicate definitive link rot rather than a network or
// CDN hiccup. Everything else (403/429/5xx, timeouts, TLS errors, resets,
// transient DNS failures) is merely unreachable from this network and must
// not be reported as dead.
const DEAD_HTTP_STATUSES: [u16; 2] = [404, 410];

// Hosts that time out on GitHub's runners because they block cloud address
// ranges, and two ATO pages that answer the runners 403 while the rest of
// their host answers. From a residential connection on 29 September 2026 every
// one resolved (issue 113). The advice-and-guidance page answered the
// 18 October 2026 run "UNREACHABLE 403" (run 39174483634, issue 134) and
// answered 200 with its own title from a residential connection on
// 19 October 2026. An unreachable link here is listed for checking by hand
// instead of making the check incomplete; a dead answer still fails.
const MANUAL_HOSTS: [&str; 2] = ["www.tpb.gov.au", "www.austrac.gov.au"];
const MANUAL_URLS: [&str; 2] = [
    "https://www.ato.gov.au/law/view/document?docid=PSR%2FPS20125%2FNAT%2FATO%2F00001",
    "https://www.ato.gov.au/about-ato/ato-advice-and-guidance/how-our-advice-and-guidance-protects-you",
];

const MAX_REPEATS: usize = 4;
const MAX_REDIRECTIONS: usize = 10;

#[derive(clap::Args)]
pub struct Args {
    /// Seconds to wait on each connection and read.
    #[arg(long, default_value_t = 20)]
    timeout: u64,
}

pub fn needs_manual_check(url: &str) -> bool {
    MANUAL_URLS.contains(&url)
        || urlsplit(url)
            .and_then(|s| s.hostname)
            .is_some_and(|h| MANUAL_HOSTS.contains(&h.as_str()))
}

pub fn collect_urls(root: &PyPath) -> Result<Vec<String>, Fail> {
    let s = builder::load_sources(root)?;
    let mut texts: Vec<String> = render_generated(&s).into_iter().map(|(_, t)| t).collect();
    for rel in ["README.md", "MAINTENANCE.md"] {
        let p = root.join(rel);
        if p.fs().is_file() {
            texts.push(builder::read_text(&p)?);
        }
    }
    let mut out: Vec<String> = Vec::new();
    for text in &texts {
        for m in URL_RE.find_iter(text) {
            let url = m.as_str().trim_end_matches(['.', ',', ';']).to_string();
            if !out.contains(&url) {
                out.push(url);
            }
        }
    }
    Ok(out)
}

/// Why a name did not resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsError {
    /// NXDOMAIN or no address records: definitive name rot.
    NoName,
    /// A transient resolver condition.
    Other,
}

/// Why one request failed. The detail is always a status, a fixed label or
/// an exception class name, never server-supplied text, because these lines
/// flow into issue bodies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GetError {
    Blocked(&'static str),
    Dns(DnsError),
    Timeout,
    Other(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub location: Option<String>,
}

/// The network as the checker sees it; the tests supply a fake.
pub trait Network: Sync {
    fn resolve(&self, host: &str, port: u16) -> Result<Vec<IpAddr>, DnsError>;
    /// One GET of `url`, following no redirect.
    fn get(&self, url: &str, timeout: Duration) -> Result<Reply, GetError>;
}

/// Why `url` must not be requested, or None when it may be.
///
/// Only public https destinations are allowed. Anything resolving to a
/// loopback, private, link-local, shared or otherwise reserved address is
/// refused, so a linked site cannot point this checker at a service on the
/// runner or inside its network. A name that does not resolve is left to the
/// request itself, which distinguishes definitive name rot from a transient
/// resolver failure. The request's own resolver repeats the address check on
/// the answer the connection actually uses.
pub fn destination_problem(network: &dyn Network, url: &str) -> Option<&'static str> {
    let Some(parts) = urlsplit(url) else {
        return Some("unparsable-url");
    };
    if parts.scheme != "https" {
        return Some("non-https");
    }
    let Ok(port) = port_number(&parts.port) else {
        return Some("unparsable-url");
    };
    let Some(host) = parts.hostname else {
        return Some("no-host");
    };
    let port = port.filter(|p| *p != 0).unwrap_or(443);
    match network.resolve(&host, port) {
        Err(_) => None,
        Ok(addresses) if addresses.iter().any(|ip| !is_global(*ip)) => Some("non-public-address"),
        Ok(_) => None,
    }
}

fn classify(error: &GetError) -> (&'static str, String) {
    match error {
        GetError::Blocked(detail) => ("unreachable", detail.to_string()),
        GetError::Dns(DnsError::NoName) => ("dead", "gaierror".into()),
        GetError::Dns(DnsError::Other) => ("unreachable", "gaierror".into()),
        GetError::Timeout => ("unreachable", "timeout".into()),
        GetError::Other(name) => ("unreachable", name.to_string()),
    }
}

fn http_error(status: u16) -> (&'static str, String) {
    if DEAD_HTTP_STATUSES.contains(&status) {
        ("dead", status.to_string())
    } else {
        ("unreachable", status.to_string())
    }
}

fn attempt(network: &dyn Network, url: &str, timeout: Duration) -> (&'static str, String) {
    // collect_urls only yields https URLs, but the fetch stays pinned to a
    // public https destination here too.
    if let Some(problem) = destination_problem(network, url) {
        return ("unreachable", problem.into());
    }
    let mut current = url.to_string();
    let mut visited: Vec<(String, usize)> = Vec::new();
    loop {
        let reply = match network.get(&current, timeout) {
            Ok(reply) => reply,
            Err(e) => return classify(&e),
        };
        if (200..300).contains(&reply.status) {
            if !current.starts_with("https://") {
                return ("unreachable", "non-https-redirect".into());
            }
            return ("ok", reply.status.to_string());
        }
        let Some(location) = reply
            .location
            .filter(|_| [301, 302, 303, 307, 308].contains(&reply.status))
        else {
            return http_error(reply.status);
        };
        // For security reasons urllib follows only http, https and ftp.
        let scheme = urlsplit(&location).map(|s| s.scheme).unwrap_or_default();
        if !["http", "https", "ftp", ""].contains(&scheme.as_str()) {
            return http_error(reply.status);
        }
        let Some(next) = url::Url::parse(&current)
            .and_then(|base| base.join(&location.replace(' ', "%20")))
            .ok()
        else {
            return ("unreachable", "unparsable-url".into());
        };
        let next = next.to_string();
        // Checked before the request that would follow it.
        if let Some(problem) = destination_problem(network, &next) {
            return ("unreachable", problem.into());
        }
        let seen = visited
            .iter()
            .find(|(u, _)| *u == next)
            .map_or(0, |(_, n)| *n);
        if seen >= MAX_REPEATS || visited.len() >= MAX_REDIRECTIONS {
            return http_error(reply.status);
        }
        match visited.iter_mut().find(|(u, _)| *u == next) {
            Some(entry) => entry.1 += 1,
            None => visited.push((next.clone(), 1)),
        }
        current = next;
    }
}

/// Classify a URL as ("ok" | "dead" | "unreachable", detail).
///
/// Unreachable results get one retry with the same timeout before the
/// classification sticks, since a single blocked probe from this network
/// must not be reported as link rot.
pub fn check(network: &dyn Network, url: &str, timeout: Duration) -> (&'static str, String) {
    let first = attempt(network, url, timeout);
    if first.0 == "unreachable" {
        attempt(network, url, timeout)
    } else {
        first
    }
}

/// Run the checks ten at a time and report in URL order. Returns the exit code.
pub fn report(
    network: &dyn Network,
    urls: &[String],
    timeout: Duration,
    out: &mut dyn std::io::Write,
) -> u8 {
    let results: Vec<std::sync::Mutex<Option<(&'static str, String)>>> =
        urls.iter().map(|_| Default::default()).collect();
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..10 {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    if i >= urls.len() {
                        break;
                    }
                    let result = check(network, &urls[i], timeout);
                    *results[i].lock().expect("unpoisoned") = Some(result);
                }
            });
        }
    });
    let (mut ok, mut dead, mut unreachable, mut manual) = (0, 0, 0, 0);
    for (url, result) in urls.iter().zip(results) {
        let (kind, detail) = result
            .into_inner()
            .expect("unpoisoned")
            .expect("every URL checked");
        if kind == "ok" {
            ok += 1;
        } else if kind == "dead" {
            dead += 1;
            let _ = writeln!(out, "DEAD {detail} {url}");
        } else if needs_manual_check(url) {
            manual += 1;
            let _ = writeln!(out, "MANUAL {detail} {url}");
        } else {
            unreachable += 1;
            let _ = writeln!(out, "UNREACHABLE {detail} {url}");
        }
    }
    let _ = writeln!(
        out,
        "checked {}: ok {ok}, dead {dead}, unreachable {unreachable}, manual {manual}",
        ok + dead + unreachable + manual
    );
    if dead > 0 {
        return 1;
    }
    if unreachable > 0 || urls.is_empty() {
        return 2;
    }
    // 3: every link that could be reached is live; the MANUAL ones need a person.
    if manual > 0 { 3 } else { 0 }
}

/// A name lookup: addresses for `host` and `port`, or why there are none.
pub type Lookup = fn(&str, u16) -> Result<Vec<SocketAddr>, DnsError>;

/// The real network: one resolution checked for public addresses and then
/// used for the connection, with no proxy, so the address checked is the
/// address connected to.
pub struct Live {
    pub lookup: Lookup,
}

impl Default for Live {
    fn default() -> Self {
        Live {
            lookup: system_lookup,
        }
    }
}

/// `getaddrinfo` for a stream socket, as the Python checker called it.
pub fn system_lookup(host: &str, port: u16) -> Result<Vec<SocketAddr>, DnsError> {
    let hints = dns_lookup::AddrInfoHints {
        socktype: dns_lookup::SockType::Stream.into(),
        ..Default::default()
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    match dns_lookup::getaddrinfo(Some(host), Some(&port.to_string()), Some(hints)) {
        Ok(infos) => {
            let addrs: Vec<SocketAddr> = infos.filter_map(Result::ok).map(|i| i.sockaddr).collect();
            if addrs.is_empty() {
                Err(DnsError::NoName)
            } else {
                Ok(addrs)
            }
        }
        Err(e) => match e.kind() {
            dns_lookup::LookupErrorKind::NoName | dns_lookup::LookupErrorKind::NoData => {
                Err(DnsError::NoName)
            }
            _ => Err(DnsError::Other),
        },
    }
}

/// Resolve once and refuse any non-public answer; the connection then uses
/// exactly these addresses, so a name cannot answer public for the check and
/// private for the connection.
pub fn resolve_public(lookup: Lookup, host: &str, port: u16) -> Result<Vec<SocketAddr>, GetError> {
    let addrs = lookup(host, port).map_err(GetError::Dns)?;
    if addrs.iter().any(|a| !is_global(a.ip())) {
        return Err(GetError::Blocked("non-public-address"));
    }
    Ok(addrs)
}

#[derive(Debug)]
struct Refusal(GetError);

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl std::error::Error for Refusal {}

#[derive(Debug)]
struct PublicResolver {
    lookup: Lookup,
}

impl ureq::unversioned::resolver::Resolver for PublicResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        _config: &ureq::config::Config,
        _timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        let host = uri.host().unwrap_or_default();
        let port = uri.port_u16().unwrap_or(443);
        let addrs = resolve_public(self.lookup, host, port)
            .map_err(|e| ureq::Error::Io(std::io::Error::other(Refusal(e))))?;
        let mut out = self.empty();
        for addr in addrs.into_iter().take(16) {
            out.push(addr);
        }
        Ok(out)
    }
}

fn io_error_name(e: &std::io::Error) -> GetError {
    use std::io::ErrorKind;
    if let Some(refusal) = e
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<Refusal>())
    {
        return refusal.0.clone();
    }
    match e.kind() {
        ErrorKind::TimedOut | ErrorKind::WouldBlock => GetError::Timeout,
        ErrorKind::ConnectionRefused => GetError::Other("ConnectionRefusedError"),
        ErrorKind::ConnectionReset => GetError::Other("ConnectionResetError"),
        ErrorKind::ConnectionAborted => GetError::Other("ConnectionAbortedError"),
        ErrorKind::UnexpectedEof => GetError::Other("RemoteDisconnected"),
        ErrorKind::InvalidData if e.to_string().to_lowercase().contains("certificate") => {
            GetError::Other("SSLCertVerificationError")
        }
        ErrorKind::InvalidData => GetError::Other("SSLError"),
        _ => GetError::Other("OSError"),
    }
}

/// The request settings: no proxy (through one, the guard would check the
/// proxy's address, not the destination's), no automatic redirects (each hop
/// is checked first), and error statuses returned rather than raised.
pub fn agent_config(timeout: Duration) -> ureq::config::Config {
    ureq::Agent::config_builder()
        .proxy(None)
        .max_redirects(0)
        .max_redirects_will_error(false)
        .http_status_as_error(false)
        .user_agent(UA)
        .timeout_connect(Some(timeout))
        .timeout_send_request(Some(timeout))
        .timeout_recv_response(Some(timeout))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
}

impl Network for Live {
    fn resolve(&self, host: &str, port: u16) -> Result<Vec<IpAddr>, DnsError> {
        (self.lookup)(host, port).map(|addrs| addrs.into_iter().map(|a| a.ip()).collect())
    }

    fn get(&self, url: &str, timeout: Duration) -> Result<Reply, GetError> {
        let resolver = PublicResolver {
            lookup: self.lookup,
        };
        let agent = ureq::Agent::with_parts(
            agent_config(timeout),
            ureq::unversioned::transport::DefaultConnector::new(),
            resolver,
        );
        match agent.get(url).call() {
            Ok(response) => Ok(Reply {
                status: response.status().as_u16(),
                location: response
                    .headers()
                    .get("location")
                    .or_else(|| response.headers().get("uri"))
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string),
            }),
            Err(ureq::Error::Io(e)) => Err(io_error_name(&e)),
            Err(ureq::Error::Timeout(_)) => Err(GetError::Timeout),
            Err(ureq::Error::HostNotFound) => Err(GetError::Dns(DnsError::Other)),
            Err(ureq::Error::Tls(_)) => Err(GetError::Other("SSLError")),
            Err(_) => Err(GetError::Other("OSError")),
        }
    }
}

pub fn main(root: &Option<String>, args: Args) -> u8 {
    let urls = match crate::root_for(root).and_then(|root| collect_urls(&root)) {
        Ok(urls) => urls,
        Err(e) => {
            errln!("linkcheck: {e}");
            return 1;
        }
    };
    report(
        &Live::default(),
        &urls,
        Duration::from_secs(args.timeout),
        &mut crate::PyText::stdout(),
    )
}
