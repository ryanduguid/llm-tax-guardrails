//! Rebuild-and-compare verification. Returns findings instead of failing.
//!
//! An `Err` means the checks themselves could not run, which the CLI reports
//! as "not checked" rather than as a finding.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;

use crate::builder::{self, SOURCE_CHECK_DATE_FILES, STAMPED_FILES};
use crate::error::Fail;
use crate::evals;
use crate::model::meta_get;
use crate::py::{self, Date, PyPath};
use crate::yaml::{self, Value};

// The two shapes the hand-written TPB statement count and GS range are
// written in outside the generated catalogue header. Check (h) compares every
// match against the sources.
static GS_RANGE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!("GS{d}+ to GS{d}+", d = *py::D)).expect("pattern"));
static GS_COUNT_RE: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
    fancy_regex::Regex::new(&format!(
        r"(?<!{d})({d}+) (?:indexed statements|live Guidance Statements)",
        d = *py::D
    ))
    .expect("pattern")
});

// How many copies of each shape each file must carry, so a copy that is
// deleted or reworded past the patterns is a failure rather than silence.
// The copies are the ones MAINTENANCE step 8 tells a maintainer to update.
const GS_COPY_COUNTS: [(&str, usize, usize); 2] = [("drdebits.md", 3, 2), ("README.md", 1, 0)];

// The two guide fragments checks (i) and (j) cross-reference the data files
// against. Named, not discovered: a renamed or deleted fragment must surface
// as a finding.
const APES_CONTROL_SET_FRAGMENT: &str = "150-apes-110-control-set.md";
const INSTRUCTION_WORDS_FRAGMENT: &str = "060-meaning-of-instruction-words.md";

// An APES 110 Part or section locator in prose: "Part 4B", "Parts 1 to 4B",
// "section 280", "sections 310, 320 and 321". The leading word boundary keeps
// "Subsections" out of the sections arm, which the optional "Sub" then
// matches on its own. Paragraph references deliberately do not match.
static APES_LOCATOR_RE: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
    let (d, s) = (&*py::D, &*py::S);
    fancy_regex::Regex::new(&format!(
        r"{b}(?:(?P<part>[Pp]arts?)|(?:[Ss]ub)?[Ss]ections?){s}+(?P<numbers>{d}+[A-Za-z]?(?:{s}*(?:,|and|to|or){s}*{d}+[A-Za-z]?)*)",
        b = py::word_boundary()
    ))
    .expect("pattern")
});
static LOCATOR_NUMBER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!("{}+[A-Za-z]?", *py::D)).expect("pattern"));

// A defined instruction word or outcome label: the bold run in one bullet.
static INSTRUCTION_WORD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\*\*(.+?)\*\*").expect("pattern"));

/// The APES 110 Parts and sections a passage cites, as ("Part"|"section", id).
fn apes_locators(text: &str) -> BTreeSet<(String, String)> {
    let mut found = BTreeSet::new();
    for m in APES_LOCATOR_RE.captures_iter(text) {
        let m = m.expect("a bounded match");
        let kind = if m.name("part").is_some() {
            "Part"
        } else {
            "section"
        };
        for number in LOCATOR_NUMBER_RE.find_iter(&m["numbers"]) {
            found.insert((kind.to_string(), number.as_str().to_string()));
        }
    }
    found
}

fn rel_text(root: &PyPath, rel: &str) -> Result<Option<String>, Fail> {
    let p = root.join(rel);
    if !p.fs().is_file() {
        return Ok(None);
    }
    builder::read_text(&p).map(Some)
}

/// CITATION.cff as one YAML mapping, read as the source files are. `Ok(Err)`
/// carries the reason it could not be read, which verify reports as a finding.
fn read_citation(path: &PyPath) -> Result<Result<Vec<(Value, Value)>, String>, Fail> {
    let text = match builder::read_text(path) {
        Ok(text) => text,
        Err(Fail::Os(message) | Fail::Decode(message)) => return Ok(Err(message)),
        Err(e) => return Err(e),
    };
    Ok(match yaml::load(&text) {
        Ok(Value::Dict(pairs)) => Ok(pairs.borrow().clone()),
        Ok(_) => Err(format!("{path}: top level must be a mapping")),
        Err(yaml::Error::Yaml(message)) => Err(message),
        Err(yaml::Error::TooDeep) => Err("maximum recursion depth exceeded".into()),
        // PyYAML raises ValueError for an invalid unquoted date scalar.
        Err(yaml::Error::Internal(message)) => match message.strip_prefix("ValueError: ") {
            Some(reason) => Err(reason.to_string()),
            None => return Err(Fail::Internal(message)),
        },
    })
}

