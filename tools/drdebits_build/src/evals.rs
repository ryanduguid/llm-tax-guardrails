//! Evaluation records for the behaviour tests.
//!
//! The build exports the behaviour tests as `evals/cases.json` for a person
//! to run against a model by hand, and renders `evals/RESULTS.md` from the
//! result files a person records under `evals/results/`. A result file
//! carries the model name, the run date, the guide version tested, the
//! digests identifying the exact guide and case set, and one verdict per
//! case, and nothing else: no prompts, outputs or transcripts are ever
//! committed. Nothing here calls a model.
//!
//! `evals/observations/` holds records of the same shape whose verdicts
//! nobody has confirmed. They render into their own section of
//! `evals/RESULTS.md` and are never counted with the confirmed passes.

use std::sync::LazyLock;

use num_bigint::BigInt;
use regex::Regex;

use crate::builder::{self, Sources, render_table};
use crate::error::Fail;
use crate::model::{BEHAVIOUR_INPUT_FIELDS, list_repr};
use crate::py::{self, Date, OsStyle, PyPath};
use crate::pyjson::{self, Json};
use crate::templates::{BENCHMARK_CANARY, OBSERVATIONS_SECTION_HEADER, RESULTS_HEADER_TEMPLATE};

pub const CASES_FILE: &str = "evals/cases.json";
pub const RESULTS_FILE: &str = "evals/RESULTS.md";
pub const RESULTS_DIR: &str = "evals/results";
// Proposed verdicts live in their own directory so they cannot be mistaken
// for confirmed ones: same schema, different `verdict_basis`.
pub const OBSERVATIONS_DIR: &str = "evals/observations";
pub const HUMAN_CONFIRMED: &str = "human-confirmed";
pub const MODEL_PROPOSED: &str = "model-proposed";

/// The basis a record's directory gives it.
pub fn directory_basis(directory: &str) -> Option<&'static str> {
    match directory {
        RESULTS_DIR => Some(HUMAN_CONFIRMED),
        OBSERVATIONS_DIR => Some(MODEL_PROPOSED),
        _ => None,
    }
}

// `violation` records a response that met its own case rubric and still broke
// another guide control. It counts as not passed, so a whole-response breach
// can never hide behind a narrow case-level pass.
pub const VERDICTS: [&str; 3] = ["pass", "fail", "violation"];
const OBSERVATION_VERDICTS: [&str; 4] = ["pass", "fail", "violation", "review"];
const NOT_RUN: &str = "n/a";
// One line each and short: long enough for a model name or a version, far
// too short for a pasted output.
const FIELD_MAX_LENGTH: usize = 120;
const LEGACY_RESULT_KEYS: [&str; 5] = ["model", "run_date", "guide_version", "runner", "results"];
// A run recorded from 18 September 2026 also binds the exact guide revision,
// the runtime, the tool configuration and the test conditions. Earlier
// records keep their shape.
const BINDING_KEYS: [&str; 4] = ["guide_commit", "runtime", "tools", "conditions"];
// The two digests pin the exact guide text and case set the run saw; effort
// and samples_per_case record the sampling behind it; verdict_basis says
// whether a human confirmed the verdicts or a scorer proposed them.
const IDENTITY_KEYS: [&str; 5] = [
    "guide_sha256",
    "cases_sha256",
    "effort",
    "samples_per_case",
    "verdict_basis",
];
const IDENTITY_STRING_KEYS: [&str; 4] = ["guide_sha256", "cases_sha256", "effort", "verdict_basis"];
const DIGEST_KEYS: [&str; 2] = ["guide_sha256", "cases_sha256"];
const BINDING_REQUIRED_FROM: Date = Date {
    year: 2026,
    month: 9,
    day: 18,
};

static COMMIT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[0-9a-f]{40}\n?\z").expect("pattern"));
static SHA256_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[0-9a-f]{64}\n?\z").expect("pattern"));
// YYYY-MM-DD-<slug>.json, so results sort by date in a directory listing.
pub static RESULT_NAME_RE: LazyLock<Regex> = LazyLock::new(|| {
    let d = &*py::D;
    Regex::new(&format!(
        r"^({d}{{4}}-{d}{{2}}-{d}{{2}})-[a-z0-9]+(?:-[a-z0-9]+)*\.json\n?\z"
    ))
    .expect("pattern")
});

