//! Compilation currency for the pinned Register sources. Report-only.
//!
//! `linkcheck` answers whether a source URL still resolves. It cannot answer
//! the question that decides this guide's accuracy: whether the compilation
//! the guide pins is still the current one. A pinned compilation keeps
//! resolving long after it has been superseded.
//!
//! The Federal Register publishes the answer through its API, so this needs
//! no scraping and no judgement. A difference is reportable; it is not a
//! licence to advance `sources_checked_at`, because the guide's source date
//! means a competent human read the material and this only means a number
//! moved.
//!
//! The pins come out of `src/guide/040-source-status.md`, where they are
//! already maintained. Each title's registered future versions are listed
//! too, because an amending instrument is registered weeks before it
//! commences; one that comes into force within `HORIZON_DAYS` is a finding.

use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;

use crate::builder;
use crate::net::quote;
use crate::py::{self, Date, MAX_DATE};
use crate::pyjson::{self, Json};

const API: &str = "https://api.prod.legislation.gov.au/v1";
// The API host is not the website host; if the API ever refuses too, the run
// exits 2 for incomplete verification. A missing answer never establishes
// that a compilation has been superseded.
const UA: &str =
    "llm-tax-guardrails source-check (+https://github.com/ryanduguid/llm-tax-guardrails)";
const TIMEOUT: u64 = 60;
const SELECT: &str = "titleId,registerId,compilationNumber,start";
// Longer than the gap between source reviews, so a change registered just
// after one review is still inside the window when the next weekly run reports it.
const HORIZON_DAYS: i64 = 120;
// Future versions come back nearest first, so a full page can only leave out
// changes later than every one it holds.
const FUTURE_VERSIONS: usize = 10;

fn title_id() -> String {
    format!("[A-Z]{d}{{4}}[A-Z]{d}{{5}}", d = *py::D)
}

// Only the source-status table is read; the same file cites a repealed
// instrument in prose, and that link is a citation, not a pin.
static TABLE_ROW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?m)^\|{s}*\[(?P<name>[^\]]+)\]\(https://www\.legislation\.gov\.au/(?P<title_id>{t})/latest\){s}*\|(?P<pin>[^|]*)\|",
        s = *py::S,
        t = title_id()
    ))
    .expect("pattern")
});
static PINNED_COMPILATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!("`(?P<register_id>{})`", title_id())).expect("pattern"));
static COMPILATION_NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        "{}{s}*(?P<number>{d}+)",
        py::ignorecase_literal("Compilation No"),
        s = *py::S,
        d = *py::D
    ))
    .expect("pattern")
});
// The Register writes an amending title as a Markdown link to its own path.
static REASON_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"\[(?P<name>[^\]]+)\]\(/(?P<title_id>{})\)",
        title_id()
    ))
    .expect("pattern")
});
static TITLE_ID_FULL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!("^{}\\z", title_id())).expect("pattern"));

const CURRENT: &str = "current";
// The Register answered for the title but gave no compilation to compare.
const INCOMPLETE: &str = "incomplete";
const SUPERSEDED: &str = "superseded";
const UNPINNED: &str = "unpinned";
const UNREACHABLE: &str = "unreachable";
const UPCOMING: &str = "upcoming";

/// One Register source as the guide records it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pin {
    pub name: String,
    pub title_id: String,
    pub register_id: String,
    pub compilation_number: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub pin: Pin,
    pub outcome: &'static str,
    pub detail: String,
}

/// What the current-version query gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Current {
    Usable {
        register_id: String,
        compilation_number: String,
        start: String,
    },
    /// A record with null registerId and compilationNumber: nothing to compare.
    Incomplete(String),
}

/// Strip the Markdown emphasis the table uses around statute titles.
fn clean(name: &str) -> String {
    py::strip(&name.replace('*', "")).to_string()
}

