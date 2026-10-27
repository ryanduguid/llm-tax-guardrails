//! Compilation currency: parsing the pins and naming a mismatch (ported from
//! test_sources.py). The Register is replaced by a scripted responder.

mod common;

use std::cell::RefCell;
use std::time::Duration;

use common::repo_root;
use drdebits::py::Date;
use drdebits::pyjson::{self, Json};
use drdebits::sources::{
    self, Current, Finding, Pin, Register, check, classify, current_version, parse_pins,
    registered_versions, upcoming,
};

const TABLE: &str = "## Source status

| Source | Version checked | Status | Role |
|---|---|---|---|
| [*Example Act 2009*](https://www.legislation.gov.au/C2009A00013/latest) | `C2025C00107`, Compilation No 26, 21 February 2025 | In force | Primary |
| [*Unpinned Act 1953*](https://www.legislation.gov.au/C1953A00001/latest) | Landing page checked; compilation ID not pinned this release | In force | Penalties |
| [TPB library](https://www.tpb.gov.au/policy-and-guidance) | 55 statements | Guidance | Interpretation |

The [repeal](https://www.legislation.gov.au/F2026L00916/asmade/details) is a
citation in prose, not a pinned source.
";

const T: Duration = Duration::from_secs(60);

/// Answers every request with `body` (or nothing) and records what was asked.
struct Scripted {
    body: Option<String>,
    asked: RefCell<Vec<(String, Duration)>>,
}

impl Scripted {
    fn new(body: Option<&str>) -> Scripted {
        Scripted {
            body: body.map(str::to_string),
            asked: RefCell::new(Vec::new()),
        }
    }
}

impl Register for Scripted {
    fn get(&self, url: &str, timeout: Duration) -> Option<String> {
        self.asked.borrow_mut().push((url.to_string(), timeout));
        self.body.clone()
    }
}

fn pin(name: &str, title: &str, register: &str, number: &str) -> Pin {
    Pin {
        name: name.into(),
        title_id: title.into(),
        register_id: register.into(),
        compilation_number: number.into(),
    }
}

fn example() -> Pin {
    pin("Example Act 2009", "C2009A00013", "C2025C00107", "26")
}

fn usable(register: &str, number: &str, start: &str) -> Current {
    Current::Usable {
        register_id: register.into(),
        compilation_number: number.into(),
        start: start.into(),
    }
}

fn day(s: &str) -> Date {
    Date::fromisoformat(s).unwrap()
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() + 1 {
            out.push(u8::from_str_radix(&text[i + 1..i + 3], 16).unwrap());
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

#[test]
fn parse_reads_the_pinned_compilation_and_its_number() {
    let pins = parse_pins(TABLE);
    let pinned: Vec<&Pin> = pins.iter().filter(|p| !p.register_id.is_empty()).collect();
    assert_eq!(pinned.len(), 1);
    assert_eq!(*pinned[0], example());
}

#[test]
fn parse_keeps_an_unpinned_row_and_ignores_the_rest() {
    let pins = parse_pins(TABLE);
    let ids: Vec<&str> = pins.iter().map(|p| p.title_id.as_str()).collect();
    assert_eq!(ids, ["C2009A00013", "C1953A00001"]);
    assert!(pins[1].register_id.is_empty());
    // A prose citation is not a pin, and only the Register can be checked.
    assert!(!ids.contains(&"F2026L00916"));
}

#[test]
fn a_newer_compilation_is_superseded_not_merely_different() {
    let finding = classify(&example(), Some(&usable("C2026C00500", "27", "2026-07-01")));
    assert_eq!(finding.outcome, "superseded");
    assert!(finding.detail.contains("C2026C00500") && finding.detail.contains("Compilation No 26"));
}

#[test]
fn a_matching_compilation_is_current() {
    assert_eq!(
        classify(&example(), Some(&usable("C2025C00107", "26", "2025-02-21"))).outcome,
        "current"
    );
}

#[test]
fn no_answer_is_unreachable_and_never_superseded() {
    assert_eq!(classify(&example(), None).outcome, "unreachable");
}

#[test]
fn current_null_metadata_reports_the_fetched_record() {
    for pinned in [true, false] {
        let body = r#"{"value": [{"titleId": "C2009A00013", "registerId": null, "compilationNumber": null, "start": "2026-10-01T00:00:00"}]}"#;
        let register = Scripted::new(Some(body));
        let p = if pinned {
            example()
        } else {
            pin("Example Act 2009", "C2009A00013", "", "")
        };
        let finding = classify(
            &p,
            current_version(&register, &p.title_id, Duration::from_secs(7)).as_ref(),
        );
        assert_eq!(finding.outcome, "incomplete");
        for needle in [
            "verification incomplete",
            "registerId=null and compilationNumber=null",
            "2026-10-01",
        ] {
            assert!(finding.detail.contains(needle), "{needle}");
        }
        assert!(
            !finding.detail.contains("did not answer") && !finding.detail.contains("SUPERSEDED")
        );
        let asked = register.asked.borrow();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].1, Duration::from_secs(7));
        let query = percent_decode(asked[0].0.split_once('?').unwrap().1);
        assert_eq!(
            query,
            "$top=1&$filter=titleId eq 'C2009A00013' and isCurrent eq true&$select=titleId,registerId,compilationNumber,start"
        );
    }
}

#[test]
fn unusable_current_records_remain_rejected() {
    for body in [
        "null",
        "[]",
        "{}",
        r#"{"value": []}"#,
        r#"{"value": [null]}"#,
        r#"{"value": [{"titleId": "C1953A00001", "registerId": null, "compilationNumber": null}]}"#,
        r#"{"value": [{"titleId": "C2009A00013", "registerId": null}]}"#,
        r#"{"value": [{"titleId": "C2009A00013", "registerId": "", "compilationNumber": null}]}"#,
        r#"{"value": [{"titleId": "C2009A00013", "registerId": "wrong", "compilationNumber": null}]}"#,
        "not json",
    ] {
        assert_eq!(
            current_version(&Scripted::new(Some(body)), "C2009A00013", T),
            None,
            "{body}"
        );
    }
}

#[test]
fn transport_failure_does_not_claim_fetched_metadata() {
    assert_eq!(
        current_version(&Scripted::new(None), "C2009A00013", T),
        None
    );
}

#[test]
fn a_comparable_id_keeps_legacy_missing_title_and_number_handling() {
    for title in ["null", "\"C2009A00013\""] {
        let body = format!(
            r#"{{"value": [{{"titleId": {title}, "registerId": "C2025C00107", "compilationNumber": null}}]}}"#
        );
        let version = current_version(&Scripted::new(Some(&body)), "C2009A00013", T);
        assert_eq!(version, Some(usable("C2025C00107", "", "")));
        assert_eq!(
            classify(
                &pin("Example", "C2009A00013", "C2025C00107", "26"),
                version.as_ref()
            )
            .outcome,
            "current"
        );
    }
}

#[test]
fn null_current_metadata_keeps_future_queries_skipped() {
    let body = r#"{"value": [{"titleId": "C2009A00013", "registerId": null, "compilationNumber": null, "start": "2026-10-01T00:00:00"}]}"#;
    let register = Scripted::new(Some(body));
    let (checked, findings) = check(&register, TABLE, day("2026-09-27"), T);
    assert_eq!(checked, 2);
    // The second pin gets another title's record back, which is refused outright.
    assert_eq!(
        findings.iter().map(|f| f.outcome).collect::<Vec<_>>(),
        ["incomplete", "unreachable"]
    );
    assert!(
        register
            .asked
            .borrow()
            .iter()
            .all(|(url, _)| url.contains("isCurrent")),
        "no future-version query"
    );
}

fn report(outcomes: &[&'static str]) -> (u8, String) {
    let findings: Vec<Finding> = outcomes
        .iter()
        .map(|o| Finding {
            pin: pin("Example", "C2009A00013", "C2025C00107", "26"),
            outcome: o,
            detail: "fixture".into(),
        })
        .collect();
    let mut out = Vec::new();
    let code = sources::report(outcomes.len(), &findings, &mut out);
    (code, String::from_utf8(out).unwrap())
}

#[test]
fn exit_precedence_is_unchanged() {
    for (outcomes, expected) in [
        (
            vec![
                "current",
                "unreachable",
                "unreachable",
                "unreachable",
                "unreachable",
            ],
            2,
        ),
        (
            vec![
                "current",
                "current",
                "incomplete",
                "incomplete",
                "incomplete",
            ],
            2,
        ),
        (vec!["superseded", "incomplete"], 1),
        (vec!["current"; 5], 0),
        (vec!["unpinned"], 2),
        (vec!["superseded", "unreachable"], 1),
        (vec!["upcoming", "unreachable"], 1),
        (vec![], 1),
    ] {
        let (code, out) = report(&outcomes);
        assert_eq!(code, expected, "{outcomes:?}");
        if outcomes.len() == 5 {
            assert!(out.contains("checked 5"));
        }
    }
}

#[test]
fn an_unpinned_row_reports_the_current_compilation_to_pin() {
    let finding = classify(
        &pin("Unpinned Act 1953", "C1953A00001", "", ""),
        Some(&usable("C2026C00393", "226", "2026-08-27")),
    );
    assert_eq!(finding.outcome, "unpinned");
    assert!(finding.detail.contains("C2026C00393"));
}

fn versions() -> Vec<Json> {
    let Json::Array(items) = pyjson::loads(
        r#"[
        {"titleId": "C2009A00013", "registerId": null, "start": "2034-10-01T00:00:00",
         "reasons": [{"affect": "Repeal", "markdown": "s 50 of the [Legislation Act 2003](/C2004A01224)"}]},
        {"titleId": "C2009A00013", "registerId": null, "start": "2026-10-01T00:00:00",
         "reasons": [{"affect": "Amend", "markdown": "sch 1 (items 1-69) of the [Treasury Laws Amendment (Example) Act 2026](/C2026A00086)"}]},
        {"titleId": "C2009A00013", "registerId": "C2025C00107", "start": "2025-02-21T00:00:00", "reasons": []}
    ]"#,
    )
    .unwrap() else {
        panic!()
    };
    items
}

#[test]
fn a_registered_amendment_inside_the_horizon_is_upcoming() {
    let findings = upcoming(&example(), Some(&versions()), day("2026-09-27"), 120);
    assert_eq!(
        findings.iter().map(|f| f.outcome).collect::<Vec<_>>(),
        ["upcoming"]
    );
    assert!(findings[0].detail.contains("2026-10-01"));
    assert!(
        findings[0]
            .detail
            .contains("Treasury Laws Amendment (Example) Act 2026 (C2026A00086)")
    );
}

#[test]
fn a_distant_sunset_and_past_versions_are_not_upcoming() {
    assert_eq!(
        upcoming(&example(), Some(&versions()), day("2026-09-27"), 120).len(),
        1
    );
    assert!(upcoming(&example(), Some(&versions()), day("2026-10-01"), 120).is_empty());
}

#[test]
fn upcoming_changes_are_reported_in_commencement_order() {
    let Json::Array(staged) = pyjson::loads(
        r#"[{"titleId": "C2009A00013", "start": "2026-12-01T00:00:00", "reasons": []},
            {"titleId": "C2009A00013", "start": "2026-10-01T00:00:00", "reasons": []}]"#,
    )
    .unwrap() else {
        panic!()
    };
    let details: Vec<String> = upcoming(&example(), Some(&staged), day("2026-09-27"), 120)
        .into_iter()
        .map(|f| f.detail)
        .collect();
    assert!(details[0].contains("2026-10-01") && details[1].contains("2026-12-01"));
    assert!(details[0].contains("the Register records no reason"));
}

#[test]
fn future_versions_are_requested_nearest_first() {
    let register = Scripted::new(Some(r#"{"value": []}"#));
    assert_eq!(
        registered_versions(&register, "C2009A00013", day("2026-09-27"), T),
        Some(vec![])
    );
    let url = percent_decode(&register.asked.borrow()[0].0);
    assert!(
        url.contains("$orderby=start asc") && url.contains("start gt 2026-09-27T00:00:00"),
        "{url}"
    );
}

#[test]
fn an_unreadable_start_date_leaves_the_check_incomplete() {
    let Json::Array(items) =
        pyjson::loads(r#"[{"titleId": "C2009A00013", "start": null, "reasons": []}]"#).unwrap()
    else {
        panic!()
    };
    assert_eq!(
        upcoming(&example(), Some(&items), day("2026-09-27"), 120)[0].outcome,
        "unreachable"
    );
}

#[test]
fn no_answer_on_future_versions_is_unreachable_not_clean() {
    assert_eq!(
        upcoming(&example(), None, day("2026-09-27"), 120)[0].outcome,
        "unreachable"
    );
}

#[test]
fn the_real_source_status_table_still_parses() {
    // A parser that silently finds nothing would report a clean check.
    let text = std::fs::read_to_string(repo_root().join("src/guide/040-source-status.md")).unwrap();
    let pins = parse_pins(&text);
    assert!(pins.len() >= 5);
    let mut ids: Vec<&str> = pins.iter().map(|p| p.title_id.as_str()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), pins.len(), "no title is pinned twice");
    assert!(pins.iter().any(|p| !p.register_id.is_empty()));
}

#[test]
fn summary_counts_incomplete_records_separately() {
    let (code, out) = report(&["current", "incomplete", "unreachable"]);
    assert_eq!(code, 2);
    assert!(out.contains("INCOMPLETE  Example: fixture"));
    assert!(out.contains("current 1, superseded 0, unpinned 0, incomplete 1, unreachable 1,"));
}

#[test]
fn null_current_metadata_with_an_unreadable_start_is_still_incomplete() {
    let body = r#"{"value": [{"titleId": "C2009A00013", "registerId": null, "compilationNumber": null, "start": "not a date"}]}"#;
    assert_eq!(
        current_version(&Scripted::new(Some(body)), "C2009A00013", T),
        Some(Current::Incomplete("unavailable".into()))
    );
}

#[test]
fn unusable_future_version_answers_are_no_answer() {
    for body in [
        r#"{"value": "not a list"}"#,
        r#"{"value": [{"titleId": "C2009A00013"}, "not a record"]}"#,
        r#"{"value": [{"titleId": "C1953A00001", "start": "2027-01-01"}]}"#,
    ] {
        assert_eq!(
            registered_versions(
                &Scripted::new(Some(body)),
                "C2009A00013",
                day("2026-02-01"),
                T
            ),
            None,
            "{body}"
        );
    }
    assert_eq!(
        registered_versions(&Scripted::new(None), "C2009A00013", day("2026-02-01"), T),
        None
    );
}

#[test]
fn a_comparable_current_version_also_checks_upcoming_versions() {
    struct Both;
    impl Register for Both {
        fn get(&self, url: &str, _: Duration) -> Option<String> {
            Some(if url.contains("isCurrent") {
                let title = if url.contains("C2009A00013") {
                    "C2009A00013"
                } else {
                    "C1953A00001"
                };
                format!(
                    r#"{{"value": [{{"titleId": "{title}", "registerId": "C2025C00107", "compilationNumber": "26", "start": "2025-01-01"}}]}}"#
                )
            } else {
                r#"{"value": []}"#.to_string()
            })
        }
    }
    let (checked, findings) = check(&Both, TABLE, day("2026-09-27"), T);
    assert_eq!(checked, 2);
    assert_eq!(
        findings.iter().map(|f| f.outcome).collect::<Vec<_>>(),
        ["current", "unpinned"]
    );
}

#[test]
fn the_date_is_taken_in_australia_not_on_the_runner() {
    // 21:00 UTC on 30 September is already 1 October, the commencement day.
    let seconds = (day("2026-09-30").ordinal() - 719_163) * 86_400 + 21 * 3600;
    assert_eq!(sources::register_date(seconds), day("2026-10-01"));
    assert_eq!(
        sources::register_date(seconds - 8 * 3600),
        day("2026-09-30")
    );
}

#[test]
fn a_version_without_a_reasons_list_records_no_reason() {
    let Json::Array(versions) = pyjson::loads(
        r#"[{"titleId": "C2009A00013", "start": "2026-10-01T00:00:00"},
            {"titleId": "C2009A00013", "start": "2026-10-02T00:00:00", "reasons": "Amend"}]"#,
    )
    .unwrap() else {
        panic!("a JSON array")
    };
    let details: Vec<String> = upcoming(&example(), Some(&versions), day("2026-09-27"), 120)
        .into_iter()
        .map(|f| f.detail)
        .collect();
    assert_eq!(
        details,
        [
            "a registered change comes into force on 2026-10-01: the Register records no reason",
            "a registered change comes into force on 2026-10-02: the Register records no reason",
        ]
    );
}

/// Serve one HTTP response to the first request on a local port; return its URL.
fn serve_once(body: &'static [u8]) -> (String, std::thread::JoinHandle<String>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1/Titles", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut byte = [0u8; 1];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
        String::from_utf8_lossy(&request).into_owned()
    });
    (url, server)
}

#[test]
fn the_live_register_returns_the_body_as_text() {
    let (url, server) = serve_once(br#"{"value": []}"#);
    assert_eq!(
        sources::Live.get(&url, Duration::from_secs(10)).as_deref(),
        Some(r#"{"value": []}"#)
    );
    let request = server.join().unwrap();
    assert!(
        request.starts_with("GET /v1/Titles HTTP/1.1\r\n"),
        "{request}"
    );
    assert!(
        request.contains("llm-tax-guardrails source-check"),
        "the request names its user agent: {request}"
    );
}

#[test]
fn the_live_register_treats_an_undecodable_or_missing_answer_as_no_answer() {
    let (url, server) = serve_once(b"\xff\xfe");
    assert_eq!(sources::Live.get(&url, Duration::from_secs(10)), None);
    server.join().unwrap();
    // Nothing listens on a port once its listener is dropped.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", closed.local_addr().unwrap());
    drop(closed);
    assert_eq!(sources::Live.get(&url, Duration::from_secs(10)), None);
}

#[test]
fn the_sources_command_reports_a_missing_or_empty_source_table() {
    let (dir, _root) = common::fixture();
    let out = common::run_in(dir.path(), &["sources"], true);
    assert_eq!(out.code, 1);
    assert!(
        out.stderr.starts_with("sources: [Errno 2] ")
            && out.stderr.contains("040-source-status.md"),
        "{}",
        out.stderr
    );
    std::fs::write(
        dir.path().join("src/guide/040-source-status.md"),
        "## Source status\n\nNo Register sources this release.\n",
    )
    .unwrap();
    let out = common::run_in(dir.path(), &["sources"], true);
    assert_eq!(
        (out.code, out.stdout.as_str()),
        (1, "no Register sources found in the source-status table\n")
    );
}