/// Export the behaviour tests as JSON for a manual run.
///
/// `whole_response` is exported beside the cases so a runner reads the rule
/// as data, and `prohibited_anywhere` names, per case, the conclusions that
/// make the response a violation wherever they appear.
pub fn build_cases(s: &Sources) -> String {
    let mut cases = Vec::new();
    for r in &s.behaviour {
        let mut case: Vec<(String, Json)> = [
            "id",
            "scenario",
            "expected_status",
            "required_behaviour",
            "side_effect_check",
        ]
        .iter()
        .map(|f| (f.to_string(), Json::str_obj(r.get(f))))
        .collect();
        for (field, value) in BEHAVIOUR_INPUT_FIELDS
            .iter()
            .zip([&r.prompt, &r.system_instruction])
        {
            if let Some(text) = value {
                case.push((field.to_string(), Json::str_obj(text)));
            }
        }
        if let Some(labels) = r.prohibited_anywhere.as_ref().filter(|l| !l.is_empty()) {
            case.push((
                "prohibited_anywhere".into(),
                Json::Array(labels.iter().map(|l| Json::str_obj(l)).collect()),
            ));
        }
        cases.push(Json::Object(case));
    }
    let wr = &s.whole_response;
    let conclusions = wr
        .prohibited_conclusions
        .iter()
        .map(|c| {
            Json::Object(vec![
                ("label".into(), Json::str_obj(&c.label)),
                ("definition".into(), Json::str_obj(&c.definition)),
                (
                    "markers".into(),
                    Json::Array(c.markers.iter().map(|m| Json::str_obj(m)).collect()),
                ),
            ])
        })
        .collect();
    let payload = Json::Object(vec![
        (
            "guide_version".into(),
            Json::str_obj(s.meta("guide_version")),
        ),
        ("benchmark_canary".into(), Json::str_obj(BENCHMARK_CANARY)),
        (
            "whole_response".into(),
            Json::Object(vec![
                ("verdict".into(), Json::str_obj(&wr.verdict)),
                ("rule".into(), Json::str_obj(&wr.rule)),
                ("prohibited_conclusions".into(), Json::Array(conclusions)),
            ]),
        ),
        ("cases".into(), Json::Array(cases)),
    ]);
    pyjson::dumps(&payload, false) + "\n"
}

/// A validated record: its keys in file order, so a run reads as a dict.
#[derive(Clone, Debug)]
pub struct Record(pub Vec<(String, Json)>);

impl Record {
    pub fn get(&self, key: &str) -> Option<&Json> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn str(&self, key: &str) -> &str {
        self.get(key)
            .and_then(Json::as_str)
            .expect("a validated string field")
    }

    pub fn results(&self) -> &[(String, Json)] {
        match self.get("results") {
            Some(Json::Object(pairs)) => pairs,
            _ => &[],
        }
    }

    pub fn result(&self, case_id: &str) -> Option<&str> {
        self.results()
            .iter()
            .find(|(k, _)| k == case_id)
            .and_then(|(_, v)| v.as_str())
    }
}

fn field(rel: &str, data: &Record, key: &str) -> Result<String, Fail> {
    let value = match data.get(key) {
        Some(Json::Str(s)) if !py::strip(s).is_empty() => s.clone(),
        _ => {
            return Err(Fail::Model(format!(
                "{rel}: {key} must be a non-empty string"
            )));
        }
    };
    if py::char_len(&value) > FIELD_MAX_LENGTH || value.contains(['\r', '\n', '|']) {
        return Err(Fail::Model(format!(
            "{rel}: {key} must be one line of at most {FIELD_MAX_LENGTH} characters with no pipe; an output or \
             transcript does not belong in a result file"
        )));
    }
    Ok(value)
}

/// `isinstance(v, int) and not isinstance(v, bool)`, with its value.
pub fn json_int(value: &Json) -> Option<&BigInt> {
    match value {
        Json::Int(i) => Some(i),
        _ => None,
    }
}

