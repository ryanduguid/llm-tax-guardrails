//! Compare recorded evaluations without assigning or promoting verdicts.
//!
//! Only matched case digests, recorded conditions and human-confirmed
//! verdicts support fixed/regressed labels. Other comparisons retain the
//! recorded changes and explain why those labels are unavailable. Nothing
//! calls a model.

use std::collections::BTreeMap;
use std::path::{Component, PathBuf};

use crate::builder::{self, Sources};
use crate::error::Fail;
use crate::evals::{self, Record};
use crate::py::{self, PyPath};
use crate::pyjson::{self, Json};

const CONDITION_KEYS: [&str; 5] = [
    "runtime",
    "tools",
    "conditions",
    "effort",
    "samples_per_case",
];

#[derive(clap::Args)]
pub struct Args {
    /// Record path relative to --root, or an absolute path.
    baseline: String,
    /// Record path relative to --root, or an absolute path.
    candidate: String,
    /// Exit 0 for no recorded regression, 1 for regression, 2 for incomplete
    /// or invalid evidence; requires full current coverage.
    #[arg(long)]
    check: bool,
}

#[cfg(windows)]
fn same_component(a: &std::ffi::OsStr, b: &std::ffi::OsStr) -> bool {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

#[cfg(not(windows))]
fn same_component(a: &std::ffi::OsStr, b: &std::ffi::OsStr) -> bool {
    a == b
}

/// `path.relative_to(root)` as POSIX text, or None when `path` is not under `root`.
fn relative_to(path: &std::path::Path, root: &std::path::Path) -> Option<Vec<String>> {
    let path: Vec<Component> = path.components().collect();
    let root: Vec<Component> = root.components().collect();
    if path.len() < root.len()
        || !root
            .iter()
            .zip(&path)
            .all(|(a, b)| same_component(a.as_os_str(), b.as_os_str()))
    {
        return None;
    }
    Some(
        path[root.len()..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect(),
    )
}

/// Reuse the result validator for one file in either evaluation directory.
pub fn load_record(root: &PyPath, sources: &Sources, name: &str) -> Result<Record, Fail> {
    let resolved_root = py::resolve(&root.fs());
    let resolved_root_py = PyPath::new(&resolved_root.to_string_lossy());
    let given = PyPath::new(name);
    let path = if given.is_absolute() {
        given
    } else {
        resolved_root_py.join(name)
    };
    let Some(relative) = relative_to(&py::resolve(&path.fs()), &resolved_root) else {
        return Err(Fail::Model(
            "comparison records must be inside the repository".into(),
        ));
    };
    let directory = if relative.len() > 1 {
        relative[..relative.len() - 1].join("/")
    } else {
        ".".into()
    };
    let fs: PathBuf = path.fs();
    let symlink = std::fs::symlink_metadata(&fs).is_ok_and(|m| m.file_type().is_symlink());
    let basis = evals::directory_basis(&directory);
    let Some(basis) = basis.filter(|_| !symlink && fs.is_file()) else {
        return Err(Fail::Model(
            "comparison records must be files under evals/results or evals/observations".into(),
        ));
    };
    let case_ids: Vec<String> = sources
        .behaviour
        .iter()
        .map(|c| c.get("id").to_string())
        .collect();
    let data = evals::load_result(
        &path,
        &relative.join("/"),
        &case_ids,
        sources.meta("guide_version"),
        basis,
    )?;
    // Directory provenance supplies the basis for legacy records in memory only.
    let mut pairs = vec![("verdict_basis".to_string(), Json::str_obj(basis))];
    for (k, v) in data.0 {
        match pairs.iter_mut().find(|(existing, _)| *existing == k) {
            Some(slot) => slot.1 = v,
            None => pairs.push((k, v)),
        }
    }
    Ok(Record(pairs))
}

fn counts<'a>(values: impl Iterator<Item = &'a str>) -> Json {
    let mut tally: BTreeMap<&str, i64> = BTreeMap::new();
    for v in values {
        *tally.entry(v).or_default() += 1;
    }
    Json::Object(
        tally
            .into_iter()
            .map(|(k, n)| (k.to_string(), Json::Int(n.into())))
            .collect(),
    )
}

fn opt_str(value: Option<&str>) -> Json {
    value.map_or(Json::Null, Json::str_obj)
}

/// Compare validated records; equal digests are recorded identity, not attestation.
pub fn compare_records(baseline: &Record, candidate: &Record) -> Vec<(String, Json)> {
    let mut reasons: Vec<String> = Vec::new();
    let before = baseline.results();
    let after = candidate.results();
    let has = |set: &[(String, Json)], k: &str| set.iter().any(|(key, _)| key == k);
    let shared: Vec<&str> = before
        .iter()
        .map(|(k, _)| k.as_str())
        .filter(|k| has(after, k))
        .collect();
    if shared.is_empty() {
        reasons.push("no shared cases were assessed".into());
    }
    let digest = |r: &Record| r.get("cases_sha256").filter(|v| v.truthy()).cloned();
    match (digest(baseline), digest(candidate)) {
        (Some(a), Some(b)) => {
            if a != b {
                reasons
                    .push("case digests differ; matching ids may describe different tests".into());
            }
        }
        _ => reasons.push("case digests are missing".into()),
    }
    for key in CONDITION_KEYS {
        match (baseline.get(key), candidate.get(key)) {
            (Some(a), Some(b)) => {
                if a != b {
                    reasons.push(format!("{key} differs"));
                }
            }
            _ => reasons.push(format!("{key} is missing")),
        }
    }
    if [baseline, candidate]
        .iter()
        .any(|r| r.get("verdict_basis").and_then(Json::as_str) != Some(evals::HUMAN_CONFIRMED))
    {
        reasons.push("both records must be human-confirmed for fixed/regressed labels".into());
    }
    let comparable = reasons.is_empty();
    let mut ids: Vec<&str> = before
        .iter()
        .chain(after)
        .map(|(k, _)| k.as_str())
        .collect();
    ids.sort();
    ids.dedup();
    let mut cases = Vec::new();
    let mut changes = Vec::new();
    for case_id in ids {
        let old = baseline.result(case_id);
        let new = candidate.result(case_id);
        let change = match (old, new) {
            (None, _) => "candidate_only",
            (_, None) => "baseline_only",
            (Some(o), Some(n)) if o == n => "unchanged",
            (Some(o), Some(n)) if comparable && o != "pass" && n == "pass" => "fixed",
            (Some(o), Some(n)) if comparable && o == "pass" && n != "pass" => "regressed",
            _ => "changed",
        };
        changes.push(change);
        cases.push(Json::Object(vec![
            ("id".into(), Json::str_obj(case_id)),
            ("baseline".into(), opt_str(old)),
            ("candidate".into(), opt_str(new)),
            ("change".into(), Json::str_obj(change)),
        ]));
    }
    let mut metadata: Vec<&str> = baseline
        .0
        .iter()
        .chain(&candidate.0)
        .map(|(k, _)| k.as_str())
        .filter(|k| *k != "results")
        .collect();
    metadata.sort();
    metadata.dedup();
    let mut metadata_changes = Vec::new();
    for key in metadata {
        let (a, b) = (baseline.get(key), candidate.get(key));
        if a != b {
            metadata_changes.push((
                key.to_string(),
                Json::Object(vec![
                    ("baseline".into(), a.cloned().unwrap_or(Json::Null)),
                    ("candidate".into(), b.cloned().unwrap_or(Json::Null)),
                ]),
            ));
        }
    }
    let shared_pass_delta = if comparable {
        let passes = |r: &Record| {
            shared
                .iter()
                .filter(|k| r.result(k) == Some("pass"))
                .count() as i64
        };
        Json::Int((passes(candidate) - passes(baseline)).into())
    } else {
        Json::Null
    };
    vec![
        ("comparison_supported".into(), Json::Bool(comparable)),
        (
            "limitations".into(),
            Json::Array(reasons.into_iter().map(Json::Str).collect()),
        ),
        ("metadata_changes".into(), Json::Object(metadata_changes)),
        (
            "baseline_counts".into(),
            counts(before.iter().filter_map(|(_, v)| v.as_str())),
        ),
        (
            "candidate_counts".into(),
            counts(after.iter().filter_map(|(_, v)| v.as_str())),
        ),
        (
            "shared_case_count".into(),
            Json::Int((shared.len() as i64).into()),
        ),
        ("shared_pass_delta".into(), shared_pass_delta),
        ("change_counts".into(), counts(changes.iter().copied())),
        ("cases".into(), Json::Array(cases)),
    ]
}

/// Check recorded regressions against complete coverage of the current inputs.
///
/// Existing nonpasses remain visible. A pass means no new recorded
/// regression, not that the guide or model is suitable for professional use.
pub fn check_regressions(
    comparison: &[(String, Json)],
    candidate: &Record,
    sources: &Sources,
) -> Json {
    let get = |k: &str| {
        comparison
            .iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v)
            .expect("a comparison field")
    };
    let mut reasons: Vec<Json> = match get("limitations") {
        Json::Array(items) => items.clone(),
        _ => Vec::new(),
    };
    let digests = builder::build_digests(sources);
    if digests
        .iter()
        .any(|(k, v)| candidate.get(k).and_then(Json::as_str) != Some(v.as_str()))
    {
        reasons.push(Json::str_obj(
            "candidate does not match the current guide and case digests",
        ));
    }
    let expected: Vec<&str> = sources.behaviour.iter().map(|c| c.get("id")).collect();
    let present: Vec<&str> = candidate
        .results()
        .iter()
        .map(|(k, _)| k.as_str())
        .collect();
    let mut missing: Vec<&str> = expected
        .iter()
        .copied()
        .filter(|k| !present.contains(k))
        .collect();
    missing.sort();
    let mut unexpected: Vec<&str> = present
        .iter()
        .copied()
        .filter(|k| !expected.contains(k))
        .collect();
    unexpected.sort();
    if !missing.is_empty() {
        reasons.push(Json::str_obj("candidate has unassessed current cases"));
    }
    if !unexpected.is_empty() {
        reasons.push(Json::str_obj(
            "candidate has cases outside the current suite",
        ));
    }
    if get("change_counts")
        .get("baseline_only")
        .is_some_and(Json::truthy)
    {
        reasons.push(Json::str_obj(
            "candidate omits cases assessed by the baseline",
        ));
    }
    let mut failures = Vec::new();
    if get("comparison_supported") == &Json::Bool(true)
        && let Json::Array(cases) = get("cases")
    {
        for case in cases {
            let old = case.get("baseline").and_then(Json::as_str);
            let new = case.get("candidate").and_then(Json::as_str);
            let new_failure =
                matches!(new, Some("fail" | "violation")) && matches!(old, None | Some("pass"));
            if new_failure || new == Some("violation") && old != Some("violation") {
                failures.push(case.get("id").cloned().expect("an id"));
            }
        }
    }
    let status = if !reasons.is_empty() {
        "incomplete"
    } else if !failures.is_empty() {
        "failed"
    } else {
        "passed"
    };
    let mut nonpasses: Vec<&str> = candidate
        .results()
        .iter()
        .filter(|(_, v)| v.as_str() != Some("pass"))
        .map(|(k, _)| k.as_str())
        .collect();
    nonpasses.sort();
    let strs = |items: Vec<&str>| Json::Array(items.into_iter().map(Json::str_obj).collect());
    Json::Object(vec![
        ("status".into(), Json::str_obj(status)),
        ("limitations".into(), Json::Array(reasons)),
        ("regression_cases".into(), Json::Array(failures)),
        ("missing_current_cases".into(), strs(missing)),
        ("unexpected_cases".into(), strs(unexpected)),
        ("candidate_nonpasses".into(), strs(nonpasses)),
    ])
}

