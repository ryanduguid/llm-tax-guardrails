//! URL collection, outcome classification and the destination guard, tested
//! offline against a scripted network (ported from test_linkcheck.py).

mod common;

use std::net::{IpAddr, SocketAddr};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use common::fixture;
use drdebits::builder;
use drdebits::linkcheck::{
    DnsError, GetError, Live, Network, Reply, agent_config, check, collect_urls,
    needs_manual_check, report, resolve_public, run, system_lookup,
};

const PUBLIC: &str = "93.184.216.34";
const T: Duration = Duration::from_secs(5);

type Step = Result<Reply, GetError>;

/// Every host resolves to `address`; each GET takes the next scripted answer.
struct Fake {
    address: Mutex<Vec<Result<Vec<IpAddr>, DnsError>>>,
    default_address: Result<Vec<IpAddr>, DnsError>,
    answers: Mutex<Vec<Step>>,
    calls: Mutex<Vec<String>>,
}

impl Fake {
    fn new(answers: Vec<Step>) -> Fake {
        Fake::resolving(Ok(vec![PUBLIC.parse().unwrap()]), answers)
    }

    fn resolving(address: Result<Vec<IpAddr>, DnsError>, mut answers: Vec<Step>) -> Fake {
        answers.reverse();
        Fake {
            address: Mutex::new(Vec::new()),
            default_address: address,
            answers: Mutex::new(answers),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

impl Network for Fake {
    fn resolve(&self, _host: &str, _port: u16) -> Result<Vec<IpAddr>, DnsError> {
        self.address
            .lock()
            .unwrap()
            .pop()
            .unwrap_or_else(|| self.default_address.clone())
    }

    fn get(&self, url: &str, _timeout: Duration) -> Result<Reply, GetError> {
        self.calls.lock().unwrap().push(url.to_string());
        self.answers
            .lock()
            .unwrap()
            .pop()
            .expect("a scripted answer")
    }
}

fn status(code: u16) -> Step {
    Ok(Reply {
        status: code,
        location: None,
    })
}

fn redirect(code: u16, location: &str) -> Step {
    Ok(Reply {
        status: code,
        location: Some(location.into()),
    })
}

#[test]
fn collect_urls_unique_ordered() {
    let (dir, root) = fixture();
    std::fs::write(
        dir.path().join("README.md"),
        "See https://x.invalid/a and https://x.invalid/a again\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("MAINTENANCE.md"), "None here\n").unwrap();
    builder::write_outputs(&root, &root, &builder::load_sources(&root).unwrap()).unwrap();
    let urls = collect_urls(&root).unwrap();
    assert_eq!(
        urls.iter().filter(|u| *u == "https://x.invalid/a").count(),
        1
    );
}

#[test]
fn collect_urls_reads_only_the_hand_written_files_present() {
    let (dir, root) = fixture();
    std::fs::write(
        dir.path().join("README.md"),
        "See https://x.invalid/readme.\n",
    )
    .unwrap();
    std::fs::remove_file(dir.path().join("MAINTENANCE.md")).unwrap();
    assert!(
        collect_urls(&root)
            .unwrap()
            .contains(&"https://x.invalid/readme".to_string())
    );
}

#[test]
fn a_404_is_dead_and_not_retried() {
    let net = Fake::new(vec![status(404)]);
    assert_eq!(
        check(&net, "https://x.invalid/gone", T),
        ("dead", "404".into())
    );
    assert_eq!(net.calls(), 1);
}

#[test]
fn a_403_is_unreachable_after_one_retry() {
    let net = Fake::new(vec![status(403), status(403)]);
    assert_eq!(
        check(&net, "https://x.invalid/blocked", T),
        ("unreachable", "403".into())
    );
    assert_eq!(net.calls(), 2);
}

#[test]
fn a_503_is_unreachable_and_the_retry_fails_too() {
    let net = Fake::new(vec![status(503), status(503)]);
    assert_eq!(
        check(&net, "https://x.invalid/busy", T),
        ("unreachable", "503".into())
    );
    assert_eq!(net.calls(), 2);
}

#[test]
fn a_timeout_then_success_is_ok() {
    let net = Fake::new(vec![Err(GetError::Timeout), status(200)]);
    assert_eq!(
        check(&net, "https://x.invalid/flaky", T),
        ("ok", "200".into())
    );
    assert_eq!(net.calls(), 2);
}

#[test]
fn a_definitive_dns_failure_is_dead() {
    let net = Fake::new(vec![Err(GetError::Dns(DnsError::NoName))]);
    assert_eq!(
        check(&net, "https://x.invalid/nxdomain", T),
        ("dead", "gaierror".into())
    );
    assert_eq!(net.calls(), 1);
}

#[test]
fn a_transient_dns_failure_is_unreachable_and_retried() {
    let net = Fake::new(vec![
        Err(GetError::Dns(DnsError::Other)),
        Err(GetError::Dns(DnsError::Other)),
    ]);
    assert_eq!(
        check(&net, "https://x.invalid/flaky-dns", T),
        ("unreachable", "gaierror".into())
    );
    assert_eq!(net.calls(), 2);
}

#[test]
fn a_non_https_url_is_never_fetched() {
    let net = Fake::new(vec![]);
    assert_eq!(
        check(&net, "file:///C:/Windows/win.ini", T),
        ("unreachable", "non-https".into())
    );
    assert_eq!(net.calls(), 0);
}

#[test]
fn errors_are_classified_by_name_never_by_server_text() {
    let net = Fake::new(vec![
        Err(GetError::Other("ConnectionResetError")),
        Err(GetError::Other("ConnectionResetError")),
    ]);
    assert_eq!(
        check(&net, "https://x.invalid/reset", T),
        ("unreachable", "ConnectionResetError".into())
    );
    let net = Fake::new(vec![Err(GetError::Timeout), Err(GetError::Timeout)]);
    assert_eq!(
        check(&net, "https://x.invalid/slow", T),
        ("unreachable", "timeout".into())
    );
}

#[test]
fn report_lists_dead_and_unreachable_and_exits_1_on_dead() {
    let urls: Vec<String> = [
        "https://x.invalid/dead",
        "https://x.invalid/blocked",
        "https://x.invalid/a",
    ]
    .map(String::from)
    .to_vec();
    struct ByUrl;
    impl Network for ByUrl {
        fn resolve(&self, _: &str, _: u16) -> Result<Vec<IpAddr>, DnsError> {
            Ok(vec![PUBLIC.parse().unwrap()])
        }
        fn get(&self, url: &str, _: Duration) -> Result<Reply, GetError> {
            if url.ends_with("/dead") {
                Ok(Reply {
                    status: 404,
                    location: None,
                })
            } else {
                Err(GetError::Timeout)
            }
        }
    }
    let mut out = Vec::new();
    assert_eq!(report(&ByUrl, &urls, T, &mut out), 1);
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("DEAD 404 https://x.invalid/dead"));
    assert!(out.contains("UNREACHABLE timeout https://x.invalid/blocked"));
    assert!(out.contains("checked 3: ok 0, dead 1, unreachable 2, manual 0"));
}

const ATO_DENIED: &str =
    "https://www.ato.gov.au/law/view/document?docid=PSR%2FPS20125%2FNAT%2FATO%2F00001";

struct Table(Vec<(&'static str, Step)>);

impl Network for Table {
    fn resolve(&self, _: &str, _: u16) -> Result<Vec<IpAddr>, DnsError> {
        Ok(vec![PUBLIC.parse().unwrap()])
    }
    fn get(&self, url: &str, _: Duration) -> Result<Reply, GetError> {
        let (_, step) = self.0.iter().find(|(u, _)| *u == url).unwrap();
        match step {
            Ok(r) => Ok(Reply {
                status: r.status,
                location: r.location.clone(),
            }),
            Err(e) => Err(e.clone()),
        }
    }
}

fn run_report(table: Vec<(&'static str, Step)>) -> (u8, String) {
    let urls: Vec<String> = table.iter().map(|(u, _)| u.to_string()).collect();
    let mut out = Vec::new();
    let code = report(&Table(table), &urls, T, &mut out);
    (code, String::from_utf8(out).unwrap())
}

#[test]
fn report_exits_2_when_only_unreachable() {
    let (code, out) = run_report(vec![
        ("https://x.invalid/blocked", status(403)),
        ("https://x.invalid/a", status(403)),
    ]);
    assert_eq!(code, 2);
    assert!(out.contains("UNREACHABLE 403 https://x.invalid/blocked"));
    assert!(out.contains("checked 2: ok 0, dead 0, unreachable 2, manual 0"));
}

#[test]
fn report_exits_3_when_only_runner_blocked_links_are_unreachable() {
    let (code, out) = run_report(vec![
        ("https://www.tpb.gov.au/guidance", Err(GetError::Timeout)),
        (
            "https://www.austrac.gov.au/obligations",
            Err(GetError::Timeout),
        ),
        (ATO_DENIED, status(403)),
        ("https://www.ato.gov.au/elsewhere", status(200)),
    ]);
    assert_eq!(code, 3);
    assert!(out.contains("MANUAL timeout https://www.tpb.gov.au/guidance"));
    assert!(out.contains(&format!("MANUAL 403 {ATO_DENIED}")));
    assert!(out.contains("checked 4: ok 1, dead 0, unreachable 0, manual 3"));
}

#[test]
fn a_dead_link_on_a_runner_blocked_host_still_fails() {
    let (code, out) = run_report(vec![
        ("https://www.tpb.gov.au/gone", status(404)),
        (
            "https://www.austrac.gov.au/obligations",
            Err(GetError::Timeout),
        ),
    ]);
    assert_eq!(code, 1);
    assert!(out.contains("DEAD 404 https://www.tpb.gov.au/gone"));
}

#[test]
fn other_unreachable_links_keep_the_check_incomplete() {
    let (code, out) = run_report(vec![
        ("https://www.tpb.gov.au/guidance", Err(GetError::Timeout)),
        ("https://www.ato.gov.au/elsewhere", status(403)),
    ]);
    assert_eq!(code, 2);
    assert!(out.contains("UNREACHABLE 403 https://www.ato.gov.au/elsewhere"));
}

#[test]
fn only_the_listed_hosts_and_page_are_manual() {
    for (url, manual) in [
        ("https://www.tpb.gov.au/guidance", true),
        ("https://www.austrac.gov.au/obligations", true),
        (ATO_DENIED, true),
        (
            "https://www.ato.gov.au/about-ato/ato-advice-and-guidance/how-our-advice-and-guidance-protects-you",
            true,
        ),
        (
            "https://www.ato.gov.au/about-ato/ato-advice-and-guidance",
            false,
        ),
        (
            "https://www.ato.gov.au/law/view/document?docid=OTHER",
            false,
        ),
        ("https://tpb.gov.au.example.com/guidance", false),
        ("https://example.com/www.tpb.gov.au", false),
    ] {
        assert_eq!(needs_manual_check(url), manual, "{url}");
    }
}

#[test]
fn report_probes_urls_concurrently() {
    // A serial probe loop can exceed the workflow's time limit.
    struct Rendezvous {
        arrived: Mutex<usize>,
        both: Condvar,
    }
    impl Network for Rendezvous {
        fn resolve(&self, _: &str, _: u16) -> Result<Vec<IpAddr>, DnsError> {
            Ok(vec![PUBLIC.parse().unwrap()])
        }
        fn get(&self, _: &str, _: Duration) -> Result<Reply, GetError> {
            let mut arrived = self.arrived.lock().unwrap();
            *arrived += 1;
            self.both.notify_all();
            let (arrived, timeout) = self
                .both
                .wait_timeout_while(arrived, Duration::from_secs(5), |n| *n < 2)
                .unwrap();
            drop(arrived);
            if timeout.timed_out() {
                Err(GetError::Other("serial"))
            } else {
                Ok(Reply {
                    status: 200,
                    location: None,
                })
            }
        }
    }
    let urls = vec![
        "https://x.invalid/one".to_string(),
        "https://x.invalid/two".to_string(),
    ];
    let mut out = Vec::new();
    let net = Rendezvous {
        arrived: Mutex::new(0),
        both: Condvar::new(),
    };
    assert_eq!(report(&net, &urls, T, &mut out), 0);
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "checked 2: ok 2, dead 0, unreachable 0, manual 0\n"
    );
}

#[test]
fn a_non_public_destination_is_never_requested() {
    for address in [
        "127.0.0.1",
        "10.0.0.5",
        "169.254.169.254",
        "192.168.1.1",
        "100.64.0.1",
    ] {
        let net = Fake::resolving(Ok(vec![address.parse().unwrap()]), vec![]);
        assert_eq!(
            check(&net, "https://metadata.example/latest", T),
            ("unreachable", "non-public-address".into())
        );
        assert_eq!(net.calls(), 0, "{address}");
    }
}

#[test]
fn a_redirect_is_checked_before_it_is_followed() {
    // To a private address: refused before the second request.
    let net = Fake::new(vec![
        redirect(302, "https://internal.example/admin"),
        redirect(302, "https://internal.example/admin"),
    ]);
    net.address.lock().unwrap().extend([
        Ok(vec!["127.0.0.1".parse().unwrap()]),
        Ok(vec![PUBLIC.parse().unwrap()]),
        Ok(vec!["127.0.0.1".parse().unwrap()]),
        Ok(vec![PUBLIC.parse().unwrap()]),
    ]);
    assert_eq!(
        check(&net, "https://x.invalid/redirector", T),
        ("unreachable", "non-public-address".into())
    );
    assert_eq!(net.calls(), 2, "each attempt made only the first request");
    // To plain http: refused by scheme.
    let net = Fake::new(vec![
        redirect(301, "http://plain.example/x"),
        redirect(301, "http://plain.example/x"),
    ]);
    assert_eq!(
        check(&net, "https://x.invalid/a", T),
        ("unreachable", "non-https".into())
    );
}

#[test]
fn a_redirect_to_a_public_https_destination_is_followed() {
    let net = Fake::new(vec![redirect(302, "https://y.invalid/b"), status(200)]);
    assert_eq!(check(&net, "https://x.invalid/a", T), ("ok", "200".into()));
    assert_eq!(
        *net.calls.lock().unwrap(),
        ["https://x.invalid/a", "https://y.invalid/b"]
    );
}

#[test]
fn a_relative_redirect_resolves_against_the_request() {
    let net = Fake::new(vec![redirect(307, "/next page"), status(200)]);
    assert_eq!(
        check(&net, "https://x.invalid/a/b", T),
        ("ok", "200".into())
    );
    assert_eq!(
        net.calls.lock().unwrap()[1],
        "https://x.invalid/next%20page"
    );
}

#[test]
fn a_redirect_loop_stops_with_the_redirect_status() {
    let answers: Vec<Step> = (0..40)
        .map(|_| redirect(302, "https://x.invalid/loop"))
        .collect();
    let net = Fake::new(answers);
    assert_eq!(
        check(&net, "https://x.invalid/loop", T),
        ("unreachable", "302".into())
    );
}

#[test]
fn a_redirect_without_a_location_or_a_safe_scheme_is_an_error_status() {
    let net = Fake::new(vec![status(302), status(302)]);
    assert_eq!(
        check(&net, "https://x.invalid/a", T),
        ("unreachable", "302".into())
    );
    let net = Fake::new(vec![
        redirect(302, "javascript:alert(1)"),
        redirect(302, "javascript:alert(1)"),
    ]);
    assert_eq!(
        check(&net, "https://x.invalid/a", T),
        ("unreachable", "302".into())
    );
}

#[test]
fn a_public_https_destination_is_still_requested() {
    let net = Fake::new(vec![status(200)]);
    assert_eq!(
        check(&net, "https://www.ato.gov.au/", T),
        ("ok", "200".into())
    );
    assert_eq!(*net.calls.lock().unwrap(), ["https://www.ato.gov.au/"]);
}

#[test]
fn malformed_https_urls_are_never_requested() {
    for (url, detail) in [
        ("https://x.invalid:notaport/", "unparsable-url"),
        ("https:///no-host", "no-host"),
    ] {
        let net = Fake::new(vec![]);
        assert_eq!(check(&net, url, T), ("unreachable", detail.into()));
        assert_eq!(net.calls(), 0);
    }
}

#[test]
fn a_name_that_does_not_resolve_is_left_to_the_request() {
    let net = Fake::resolving(Err(DnsError::NoName), vec![status(200)]);
    assert_eq!(check(&net, "https://x.invalid/a", T), ("ok", "200".into()));
}

#[test]
fn an_error_status_returned_without_an_error_is_unreachable() {
    let net = Fake::new(vec![status(503), status(503)]);
    assert_eq!(
        check(&net, "https://x.invalid/a", T),
        ("unreachable", "503".into())
    );
}

#[test]
fn the_connection_resolver_refuses_a_name_that_turns_private() {
    // The early check and the connection each resolve the name; the
    // connection's own resolution is checked too and is what it connects to.
    fn private(_: &str, port: u16) -> Result<Vec<SocketAddr>, DnsError> {
        Ok(vec![SocketAddr::new("127.0.0.1".parse().unwrap(), port)])
    }
    fn public(_: &str, port: u16) -> Result<Vec<SocketAddr>, DnsError> {
        Ok(vec![SocketAddr::new(PUBLIC.parse().unwrap(), port)])
    }
    assert_eq!(
        resolve_public(private, "rebind.example", 443),
        Err(GetError::Blocked("non-public-address"))
    );
    assert_eq!(
        resolve_public(public, "public.example", 443).unwrap(),
        [SocketAddr::new(PUBLIC.parse().unwrap(), 443)]
    );
    // Through the real request path the refusal comes before any connection.
    let live = Live { lookup: private };
    assert_eq!(
        live.get("https://rebind.example/", T).unwrap_err(),
        GetError::Blocked("non-public-address")
    );
}

#[test]
fn an_environment_proxy_is_not_used() {
    // Through a proxy the guard would check the proxy's address.
    // SAFETY: this test alone sets these variables, and only reads follow.
    unsafe {
        std::env::set_var("HTTPS_PROXY", "http://10.0.0.9:3128");
        std::env::set_var("https_proxy", "http://10.0.0.9:3128");
    }
    assert!(agent_config(T).proxy().is_none());
    assert_eq!(agent_config(T).max_redirects(), 0);
}

#[test]
fn a_refusal_while_connecting_is_unreachable_and_retried() {
    let blocked = || Err(GetError::Blocked("non-public-address"));
    let net = Fake::new(vec![blocked(), blocked()]);
    assert_eq!(
        check(&net, "https://x.invalid/a", T),
        ("unreachable", "non-public-address".into())
    );
    assert_eq!(net.calls(), 2);
}

#[test]
fn a_final_address_that_is_not_lower_case_https_is_refused() {
    // urlsplit lower-cases the scheme for the guard, but the final URL must
    // still start with "https://".
    let net = Fake::new(vec![status(200), status(200)]);
    assert_eq!(
        check(&net, "HTTPS://x.invalid/a", T),
        ("unreachable", "non-https-redirect".into())
    );
    assert_eq!(net.calls(), 2);
}

#[test]
fn a_redirect_to_an_unparsable_url_is_not_followed() {
    let net = Fake::new(vec![
        redirect(302, "https://y.invalid:notaport/"),
        redirect(302, "https://y.invalid:notaport/"),
    ]);
    assert_eq!(
        check(&net, "https://x.invalid/a", T),
        ("unreachable", "unparsable-url".into())
    );
    assert_eq!(net.calls(), 2, "each attempt made only the first request");
}

#[test]
fn an_unclosed_ipv6_bracket_is_never_requested() {
    // Python's urlsplit raised ValueError here, outside the request's handler.
    let net = Fake::new(vec![]);
    assert_eq!(
        check(&net, "https://[::1/", T),
        ("unreachable", "unparsable-url".into())
    );
    assert_eq!(net.calls(), 0);
}

#[test]
fn the_live_guard_resolves_before_any_request() {
    fn private(_: &str, port: u16) -> Result<Vec<SocketAddr>, DnsError> {
        Ok(vec![SocketAddr::new("10.0.0.5".parse().unwrap(), port)])
    }
    assert_eq!(
        check(&Live { lookup: private }, "https://internal.example/", T),
        ("unreachable", "non-public-address".into())
    );
}

#[test]
fn the_system_resolver_answers_numeric_hosts_offline() {
    for host in ["127.0.0.1", "[127.0.0.1]"] {
        assert_eq!(
            system_lookup(host, 8443).unwrap(),
            [SocketAddr::new("127.0.0.1".parse().unwrap(), 8443)],
            "{host}"
        );
    }
    assert_eq!(system_lookup("bad\0host", 443), Err(DnsError::Other));
}

#[test]
fn run_checks_every_collected_url() {
    struct AllOk;
    impl Network for AllOk {
        fn resolve(&self, _: &str, _: u16) -> Result<Vec<IpAddr>, DnsError> {
            Ok(vec![PUBLIC.parse().unwrap()])
        }
        fn get(&self, _: &str, _: Duration) -> Result<Reply, GetError> {
            Ok(Reply {
                status: 200,
                location: None,
            })
        }
    }
    let (dir, root) = fixture();
    let count = collect_urls(&root).unwrap().len();
    assert!(count > 0);
    let mut out = Vec::new();
    let root_arg = Some(dir.path().to_string_lossy().into_owned());
    assert_eq!(run(&root_arg, &AllOk, T, &mut out), 0);
    assert_eq!(
        String::from_utf8(out).unwrap(),
        format!("checked {count}: ok {count}, dead 0, unreachable 0, manual 0\n")
    );
    let empty = tempfile::tempdir().unwrap();
    let mut out = Vec::new();
    let missing = Some(empty.path().to_string_lossy().into_owned());
    assert_eq!(run(&missing, &AllOk, T, &mut out), 1);
    assert!(out.is_empty());
}
