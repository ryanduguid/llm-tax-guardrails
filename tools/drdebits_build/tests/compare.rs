//! Comparisons preserve provenance, partial coverage and unconfirmed outcomes
//! (ported from test_compare.py).

mod common;

use common::*;
use drdebits::builder::{self, Sources};
use drdebits::compare::{check_regressions, compare_records, load_record};
use drdebits::error::Fail;
use drdebits::evals::Record;
use drdebits::pyjson::{self, Json};

fn record(json: Json) -> Record {
    let Json::Object(pairs) = json else {
        panic!("an object")
    };
    Record(pairs)
}

fn field<'a>(out: &'a [(String, Json)], key: &str) -> &'a Json {
    &out.iter().find(|(k, _)| k == key).unwrap().1
}

fn results(pairs: &[(&str, &str)]) -> Json {
    Json::Object(pairs.iter().map(|(k, v)| (k.to_string(), s(v))).collect())
}

fn with_digests(sources: &Sources, base: &Json) -> Json {
    let digests: Vec<(&str, Json)> = builder::build_digests(sources)
        .into_iter()
        .map(|(k, v)| (k, s(&v)))
        .collect();
    merged(base, &digests)
}

fn changes(out: &[(String, Json)]) -> Vec<(String, String)> {
    let Json::Array(cases) = field(out, "cases") else {
        panic!()
    };
    cases
        .iter()
        .map(|c| {
            (
                c.get("id").unwrap().as_str().unwrap().to_string(),
                c.get("change").unwrap().as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn changes_keep_missing_cases_separate_from_regressions() {
    let before = merged(
        &bound(),
        &[(
            "results",
            results(&[
                ("fixed", "fail"),
                ("broken", "pass"),
                ("changed", "fail"),
                ("same", "violation"),
                ("old", "pass"),
            ]),
        )],
    );
    let after = merged(
        &bound(),
        &[(
            "results",
            results(&[
                ("fixed", "pass"),
                ("broken", "violation"),
                ("changed", "violation"),
                ("same", "violation"),
                ("new", "pass"),
            ]),
        )],
    );
    let out = compare_records(&record(before), &record(after));
    assert_eq!(field(&out, "comparison_supported"), &Json::Bool(true));
    assert_eq!(field(&out, "shared_case_count"), &int(4));
    assert_eq!(field(&out, "shared_pass_delta"), &int(0));
    let mut expected = [
        ("broken", "regressed"),
        ("changed", "changed"),
        ("fixed", "fixed"),
        ("new", "candidate_only"),
        ("old", "baseline_only"),
        ("same", "unchanged"),
    ];
    expected.sort();
    assert_eq!(
        changes(&out),
        expected
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn changed_conditions_do_not_claim_fixes() {
    for (key, value) in [
        ("cases_sha256", s(&"c".repeat(64))),
        ("runtime", s("Another client")),
        ("tools", s("disabled")),
        ("conditions", s("different source snapshot")),
        ("effort", s("low")),
        ("samples_per_case", int(3)),
    ] {
        let before = merged(&bound(), &[("results", results(&[("A-001", "fail")]))]);
        let after = merged(
            &bound(),
            &[
                (key, value.clone()),
                ("results", results(&[("A-001", "pass")])),
            ],
        );
        let out = compare_records(&record(before), &record(after));
        assert_eq!(field(&out, "comparison_supported"), &Json::Bool(false));
        assert_ne!(field(&out, "limitations"), &Json::Array(vec![]));
        assert_eq!(field(&out, "shared_pass_delta"), &Json::Null);
        assert_eq!(changes(&out)[0].1, "changed");
        assert_eq!(
            field(&out, "metadata_changes")
                .get(key)
                .unwrap()
                .get("candidate"),
            Some(&value)
        );
    }
}

#[test]
fn proposed_results_are_never_promoted() {
    for candidate in [proposed(), bound()] {
        let before = merged(&proposed(), &[("results", results(&[("A-001", "review")]))]);
        let out = compare_records(&record(before), &record(candidate));
        assert_eq!(field(&out, "comparison_supported"), &Json::Bool(false));
        assert_eq!(field(&out, "baseline_counts"), &obj(&[("review", int(1))]));
        assert_eq!(field(&out, "shared_pass_delta"), &Json::Null);
        assert_eq!(changes(&out)[0].1, "changed");
    }
}

#[test]
fn guide_and_model_changes_are_visible_with_identical_test_conditions() {
    let after = merged(
        &bound(),
        &[
            ("model", s("candidate model")),
            ("guide_sha256", s(&"c".repeat(64))),
        ],
    );
    let out = compare_records(&record(bound()), &record(after));
    assert_eq!(field(&out, "comparison_supported"), &Json::Bool(true));
    let Json::Object(changes) = field(&out, "metadata_changes") else {
        panic!()
    };
    let mut keys: Vec<&str> = changes.iter().map(|(k, _)| k.as_str()).collect();
    keys.sort();
    assert_eq!(keys, ["guide_sha256", "model"]);
}

#[test]
fn disjoint_subsets_have_no_pass_delta() {
    let after = merged(&bound(), &[("results", results(&[("A-002", "pass")]))]);
    let out = compare_records(&record(bound()), &record(after));
    assert_eq!(field(&out, "shared_case_count"), &int(0));
    assert_eq!(field(&out, "comparison_supported"), &Json::Bool(false));
    assert_eq!(field(&out, "shared_pass_delta"), &Json::Null);
    assert_eq!(
        field(&out, "change_counts"),
        &obj(&[("baseline_only", int(1)), ("candidate_only", int(1))])
    );
}

#[test]
fn cli_reuses_validation_and_never_writes_records() {
    let (dir, _root) = fixture();
    write_result_from(dir.path(), "2026-09-18-baseline.json", &bound(), &[]);
    write_observation(
        dir.path(),
        "2026-09-18-candidate.json",
        &[("results", results(&[("A-001", "review")]))],
    );
    let before = std::fs::read(dir.path().join("evals/results/2026-09-18-baseline.json")).unwrap();
    let args = [
        "compare",
        "evals/results/2026-09-18-baseline.json",
        "evals/observations/2026-09-18-candidate.json",
    ];
    let first = run_in(dir.path(), &args, true);
    assert_eq!(first.code, 0, "{}", first.stderr);
    let out = pyjson::loads(&first.stdout).unwrap();
    assert_eq!(
        out.get("matches_current_inputs"),
        Some(&obj(&[
            ("baseline", Json::Bool(false)),
            ("candidate", Json::Bool(false))
        ]))
    );
    assert_eq!(out.get("comparison_supported"), Some(&Json::Bool(false)));
    assert_eq!(
        std::fs::read(dir.path().join("evals/results/2026-09-18-baseline.json")).unwrap(),
        before
    );
    assert_eq!(run_in(dir.path(), &args, true).stdout, first.stdout);
}

#[test]
fn legacy_records_are_descriptive_only() {
    let (dir, root) = fixture();
    write_result(dir.path(), "2026-02-01-example-model.json", &[]);
    let sources = builder::load_sources(&root).unwrap();
    let data = load_record(
        &root,
        &sources,
        "evals/results/2026-02-01-example-model.json",
    )
    .unwrap();
    let out = compare_records(&data, &record(bound()));
    assert_eq!(field(&out, "comparison_supported"), &Json::Bool(false));
    let Json::Array(limitations) = field(&out, "limitations") else {
        panic!()
    };
    assert!(limitations.contains(&s("case digests are missing")));
}

#[test]
fn invalid_records_use_the_existing_rules() {
    let cases = [
        (
            r#"{"model":"one","model":"two"}"#.to_string(),
            "duplicate key",
        ),
        (
            pyjson::dumps(
                &merged(
                    &bound(),
                    &[(
                        "results",
                        results(&[("A-001", "pass"), ("UNKNOWN", "pass")]),
                    )],
                ),
                false,
            ),
            "unknown case",
        ),
        (
            pyjson::dumps(
                &merged(&bound(), &[("results", results(&[("A-001", "review")]))]),
                false,
            ),
            "must be pass",
        ),
        (
            pyjson::dumps(
                &merged(&bound(), &[("verdict_basis", s("model-proposed"))]),
                false,
            ),
            "verdict_basis",
        ),
    ];
    for (content, needle) in cases {
        let (dir, root) = fixture();
        let path = dir.path().join("evals/results/2026-09-18-baseline.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        let sources = builder::load_sources(&root).unwrap();
        match load_record(&root, &sources, &path.to_string_lossy()) {
            Err(Fail::Model(m)) => assert!(m.contains(needle), "{m:?} lacks {needle:?}"),
            other => panic!("expected a ModelError, got {other:?}"),
        }
    }
}

#[test]
fn cli_refuses_outside_paths_and_invalid_dates() {
    let (dir, _root) = fixture();
    let outside = tempfile::tempdir().unwrap();
    let outside_path = outside.path().join("outside.json");
    let out = run_in(
        dir.path(),
        &["compare", &outside_path.to_string_lossy(), "unused"],
        true,
    );
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("inside the repository"));
    write_result_from(dir.path(), "2026-99-99-invalid.json", &bound(), &[]);
    let out = run_in(
        dir.path(),
        &["compare", "evals/results/2026-99-99-invalid.json", "unused"],
        true,
    );
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("compare:"));
}

fn fixture_sources(extra_case: bool) -> Sources {
    let (_dir, root) = fixture();
    let mut sources = builder::load_sources(&root).unwrap();
    if extra_case {
        let mut second = sources.behaviour[0].clone();
        second.row.set("id", "A-002");
        sources.behaviour.push(second);
    }
    sources
}

fn status(out: &Json) -> &str {
    out.get("status").unwrap().as_str().unwrap()
}

fn strings(out: &Json, key: &str) -> Vec<String> {
    let Some(Json::Array(items)) = out.get(key) else {
        panic!()
    };
    items
        .iter()
        .map(|i| i.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn check_keeps_individual_regressions_and_existing_nonpasses() {
    let sources = fixture_sources(false);
    for (old, new, expected) in [
        ("pass", "pass", "passed"),
        ("fail", "pass", "passed"),
        ("violation", "pass", "passed"),
        ("fail", "fail", "passed"),
        ("violation", "violation", "passed"),
        ("violation", "fail", "passed"),
        ("pass", "fail", "failed"),
        ("pass", "violation", "failed"),
        ("fail", "violation", "failed"),
    ] {
        let before = merged(
            &with_digests(&sources, &bound()),
            &[("results", results(&[("A-001", old)]))],
        );
        let after = merged(&before, &[("results", results(&[("A-001", new)]))]);
        let (before, after) = (record(before), record(after));
        let out = check_regressions(&compare_records(&before, &after), &after, &sources);
        assert_eq!(status(&out), expected, "{old} -> {new}");
        assert_eq!(
            strings(&out, "regression_cases"),
            if expected == "failed" {
                vec!["A-001".to_string()]
            } else {
                vec![]
            }
        );
        assert_eq!(
            strings(&out, "candidate_nonpasses"),
            if new == "pass" {
                vec![]
            } else {
                vec!["A-001".to_string()]
            }
        );
    }
}

#[test]
fn gains_cannot_cancel_a_regression() {
    let sources = fixture_sources(true);
    let before = merged(
        &with_digests(&sources, &bound()),
        &[("results", results(&[("A-001", "pass"), ("A-002", "fail")]))],
    );
    let after = merged(
        &before,
        &[("results", results(&[("A-001", "fail"), ("A-002", "pass")]))],
    );
    let (before, after) = (record(before), record(after));
    let comparison = compare_records(&before, &after);
    assert_eq!(field(&comparison, "shared_pass_delta"), &int(0));
    let out = check_regressions(&comparison, &after, &sources);
    assert_eq!(status(&out), "failed");
    assert_eq!(strings(&out, "regression_cases"), ["A-001"]);
}

#[test]
fn check_cannot_pass_with_unmatched_or_unconfirmed_evidence() {
    let sources = fixture_sources(false);
    for (key, value) in [
        ("guide_sha256", s(&"d".repeat(64))),
        ("cases_sha256", s(&"d".repeat(64))),
        ("verdict_basis", s("model-proposed")),
        ("conditions", s("different conditions")),
    ] {
        let before = with_digests(&sources, &bound());
        let after = merged(&before, &[(key, value)]);
        let (before, after) = (record(before), record(after));
        let out = check_regressions(&compare_records(&before, &after), &after, &sources);
        assert_eq!(status(&out), "incomplete", "{key}");
        assert!(!strings(&out, "limitations").is_empty());
    }
}

#[test]
fn partial_comparison_cannot_pass_the_check() {
    let sources = fixture_sources(true);
    let before = merged(
        &with_digests(&sources, &bound()),
        &[("results", results(&[("A-001", "pass"), ("A-002", "fail")]))],
    );
    let after = merged(&before, &[("results", results(&[("A-001", "pass")]))]);
    let (before, after) = (record(before), record(after));
    let comparison = compare_records(&before, &after);
    assert_eq!(
        field(&comparison, "comparison_supported"),
        &Json::Bool(true)
    );
    let out = check_regressions(&comparison, &after, &sources);
    assert_eq!(status(&out), "incomplete");
    assert_eq!(strings(&out, "missing_current_cases"), ["A-002"]);
    assert!(strings(&out, "regression_cases").is_empty());
}

#[test]
fn newly_assessed_cases_are_not_hidden_by_baseline_coverage() {
    let sources = fixture_sources(true);
    for (verdict, expected) in [
        ("pass", "passed"),
        ("fail", "failed"),
        ("violation", "failed"),
    ] {
        let before = with_digests(&sources, &bound());
        let after = merged(
            &before,
            &[("results", results(&[("A-001", "pass"), ("A-002", verdict)]))],
        );
        let (before, after) = (record(before), record(after));
        let out = check_regressions(&compare_records(&before, &after), &after, &sources);
        assert_eq!(status(&out), expected);
        assert_eq!(
            strings(&out, "regression_cases"),
            if verdict == "pass" {
                vec![]
            } else {
                vec!["A-002".to_string()]
            }
        );
    }
}

#[test]
fn cli_check_returns_machine_readable_status() {
    for (verdict, exit_code) in [("pass", 0), ("fail", 1), ("violation", 1)] {
        let (dir, root) = fixture();
        let sources = builder::load_sources(&root).unwrap();
        let base = with_digests(&sources, &bound());
        write_result_from(dir.path(), "2026-09-18-baseline.json", &base, &[]);
        write_result_from(
            dir.path(),
            "2026-09-18-candidate.json",
            &base,
            &[("results", results(&[("A-001", verdict)]))],
        );
        let mut args = vec![
            "compare",
            "evals/results/2026-09-18-baseline.json",
            "evals/results/2026-09-18-candidate.json",
            "--check",
        ];
        let out = run_in(dir.path(), &args, true);
        assert_eq!(out.code, exit_code, "{}", out.stderr);
        let parsed = pyjson::loads(&out.stdout).unwrap();
        assert_eq!(
            status(parsed.get("regression_check").unwrap()),
            if exit_code == 1 { "failed" } else { "passed" }
        );
        args[1] = "evals/results/2026-09-18-missing.json";
        let out = run_in(dir.path(), &args, true);
        assert_eq!(out.code, 2);
        assert!(out.stderr.contains("compare:"));
    }
}

#[test]
fn cli_check_legacy_records_are_incomplete() {
    let (dir, _root) = fixture();
    write_result(dir.path(), "2026-02-01-example-model.json", &[]);
    let rec = "evals/results/2026-02-01-example-model.json";
    let out = run_in(dir.path(), &["compare", rec, rec, "--check"], true);
    assert_eq!(out.code, 2);
    assert_eq!(
        status(
            pyjson::loads(&out.stdout)
                .unwrap()
                .get("regression_check")
                .unwrap()
        ),
        "incomplete"
    );
}

#[test]
fn check_flags_candidate_cases_outside_the_current_suite() {
    let sources = fixture_sources(false);
    let before = merged(
        &with_digests(&sources, &bound()),
        &[("results", results(&[("A-001", "pass")]))],
    );
    let after = merged(
        &before,
        &[("results", results(&[("A-001", "pass"), ("Z-9", "pass")]))],
    );
    let (before, after) = (record(before), record(after));
    let out = check_regressions(&compare_records(&before, &after), &after, &sources);
    assert_eq!(status(&out), "incomplete");
    assert!(
        strings(&out, "limitations")
            .contains(&"candidate has cases outside the current suite".to_string())
    );
}

#[test]
fn a_record_outside_the_result_directories_is_refused() {
    let (dir, root) = fixture();
    let sources = builder::load_sources(&root).unwrap();
    std::fs::write(dir.path().join("top.json"), pyjson::dumps(&bound(), false)).unwrap();
    match load_record(&root, &sources, "top.json") {
        Err(Fail::Model(m)) => assert_eq!(
            m,
            "comparison records must be files under evals/results or evals/observations"
        ),
        other => panic!("expected a ModelError, got {other:?}"),
    }
}

#[test]
fn a_comparison_without_a_limitations_list_adds_none() {
    // The port reads a missing list as empty; compare_records always writes one.
    let sources = fixture_sources(false);
    let before = merged(
        &with_digests(&sources, &bound()),
        &[("results", results(&[("A-001", "pass")]))],
    );
    let after = merged(&before, &[("results", results(&[("A-001", "fail")]))]);
    let (before, after) = (record(before), record(after));
    let with = |limitations: Json| -> Json {
        let mut comparison = compare_records(&before, &after);
        for (key, value) in comparison.iter_mut() {
            if key == "limitations" {
                *value = limitations.clone();
            }
        }
        check_regressions(&comparison, &after, &sources)
    };
    let out = with(Json::Null);
    assert_eq!(out, with(Json::Array(Vec::new())));
    assert_eq!(status(&out), "failed");
    assert!(strings(&out, "limitations").is_empty());
}

#[test]
fn a_record_path_through_a_missing_directory_follows_the_platform() {
    // POSIX cannot open a path through a missing directory, so Python's
    // is_file() refused it; Windows folds the `..` first and opens the record.
    let (dir, root) = fixture();
    write_result(dir.path(), "2026-02-01-example-model.json", &[]);
    let sources = builder::load_sources(&root).unwrap();
    let result = load_record(
        &root,
        &sources,
        "missing/../evals/results/2026-02-01-example-model.json",
    );
    if cfg!(windows) {
        assert!(result.is_ok(), "{result:?}");
    } else {
        match result {
            Err(Fail::Model(m)) => assert_eq!(
                m,
                "comparison records must be files under evals/results or evals/observations"
            ),
            other => panic!("expected a ModelError, got {other:?}"),
        }
    }
}

#[test]
fn a_source_value_error_is_invalid_evidence_as_python_caught_it() {
    // Python's handler caught ValueError with ModelError, BuildError and
    // OSError, exiting 2 under --check and 1 without it. KeyError escaped as
    // a traceback, which exits 1 either way.
    let rec = "evals/results/2026-02-01-example-model.json";
    for (value, check_code, message) in [
        ("!!int x", 2, "invalid literal for int() with base 10: 'x'"),
        ("!!float x", 2, "could not convert string to float: 'x'"),
        ("!!bool x", 1, "unexpected error: KeyError: 'x'"),
    ] {
        let (dir, _root) = fixture();
        write_result(dir.path(), "2026-02-01-example-model.json", &[]);
        let meta = dir.path().join("src/data/metadata.yaml");
        let text = std::fs::read_to_string(&meta).unwrap();
        std::fs::write(&meta, format!("{text}bogus: {value}\n")).unwrap();
        let shown = format!("compare: {message}\n");
        for (args, code) in [
            (vec!["compare", rec, rec, "--check"], check_code),
            (vec!["compare", rec, rec], 1),
        ] {
            let out = run_in(dir.path(), &args, true);
            assert_eq!((out.code, &out.stderr), (code, &shown), "{value} {args:?}");
            assert_eq!(out.stdout, "", "{value} {args:?}");
        }
    }
}