pub fn main(root: &Option<String>, args: Args) -> u8 {
    let result = (|| -> Result<Json, Fail> {
        let root = crate::root_for(root)?;
        let sources = builder::load_sources(&root)?;
        let before = load_record(&root, &sources, &args.baseline)?;
        let after = load_record(&root, &sources, &args.candidate)?;
        let mut result = compare_records(&before, &after);
        let digests = builder::build_digests(&sources);
        let matches = |run: &Record| {
            Json::Bool(
                digests
                    .iter()
                    .all(|(k, v)| run.get(k).and_then(Json::as_str) == Some(v.as_str())),
            )
        };
        result.push((
            "matches_current_inputs".into(),
            Json::Object(vec![
                ("baseline".into(), matches(&before)),
                ("candidate".into(), matches(&after)),
            ]),
        ));
        if args.check {
            let check = check_regressions(&result, &after, &sources);
            result.push(("regression_check".into(), check));
        }
        Ok(Json::Object(result))
    })();
    match result {
        Ok(result) => {
            outln!("{}", pyjson::dumps(&result, true));
            if !args.check {
                return 0;
            }
            match result
                .get("regression_check")
                .and_then(|c| c.get("status"))
                .and_then(Json::as_str)
            {
                Some("passed") => 0,
                Some("failed") => 1,
                _ => 2,
            }
        }
        Err(
            e @ (Fail::Model(_) | Fail::Build(_) | Fail::Os(_) | Fail::Decode(_) | Fail::Bundle(_)),
        ) => {
            errln!("compare: {e}");
            if args.check { 2 } else { 1 }
        }
        // Python's `except ValueError` also caught one escaping a reader,
        // such as `int()` refusing `!!int x` in a source.
        Err(Fail::Internal(m)) if m.starts_with("ValueError: ") => {
            errln!("compare: {}", &m["ValueError: ".len()..]);
            if args.check { 2 } else { 1 }
        }
        Err(e) => {
            errln!("compare: {e}");
            1
        }
    }
}