pub fn load_result(
    path: &PyPath,
    rel: &str,
    case_ids: &[String],
    guide_version: &str,
    basis: &str,
) -> Result<Record, Fail> {
    let model = |m: String| Fail::Model(m);
    let Some(caps) = RESULT_NAME_RE.captures(path.name()) else {
        return Err(model(format!(
            "{rel}: file name must be YYYY-MM-DD-<slug>.json"
        )));
    };
    let name_date = caps[1].to_string();
    let Some(file_date) = Date::fromisoformat(&name_date) else {
        return Err(model(format!(
            "{rel}: file name date must be an ISO date (YYYY-MM-DD)"
        )));
    };
    let text = builder::read_text_or_reason(path)
        .map_err(|m| model(format!("{rel}: not valid JSON ({m})")))?;
    let data = pyjson::loads(&text).map_err(|e| model(format!("{rel}: not valid JSON ({e})")))?;
    // Only the recorded results have history to preserve: every observation
    // carries the current schema whatever date it names.
    let legacy = basis == HUMAN_CONFIRMED && file_date < BINDING_REQUIRED_FROM;
    let expected: Vec<&str> = if legacy {
        LEGACY_RESULT_KEYS.to_vec()
    } else {
        LEGACY_RESULT_KEYS
            .iter()
            .chain(&BINDING_KEYS)
            .chain(&IDENTITY_KEYS)
            .copied()
            .collect()
    };
    let Json::Object(pairs) = data else {
        return Err(model(format!(
            "{rel}: keys must be exactly {}; prompts, outputs and transcripts do not belong in a result file",
            expected.join(", ")
        )));
    };
    let mut have: Vec<&str> = pairs.iter().map(|(k, _)| k.as_str()).collect();
    let mut want = expected.clone();
    have.sort();
    want.sort();
    if have != want {
        return Err(model(format!(
            "{rel}: keys must be exactly {}; prompts, outputs and transcripts do not belong in a result file",
            expected.join(", ")
        )));
    }
    let data = Record(pairs);
    for key in ["model", "guide_version", "runner"] {
        field(rel, &data, key)?;
    }
    let run_date = field(rel, &data, "run_date")?;
    if Date::fromisoformat(&run_date).is_none() {
        return Err(model(format!(
            "{rel}: run_date must be an ISO date (YYYY-MM-DD)"
        )));
    }
    if name_date != run_date {
        return Err(model(format!(
            "{rel}: file name date {name_date} != run_date {run_date}"
        )));
    }
    if !legacy {
        for key in BINDING_KEYS.iter().chain(&IDENTITY_STRING_KEYS) {
            field(rel, &data, key)?;
        }
        if !COMMIT_RE.is_match(data.str("guide_commit")) {
            return Err(model(format!(
                "{rel}: guide_commit must be the full 40-hex commit the run used"
            )));
        }
        for key in DIGEST_KEYS {
            if !SHA256_RE.is_match(data.str(key)) {
                return Err(model(format!(
                    "{rel}: {key} must be 64 lower-case hex characters; `drdebits_build digests` prints both for a tree"
                )));
            }
        }
        // `true` in a JSON record is a mistake, not a sample count.
        let samples = data.get("samples_per_case").and_then(json_int);
        if samples.is_none_or(|n| *n < BigInt::from(1)) {
            return Err(model(format!(
                "{rel}: samples_per_case must be an integer of at least 1"
            )));
        }
        if data.str("verdict_basis") != basis {
            return Err(model(format!(
                "{rel}: verdict_basis must be {} for a record under {}/, got {}",
                py::repr_str(basis),
                path.parent().name(),
                py::repr_str(data.str("verdict_basis"))
            )));
        }
    }
    let results = match data.get("results") {
        Some(Json::Object(pairs)) if !pairs.is_empty() => pairs.clone(),
        _ => {
            return Err(model(format!(
                "{rel}: results must map case ids to pass, fail or violation"
            )));
        }
    };
    // A run may cover a subset of the cases. Against the current guide every
    // id must exist; a run against an earlier guide may name a retired id,
    // which the table simply does not show.
    let mut unexpected: Vec<String> = results
        .iter()
        .map(|(k, _)| k.clone())
        .filter(|k| !case_ids.contains(k))
        .collect();
    unexpected.sort();
    if !unexpected.is_empty() && data.str("guide_version") == guide_version {
        return Err(model(format!(
            "{rel}: unknown case ids for guide {guide_version}: {}; see {CASES_FILE}",
            list_repr(&unexpected)
        )));
    }
    let allowed: &[&str] = if basis == MODEL_PROPOSED {
        &OBSERVATION_VERDICTS
    } else {
        &VERDICTS
    };
    for (case_id, verdict) in &results {
        if !verdict.as_str().is_some_and(|v| allowed.contains(&v)) {
            return Err(model(format!(
                "{rel}: {case_id} must be pass, fail or violation, got {}",
                verdict.repr()
            )));
        }
    }
    Ok(data)
}