/// Read every Register source out of the source-status table.
pub fn parse_pins(text: &str) -> Vec<Pin> {
    TABLE_ROW
        .captures_iter(text)
        .map(|row| {
            let cell = &row["pin"];
            Pin {
                name: clean(&row["name"]),
                title_id: row["title_id"].to_string(),
                register_id: PINNED_COMPILATION
                    .captures(cell)
                    .map(|c| c["register_id"].to_string())
                    .unwrap_or_default(),
                compilation_number: COMPILATION_NUMBER
                    .captures(cell)
                    .map(|c| c["number"].to_string())
                    .unwrap_or_default(),
            }
        })
        .collect()
}

/// Fetches a Register response body; None for any network, HTTP or decoding failure.
pub trait Register {
    fn get(&self, url: &str, timeout: Duration) -> Option<String>;
}

fn fetch_json(register: &dyn Register, url: &str, timeout: Duration) -> Option<Json> {
    pyjson::loads(&register.get(url, timeout)?).ok()
}

fn title_matches(version: &Json, title_id: &str) -> bool {
    matches!(version.get("titleId"), None | Some(Json::Null))
        || version.get("titleId").and_then(Json::as_str) == Some(title_id)
}

fn field_str(version: &Json, key: &str) -> String {
    version
        .get(key)
        .filter(|v| v.truthy())
        .map(Json::py_str)
        .unwrap_or_default()
}

/// Comparable or incomplete current metadata, or None for an unusable result.
///
/// `$filter` with `isCurrent eq true` is the documented way to resolve this;
/// key lookup and a `/latest/` path are not aliases for it.
pub fn current_version(
    register: &dyn Register,
    title_id: &str,
    timeout: Duration,
) -> Option<Current> {
    let query = quote(&format!("titleId eq '{title_id}' and isCurrent eq true"));
    let url = format!("{API}/versions?$top=1&$filter={query}&$select={SELECT}");
    let document = fetch_json(register, &url, timeout)?;
    let values = match document.get("value") {
        Some(Json::Array(values)) if !values.is_empty() => values,
        _ => return None,
    };
    let version = &values[0];
    if !matches!(version, Json::Object(_)) {
        return None;
    }
    // A response carrying another Act's record is worse than no response.
    if !title_matches(version, title_id) {
        return None;
    }
    let is_null = |key: &str| matches!(version.get(key), Some(Json::Null));
    if version.get("titleId").and_then(Json::as_str) == Some(title_id)
        && is_null("registerId")
        && is_null("compilationNumber")
    {
        let start = Date::fromisoformat(py::prefix_chars(&field_str(version, "start"), 10))
            .map_or_else(|| "unavailable".to_string(), |d| d.isoformat());
        return Some(Current::Incomplete(start));
    }
    let register_id = version.get("registerId").and_then(Json::as_str)?;
    if !TITLE_ID_FULL.is_match(register_id) {
        return None;
    }
    Some(Current::Usable {
        register_id: field_str(version, "registerId"),
        compilation_number: field_str(version, "compilationNumber"),
        start: field_str(version, "start"),
    })
}

/// The title's versions starting after `after`, nearest first, or None.
pub fn registered_versions(
    register: &dyn Register,
    title_id: &str,
    after: Date,
    timeout: Duration,
) -> Option<Vec<Json>> {
    let query = quote(&format!(
        "titleId eq '{title_id}' and start gt {}T00:00:00",
        after.isoformat()
    ));
    let order = quote("start asc");
    let url = format!("{API}/versions?$top={FUTURE_VERSIONS}&$orderby={order}&$filter={query}");
    let document = fetch_json(register, &url, timeout)?;
    let Some(Json::Array(values)) = document.get("value") else {
        return None;
    };
    if !values.iter().all(|v| matches!(v, Json::Object(_))) {
        return None;
    }
    if !values.iter().all(|v| title_matches(v, title_id)) {
        return None;
    }
    Some(values.clone())
}