pub fn run_verify(root: &PyPath, today: Date) -> Result<Vec<String>, Fail> {
    let s = match builder::load_sources(root) {
        Ok(s) => s,
        Err(e @ (Fail::Model(_) | Fail::Build(_) | Fail::Os(_))) => {
            return Ok(vec![format!("sources: {e}")]);
        }
        Err(e) => return Err(e),
    };
    let mut failures: Vec<String> = Vec::new();
    let mut built = builder::render_generated(&s);

    // A missing checksum_files member is a finding, not a crash.
    match builder::build_sha256sums(root, &s, &built) {
        Ok(sums) => built.push(("SHA256SUMS".into(), sums)),
        Err(e @ (Fail::Model(_) | Fail::Build(_) | Fail::Os(_))) => {
            failures.push(format!("SHA256SUMS: cannot rebuild ({e})"));
        }
        Err(e) => return Err(e),
    }
    // A malformed result file is a finding too.
    match evals::build_results_md(root, &s) {
        Ok(text) => built.push((evals::RESULTS_FILE.into(), text)),
        Err(e @ (Fail::Model(_) | Fail::Build(_) | Fail::Os(_))) => {
            failures.push(format!("{}: cannot rebuild ({e})", evals::RESULTS_FILE));
        }
        Err(e) => return Err(e),
    }

    for (rel, content) in &built {
        let committed = root.join(rel);
        if !committed.fs().is_file() {
            failures.push(format!("{rel}: missing committed file"));
        } else if builder::read_bytes(&committed)? != content.as_bytes() {
            failures.push(format!(
                "{rel}: committed file differs from build output; edit src/ and run build"
            ));
        }
    }

    // The committed guide is read once and reused below; a missing file is
    // already reported above, so those checks skip.
    let committed_text = rel_text(root, "drdebits.md")?;

    // The end marker is checked on the committed guide, so a truncated commit is caught.
    let marker = s.meta("guide_end_marker");
    if let Some(text) = &committed_text {
        let trimmed = text.trim_end_matches('\n');
        let last = trimmed.rsplit('\n').next().unwrap_or("");
        if last != marker {
            failures.push(format!(
                "end marker: last line {} != {}",
                py::repr_str(last),
                py::repr_str(marker)
            ));
        }
    }

    // Stamped files must exist and carry at least one version token each.
    let version = s.meta("guide_version");
    for rel in STAMPED_FILES {
        match rel_text(root, rel)? {
            None => failures.push(format!("{rel}: missing")),
            Some(text) => {
                let matches = builder::stamp_matches(&text);
                if matches.is_empty() {
                    failures.push(format!("{rel}: no version stamp found"));
                }
                for (start, end) in matches {
                    let found = &text[start..end];
                    if found != version {
                        failures.push(format!("{rel}: version {found} != {version}"));
                    }
                }
            }
        }
    }

    // (a) the committed drdebits.md header must carry the exact version line.
    if let Some(text) = &committed_text {
        let line = format!("> Version: `{version}`");
        if !py::splitlines(text).contains(&line.as_str()) {
            failures.push("drdebits.md: header version line does not match guide_version".into());
        }
    }

    // (b) release_tag must be "v" + guide_version.
    let release_tag = s.meta("release_tag");
    if release_tag != format!("v{version}") {
        failures.push(format!("metadata: release_tag {release_tag} != v{version}"));
    }

    // (c) guide_end_marker must be "DRDEBITS-END-" + release_tag.
    if marker != format!("DRDEBITS-END-{release_tag}") {
        failures.push(format!(
            "metadata: guide_end_marker {marker} != DRDEBITS-END-{release_tag}"
        ));
    }

    // (d) the newest (first) changelog entry must match guide_version.
    let newest = s.changelog[0].get("version");
    if newest != version {
        failures.push(format!("changelog: newest entry {newest} != {version}"));
    }

    // (e) the guide header, README and llms.txt carry hand-written copies of
    // the source-check date, so a metadata bump that misses one is caught.
    let checked_date = py::prefix_chars(s.meta("sources_checked_at"), 10).to_string();
    if !checked_date.is_empty() {
        // Substring: the real header line carries a timezone suffix.
        let guide_line = format!("> Sources last checked: `{checked_date}`");
        if committed_text
            .as_ref()
            .is_some_and(|t| !t.contains(&guide_line))
        {
            failures.push(format!(
                "drdebits.md: header source-check date line does not match sources_checked_at ({checked_date})"
            ));
        }
        for rel in SOURCE_CHECK_DATE_FILES {
            let Some(text) = rel_text(root, rel)? else {
                continue;
            };
            if !text.contains(&format!("Sources last checked: {checked_date}")) {
                failures.push(format!(
                    "{rel}: source-check date does not match sources_checked_at ({checked_date})"
                ));
            }
        }
    }

    let candidate = s.changelog[0].get("status") == "Unreleased";
    let candidate_status = meta_get(&s.meta, "status") == Some("candidate");
    if candidate && !candidate_status {
        failures.push("metadata: Unreleased changelog requires candidate status".into());
    }
    if !candidate && candidate_status {
        failures.push("metadata: candidate status requires Unreleased changelog".into());
    }

    // (f) CITATION.cff, when present, carries its own version and release
    // date, read as YAML so quoted keys and scalars are both accepted. An
    // Unreleased candidate must omit date-released.
    let citation = root.join("CITATION.cff");
    if citation.fs().is_file() {
        match read_citation(&citation)? {
            Err(message) => {
                failures.push(format!("CITATION.cff: cannot parse YAML ({message})"));
            }
            Ok(record) => {
                let field = |key: &str| record.iter().find(|(k, _)| k.as_str() == Some(key));
                if field("version").and_then(|(_, v)| v.as_str()) != Some(version) {
                    failures.push(format!(
                        "CITATION.cff: version does not match guide_version {version}"
                    ));
                }
                let release_date = s.changelog[0].get("date");
                let released = field("date-released").map(|(_, v)| v);
                if candidate {
                    if released.is_some() {
                        failures.push(
                            "CITATION.cff: Unreleased candidate must omit date-released".into(),
                        );
                    }
                } else if !released.is_some_and(|v| match v {
                    Value::Str(text) => text == release_date,
                    Value::Date(date) => date.isoformat() == release_date,
                    _ => false,
                }) {
                    failures.push(format!(
                        "CITATION.cff: date-released does not match newest changelog date {release_date}"
                    ));
                }
            }
        }
    }

    // (g) once review_due has passed the guide labels its own contents
    // SOURCE CURRENCY NOT CONFIRMED, so review_due must fall after both the
    // source-check date and the day verify runs.
    let review_due = s.meta("review_due");
    match Date::fromisoformat(review_due) {
        None => failures.push(format!(
            "metadata: review_due must be an ISO date (YYYY-MM-DD), got {}",
            py::repr_str(review_due)
        )),
        Some(due) => {
            let checked =
                Date::fromisoformat(&checked_date).expect("validated when the sources loaded");
            if due <= checked {
                failures.push(format!(
                    "metadata: review_due {review_due} is not after sources_checked_at {}; the release would ship \
                     already due for review",
                    checked.isoformat()
                ));
            }
            if due <= today {
                failures.push(format!(
                    "metadata: review_due {review_due} is not after the verification date {}; the sources are due \
                     for review now, so the guide labels its own contents SOURCE CURRENCY NOT CONFIRMED",
                    today.isoformat()
                ));
            }
        }
    }

    // (h) every hand-written copy of the statement count and GS range must
    // agree with the sources the catalogue header is built from, and each
    // file must carry the documented number of copies.
    let gs_count = s.meta("tpb_guidance_statement_count");
    let gs_range = format!(
        "{} to {}",
        s.catalogue[0].get("id"),
        s.catalogue[s.catalogue.len() - 1].get("id")
    );
    for (rel, ranges_expected, counts_expected) in GS_COPY_COUNTS {
        let Some(text) = rel_text(root, rel)? else {
            continue;
        };
        let ranges: Vec<&str> = GS_RANGE_RE.find_iter(&text).map(|m| m.as_str()).collect();
        let counts: Vec<String> = GS_COUNT_RE
            .captures_iter(&text)
            .map(|c| {
                c.expect("a bounded match")
                    .get(1)
                    .expect("group")
                    .as_str()
                    .to_string()
            })
            .collect();
        for value in &ranges {
            if *value != gs_range {
                failures.push(format!(
                    "{rel}: statement range {} does not match the catalogue range {}",
                    py::repr_str(value),
                    py::repr_str(&gs_range)
                ));
            }
        }
        for value in &counts {
            if value != gs_count {
                failures.push(format!(
                    "{rel}: statement count {value} does not match tpb_guidance_statement_count {gs_count}"
                ));
            }
        }
        for (shape, found, expected) in [
            ("range", ranges.len(), ranges_expected),
            ("count", counts.len(), counts_expected),
        ] {
            if found != expected {
                failures.push(format!(
                    "{rel}: found {found} statement {shape} copies, expected {expected}; a hand-written copy has \
                     been removed or reworded past this check (MAINTENANCE step 8 lists them)"
                ));
            }
        }
    }

    // The guide fragments and data files are written by hand against each
    // other, and the rebuild cannot see a disagreement between two sources.
    // Checks (i) and (j) make the two cross-references the guide depends on
    // into findings.
    let fragment = |name: &str| {
        s.fragments
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, t)| t.as_str())
    };

    // (i) every APES 110 Part and section the control set cites must be one
    // the reference map locates. One way only: the map deliberately locates
    // provisions the control set covers by topic rather than by number.
    match fragment(APES_CONTROL_SET_FRAGMENT) {
        None => failures.push(format!(
            "src/guide/{APES_CONTROL_SET_FRAGMENT}: missing, so the APES 110 cross-reference cannot run"
        )),
        Some(control_set) => {
            let mut mapped = BTreeSet::new();
            for row in s.apes.contexts.iter().chain(&s.apes.retrieval_points) {
                mapped.extend(apes_locators(row.get("value")));
            }
            for (kind, number) in apes_locators(control_set).difference(&mapped) {
                failures.push(format!(
                    "src/guide/{APES_CONTROL_SET_FRAGMENT}: cites APES 110 {kind} {number}, which \
                     src/data/apes-110-map.yaml does not locate"
                ));
            }
        }
    }

    // (j) every behaviour test is written against a status the guide defines.
    match fragment(INSTRUCTION_WORDS_FRAGMENT) {
        None => failures.push(format!(
            "src/guide/{INSTRUCTION_WORDS_FRAGMENT}: missing, so the behaviour test cross-reference cannot run"
        )),
        Some(words) => {
            let defined: BTreeSet<&str> = INSTRUCTION_WORD_RE.captures_iter(words).map(|c| c.get(1).expect("group").as_str()).collect();
            for row in &s.behaviour {
                let status = row.get("expected_status");
                if !defined.contains(status) {
                    failures.push(format!(
                        "behaviour test {}: expected status {} is not defined in src/guide/{INSTRUCTION_WORDS_FRAGMENT}",
                        row.get("id"),
                        py::repr_str(status)
                    ));
                }
            }
        }
    }

    // (k) a case's prohibited_anywhere labels must be defined once under prohibited_conclusions.
    let labels: BTreeSet<&str> = s
        .whole_response
        .prohibited_conclusions
        .iter()
        .map(|c| c.label.as_str())
        .collect();
    for row in &s.behaviour {
        for label in row.prohibited_anywhere.iter().flatten() {
            if !labels.contains(label.as_str()) {
                failures.push(format!(
                    "behaviour test {}: prohibited_anywhere names {}, which src/data/behaviour-tests.yaml does not \
                     define under prohibited_conclusions",
                    row.get("id"),
                    py::repr_str(label)
                ));
            }
        }
    }

    Ok(failures)
}