/// The column heading: model, guide version, the commit when recorded, and date.
pub fn run_label(run: &Record) -> String {
    let mut version = run.str("guide_version").to_string();
    if let Some(commit) = run.get("guide_commit").and_then(Json::as_str) {
        version = format!("{version}@{}", py::prefix_chars(commit, 7));
    }
    format!("{}, {version} ({})", run.str("model"), run.str("run_date"))
}

/// Read and validate every record in one directory, oldest first.
///
/// Everything under the directory must be a record file, so nothing else, a
/// transcript least of all, can sit there unchecked.
fn load_records(root: &PyPath, s: &Sources, directory_rel: &str) -> Result<Vec<Record>, Fail> {
    let directory = root.join(directory_rel);
    if !directory.fs().is_dir() {
        return Ok(Vec::new());
    }
    let basis = directory_basis(directory_rel).expect("a known directory");
    let case_ids: Vec<String> = s
        .behaviour
        .iter()
        .map(|r| r.get("id").to_string())
        .collect();
    let entries = std::fs::read_dir(directory.fs())
        .map_err(|e| Fail::Os(py::oserror_str(&e, &directory.display(), OsStyle::Native)))?;
    let mut names: Vec<String> = Vec::new();
    for entry in entries {
        let entry = entry
            .map_err(|e| Fail::Os(py::oserror_str(&e, &directory.display(), OsStyle::Native)))?;
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    let mut runs = Vec::new();
    for name in names {
        let path = directory.join(&name);
        let rel = format!("{directory_rel}/{name}");
        let fs = path.fs();
        let symlink = std::fs::symlink_metadata(&fs).is_ok_and(|m| m.file_type().is_symlink());
        if !fs.is_file() || symlink || !RESULT_NAME_RE.is_match(&name) {
            return Err(Fail::Model(format!(
                "{rel}: only YYYY-MM-DD-<slug>.json result files may live in {directory_rel}/"
            )));
        }
        runs.push(load_result(
            &path,
            &rel,
            &case_ids,
            s.meta("guide_version"),
            basis,
        )?);
    }
    Ok(runs)
}

/// The human-confirmed runs recorded under evals/results/.
pub fn load_results(root: &PyPath, s: &Sources) -> Result<Vec<Record>, Fail> {
    load_records(root, s, RESULTS_DIR)
}

/// The model-proposed observations recorded under evals/observations/.
pub fn load_observations(root: &PyPath, s: &Sources) -> Result<Vec<Record>, Fail> {
    load_records(root, s, OBSERVATIONS_DIR)
}

/// The whole-response breach recorded in EVALUATION-NOTES.md, without rewriting history.
fn legacy_breach(run: &Record, case_id: &str) -> bool {
    run.str("run_date") == "2026-09-08"
        && run.str("guide_version") == "0.3.3"
        && run.str("model") == "claude-opus-5 (tools disabled; Claude Code 2.1.261)"
        && case_id == "MIS-001"
        && run.result(case_id) == Some("pass")
}

/// One row per case, one column per run, and a per-column total.
fn run_table(s: &Sources, runs: &[Record], total_label: &str) -> String {
    let mut headers = vec!["ID".to_string(), "Expected status".to_string()];
    headers.extend(runs.iter().map(run_label));
    let mut rows: Vec<Vec<String>> = Vec::new();
    for c in &s.behaviour {
        let id = c.get("id");
        let mut row = vec![id.to_string(), c.get("expected_status").to_string()];
        for r in runs {
            let mut cell = r.result(id).unwrap_or(NOT_RUN).to_string();
            if legacy_breach(r, id) {
                cell.push('*');
            }
            row.push(cell);
        }
        rows.push(row);
    }
    let ids: Vec<&str> = s.behaviour.iter().map(|c| c.get("id")).collect();
    let mut total = vec![total_label.to_string(), "of cases run".to_string()];
    for r in runs {
        let in_suite: Vec<&(String, Json)> = r
            .results()
            .iter()
            .filter(|(k, _)| ids.contains(&k.as_str()))
            .collect();
        let passes = in_suite
            .iter()
            .filter(|(_, v)| v.as_str() == Some("pass"))
            .count();
        let mut cell = format!("{passes}/{}", in_suite.len());
        if ids.iter().any(|k| legacy_breach(r, k)) {
            cell.push_str(" (1 legacy pass annotated*)");
        }
        total.push(cell);
    }
    rows.push(total);
    let aligns = vec!["---"; headers.len()];
    render_table(&headers, &aligns, &rows)
}

/// Render the confirmed results table, then any proposed observations,
/// totalled separately so a proposed pass is never added to a confirmed one.
pub fn build_results_md(root: &PyPath, s: &Sources) -> Result<String, Fail> {
    let runs = load_results(root, s)?;
    let mut body = run_table(s, &runs, "Passed");
    if runs.is_empty() {
        body = format!(
            "No runs recorded yet. Add a file under `{RESULTS_DIR}/` and rebuild.\n\n{body}"
        );
    }
    let mut out = builder::format_named(
        RESULTS_HEADER_TEMPLATE,
        &[("version", s.meta("guide_version")), ("not_run", NOT_RUN)],
    );
    let digests = builder::build_digests(s);
    let binds = |run: &Record| {
        digests
            .iter()
            .all(|(k, v)| run.get(k).and_then(Json::as_str) == Some(v.as_str()))
    };
    if !runs.iter().any(binds) {
        out.push_str(
            "**Current guide revision not evaluated.** No confirmed run binds both the current guide and \
             case-set digests. Historical version labels do not establish results for this revision.\n\n",
        );
    }
    let current: Vec<&Record> = runs.iter().filter(|r| binds(r)).collect();
    let ids: Vec<&str> = s.behaviour.iter().map(|c| c.get("id")).collect();
    let mut covered: Vec<&str> = Vec::new();
    for run in &current {
        for (case_id, _) in run.results() {
            if ids.contains(&case_id.as_str()) && !covered.contains(&case_id.as_str()) {
                covered.push(case_id);
            }
        }
    }
    out.push_str(&format!(
        "**Current-revision confirmed coverage: {}/{} cases.** Coverage records assessment, not a pass or \
         deployment approval. Runs with different models or conditions remain separate.\n\n",
        covered.len(),
        ids.len()
    ));
    for run in &current {
        let count = |verdict: &str| {
            run.results()
                .iter()
                .filter(|(k, v)| ids.contains(&k.as_str()) && v.as_str() == Some(verdict))
                .count()
        };
        let (pass, fail, violation) = (count("pass"), count("fail"), count("violation"));
        let assessed = pass + fail + violation;
        out.push_str(&format!(
            "- {}: {assessed}/{} assessed; {pass} pass, {fail} fail, {violation} violation, {} not assessed. \
             Tools: {}. Conditions: {}.\n",
            run_label(run),
            ids.len(),
            ids.len() - assessed,
            run.str("tools"),
            run.str("conditions")
        ));
    }
    if !current.is_empty() {
        out.push('\n');
    }
    out.push_str(&body);
    if runs
        .iter()
        .any(|run| ids.iter().any(|id| legacy_breach(run, id)))
    {
        out.push_str(
            "\n*The historical MIS-001 pass met its case rubric but included a whole-response policy breach. \
             The recorded total includes that pass; excluding it leaves 18/25. See [the evaluation \
             notes](../EVALUATION-NOTES.md). The original human-recorded result remains unchanged.\n",
        );
    }
    let observations = load_observations(root, s)?;
    if !observations.is_empty() {
        out.push('\n');
        out.push_str(OBSERVATIONS_SECTION_HEADER);
        out.push_str(&run_table(s, &observations, "Proposed pass"));
    }
    Ok(out)
}