/// Summarise why a version exists, from the Register's own reasons.
fn describe(version: &Json) -> String {
    let parts: Vec<String> = match version.get("reasons") {
        Some(Json::Array(reasons)) => reasons
            .iter()
            .filter(|r| matches!(r, Json::Object(_)))
            .map(|reason| {
                let affect = reason
                    .get("affect")
                    .filter(|v| v.truthy())
                    .map_or_else(|| "Change".into(), Json::py_str);
                let markdown = field_str(reason, "markdown");
                format!(
                    "{affect}: {}",
                    REASON_LINK.replace_all(&markdown, "${name} (${title_id})")
                )
            })
            .collect(),
        _ => Vec::new(),
    };
    if parts.is_empty() {
        "the Register records no reason".into()
    } else {
        parts.join("; ")
    }
}

/// Today's date in Australia, where commencement dates fall, not the runner's.
///
/// shortcut: a fixed UTC+10 ignores daylight saving, so between 13:00 and
/// 14:00 UTC in summer it still reads the previous day; use a time zone
/// database if a run is ever scheduled in that hour.
pub fn register_today() -> Date {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    register_date(seconds)
}

/// The Australian (UTC+10) date at `unix_seconds`.
pub fn register_date(unix_seconds: i64) -> Date {
    let days = (unix_seconds + 10 * 3600).div_euclid(86_400);
    // 1970-01-01 is ordinal 719163.
    Date::from_ordinal(719_163 + days).expect("a date in range")
}

/// Report each registered version that comes into force within the horizon.
pub fn upcoming(
    pin: &Pin,
    versions: Option<&[Json]>,
    today: Date,
    horizon_days: i64,
) -> Vec<Finding> {
    let Some(versions) = versions else {
        return vec![Finding {
            pin: pin.clone(),
            outcome: UNREACHABLE,
            detail: "the Register did not answer for this title's future versions".into(),
        }];
    };
    let cutoff = today.add_days(horizon_days).unwrap_or(MAX_DATE);
    let mut findings: Vec<(Date, Finding)> = Vec::new();
    for version in versions {
        match Date::fromisoformat(py::prefix_chars(&field_str(version, "start"), 10)) {
            // A registered change with no readable date could fall anywhere,
            // so it leaves the check incomplete rather than clean.
            None => findings.push((
                MAX_DATE,
                Finding {
                    pin: pin.clone(),
                    outcome: UNREACHABLE,
                    detail: format!(
                        "the Register listed a future version without a readable start date: {}",
                        describe(version)
                    ),
                },
            )),
            Some(start) if today < start && start <= cutoff => findings.push((
                start,
                Finding {
                    pin: pin.clone(),
                    outcome: UPCOMING,
                    detail: format!(
                        "a registered change comes into force on {}: {}",
                        start.isoformat(),
                        describe(version)
                    ),
                },
            )),
            Some(_) => {}
        }
    }
    findings.sort_by_key(|(date, _)| *date);
    findings.into_iter().map(|(_, f)| f).collect()
}

pub fn classify(pin: &Pin, version: Option<&Current>) -> Finding {
    let finding = |outcome, detail: String| Finding {
        pin: pin.clone(),
        outcome,
        detail,
    };
    match version {
        None => finding(
            UNREACHABLE,
            "the Register did not provide a usable current-version result".into(),
        ),
        Some(Current::Incomplete(start)) => finding(
            INCOMPLETE,
            format!(
                "verification incomplete: the current-version query returned a record for {} with registerId=null \
                 and compilationNumber=null; API-reported start={start}. This response provides no compilation ID \
                 to compare with the guide's pin. Future versions were not checked.",
                pin.title_id
            ),
        ),
        Some(Current::Usable {
            register_id,
            compilation_number,
            start,
        }) => {
            let live = format!("{register_id}, Compilation No {compilation_number}");
            let served = if start.is_empty() {
                String::new()
            } else {
                format!(" (in force from {})", py::prefix_chars(start, 10))
            };
            if pin.register_id.is_empty() {
                finding(
                    UNPINNED,
                    format!("the guide pins no compilation; current is {live}{served}"),
                )
            } else if *register_id == pin.register_id {
                finding(CURRENT, format!("still current at {live}"))
            } else {
                finding(
                    SUPERSEDED,
                    format!(
                        "the guide reads {}, Compilation No {}; current is {live}{served}",
                        pin.register_id, pin.compilation_number
                    ),
                )
            }
        }
    }
}

/// The number of sources checked and every finding for them.
pub fn check(
    register: &dyn Register,
    text: &str,
    today: Date,
    timeout: Duration,
) -> (usize, Vec<Finding>) {
    let pins = parse_pins(text);
    let mut findings = Vec::new();
    for pin in &pins {
        let version = current_version(register, &pin.title_id, timeout);
        findings.push(classify(pin, version.as_ref()));
        // Unusable or incomplete current metadata is already reported.
        if matches!(version, Some(Current::Usable { .. })) {
            let versions = registered_versions(register, &pin.title_id, today, timeout);
            findings.extend(upcoming(pin, versions.as_deref(), today, HORIZON_DAYS));
        }
    }
    (pins.len(), findings)
}

/// Print the report and return the exit code.
pub fn report(checked: usize, findings: &[Finding], out: &mut dyn std::io::Write) -> u8 {
    if checked == 0 {
        let _ = writeln!(out, "no Register sources found in the source-status table");
        return 1;
    }
    for f in findings {
        let _ = writeln!(
            out,
            "{:<11} {}: {}",
            f.outcome.to_uppercase(),
            f.pin.name,
            f.detail
        );
    }
    let count = |name: &str| findings.iter().filter(|f| f.outcome == name).count();
    let _ = writeln!(
        out,
        "checked {checked}: current {}, superseded {}, unpinned {}, incomplete {}, unreachable {}, upcoming within \
         {HORIZON_DAYS} days {}",
        count(CURRENT),
        count(SUPERSEDED),
        count(UNPINNED),
        count(INCOMPLETE),
        count(UNREACHABLE),
        count(UPCOMING)
    );
    if count(SUPERSEDED) > 0 {
        let _ = writeln!(
            out,
            "A superseded compilation means the guide reads material that is no longer in force. Label the affected \
             material SOURCE CURRENCY NOT CONFIRMED until a competent human has reviewed the replacement. Do not \
             advance sources_checked_at on the strength of this check."
        );
    }
    if count(UPCOMING) > 0 {
        let _ = writeln!(
            out,
            "An upcoming change means the pinned text is scheduled to change. Read the amending title before it comes \
             into force, decide the guide's treatment of conduct either side of that date, and re-pin once the new \
             compilation is registered."
        );
    }
    if count(SUPERSEDED) > 0 || count(UPCOMING) > 0 {
        return 1;
    }
    if count(INCOMPLETE) > 0 || count(UNREACHABLE) > 0 || count(UNPINNED) > 0 {
        2
    } else {
        0
    }
}

/// The Register over HTTPS, with the environment's proxy and redirects.
pub struct Live;

impl Register for Live {
    fn get(&self, url: &str, timeout: Duration) -> Option<String> {
        let config = ureq::Agent::config_builder()
            .user_agent(UA)
            .timeout_global(Some(timeout))
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .build(),
            )
            .build();
        let agent: ureq::Agent = config.into();
        let mut response = agent.get(url).call().ok()?;
        let bytes = response
            .body_mut()
            .with_config()
            .limit(64 * 1024 * 1024)
            .read_to_vec()
            .ok()?;
        py::decode_utf8(&bytes).ok()
    }
}

#[derive(clap::Args)]
pub struct Args {
    /// Seconds to wait for each Register request.
    #[arg(long, default_value_t = TIMEOUT)]
    timeout: u64,
}

pub fn main(root: Option<&str>, args: Args) -> u8 {
    let text = match crate::root_for(root).and_then(|root| {
        builder::read_text(&root.join("src").join("guide").join("040-source-status.md"))
    }) {
        Ok(text) => text,
        Err(e) => {
            errln!("sources: {e}");
            return 1;
        }
    };
    let (checked, findings) = check(
        &Live,
        &text,
        register_today(),
        Duration::from_secs(args.timeout),
    );
    report(checked, &findings, &mut crate::PyText::stdout())
}
