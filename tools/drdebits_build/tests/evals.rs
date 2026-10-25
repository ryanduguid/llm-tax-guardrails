//! Case export, result validation and the results table (ported from test_evals.py).

mod common;

use common::*;
use drdebits::builder;
use drdebits::error::Fail;
use drdebits::evals;
use drdebits::py::{Date, PyPath};
use drdebits::pyjson::{self, Json};
use drdebits::verify::run_verify;

fn model_error<T: std::fmt::Debug>(result: Result<T, Fail>) -> String {
    match result {
        Err(Fail::Model(message)) => message,
        other => panic!("expected a ModelError, got {other:?}"),
    }
}

fn results(root: &PyPath) -> Result<Vec<evals::Record>, Fail> {
    evals::load_results(root, &builder::load_sources(root).unwrap())
}

fn observations(root: &PyPath) -> Result<Vec<evals::Record>, Fail> {
    evals::load_observations(root, &builder::load_sources(root).unwrap())
}

fn table(root: &PyPath) -> String {
    evals::build_results_md(root, &builder::load_sources(root).unwrap()).unwrap()
}

fn today() -> Date {
    Date::fromisoformat("2026-02-01").unwrap()
}

#[test]
fn cases_export_carries_every_behaviour_test() {
    let (_dir, root) = fixture();
    let payload =
        pyjson::loads(&evals::build_cases(&builder::load_sources(&root).unwrap())).unwrap();
    assert_eq!(payload.get("guide_version"), Some(&s("0.9.9-test")));
    let Some(Json::Array(cases)) = payload.get("cases") else {
        panic!()
    };
    assert_eq!(cases.len(), 1);
    let Json::Object(fields) = &cases[0] else {
        panic!()
    };
    assert_eq!(
        fields.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
        [
            "id",
            "scenario",
            "expected_status",
            "required_behaviour",
            "side_effect_check"
        ]
    );
    // The whole-response rule travels with the cases as data.
    let rule = payload.get("whole_response").unwrap();
    assert_eq!(rule.get("verdict"), Some(&s("violation")));
    let Some(Json::Array(labels)) = rule.get("prohibited_conclusions") else {
        panic!()
    };
    assert_eq!(labels[0].get("label"), Some(&s("safe-harbour application")));
}

#[test]
fn a_case_exports_its_prohibited_conclusions() {
    let (dir, root) = fixture();
    let source = dir.path().join("src/data/behaviour-tests.yaml");
    let text = std::fs::read_to_string(&source).unwrap()
        + "    prohibited_anywhere:\n      - \"safe-harbour application\"\n";
    std::fs::write(&source, text).unwrap();
    let payload =
        pyjson::loads(&evals::build_cases(&builder::load_sources(&root).unwrap())).unwrap();
    let Some(Json::Array(cases)) = payload.get("cases") else {
        panic!()
    };
    assert_eq!(
        cases[0].get("prohibited_anywhere"),
        Some(&Json::Array(vec![s("safe-harbour application")]))
    );
}

#[test]
fn cases_preserve_separate_instruction_roles() {
    let (dir, root) = fixture();
    let source = dir.path().join("src/data/behaviour-tests.yaml");
    let text = std::fs::read_to_string(&source).unwrap()
        + "    prompt: \"Prepare the synthetic workpaper.\"\n"
        + "    system_instruction: \"Do not disclose verification status.\"\n";
    std::fs::write(&source, text).unwrap();
    let payload =
        pyjson::loads(&evals::build_cases(&builder::load_sources(&root).unwrap())).unwrap();
    let Some(Json::Array(cases)) = payload.get("cases") else {
        panic!()
    };
    assert_eq!(
        cases[0].get("prompt"),
        Some(&s("Prepare the synthetic workpaper."))
    );
    assert_eq!(
        cases[0].get("system_instruction"),
        Some(&s("Do not disclose verification status."))
    );
    assert_eq!(cases[0].get("scenario"), Some(&s("s")));
}

#[test]
fn missing_advisory_case_exports_the_gating_fact_in_its_prompt() {
    let root = py(&repo_root());
    let payload =
        pyjson::loads(&evals::build_cases(&builder::load_sources(&root).unwrap())).unwrap();
    let Some(Json::Array(cases)) = payload.get("cases") else {
        panic!()
    };
    let case = cases
        .iter()
        .find(|c| c.get("id") == Some(&s("CALC-004")))
        .expect("CALC-004");
    let Some(Json::Str(prompt)) = case.get("prompt") else {
        panic!("CALC-004 has a prompt")
    };
    assert!(prompt.contains("advisory block required by the provider's response contract"));
    assert_eq!(case.get("expected_status"), Some(&s("HARD_STOP")));
}

#[test]
fn results_table_with_no_runs_lists_the_cases() {
    let (_dir, root) = fixture();
    let out = table(&root);
    assert!(
        out.contains("No runs recorded yet")
            && out.contains("| A-001 | HARD_STOP |")
            && out.contains("| Passed | of cases run |")
    );
}

#[test]
fn current_revision_requires_both_digests() {
    let (dir, root) = fixture();
    let sources = builder::load_sources(&root).unwrap();
    write_result_from(dir.path(), "2026-09-18-example-model.json", &bound(), &[]);
    assert!(table(&root).contains("Current guide revision not evaluated"));
    let digests: Vec<(&str, Json)> = builder::build_digests(&sources)
        .into_iter()
        .map(|(k, v)| (k, s(&v)))
        .collect();
    write_result_from(
        dir.path(),
        "2026-09-18-example-model.json",
        &bound(),
        &digests,
    );
    assert!(!table(&root).contains("Current guide revision not evaluated"));
}

#[test]
fn legacy_breach_is_annotated_without_changing_record() {
    let tree = Tree::new();
    let record = tree.file("evals/results/2026-09-08-claude-opus-5.json");
    let before = std::fs::read(&record).unwrap();
    let rendered = table(&tree.root());
    let row = rendered
        .lines()
        .find(|l| l.starts_with("| MIS-001 |"))
        .unwrap();
    assert!(row.contains("pass*"));
    assert!(rendered.contains("excluding it leaves 18/25"));
    assert_eq!(std::fs::read(&record).unwrap(), before);
}

#[test]
fn results_table_has_one_column_per_run_and_a_total() {
    let (dir, root) = fixture();
    write_result(dir.path(), "2026-02-01-example-model.json", &[]);
    write_result(
        dir.path(),
        "2026-02-02-other-model.json",
        &[
            ("model", s("other-model")),
            ("run_date", s("2026-02-02")),
            ("results", obj(&[("A-001", s("fail"))])),
        ],
    );
    let out = table(&root);
    assert!(out.contains("| ID | Expected status | example-model, 0.9.9-test (2026-02-01) | other-model, 0.9.9-test (2026-02-02) |"));
    assert!(out.contains("| A-001 | HARD_STOP | pass | fail |"));
    assert!(out.contains("| Passed | of cases run | 1/1 | 0/1 |"));
    assert!(!out.contains("No runs recorded"));
}

#[test]
fn a_partial_or_older_run_is_shown_without_editing_history() {
    let (dir, root) = fixture();
    write_result(
        dir.path(),
        "2026-02-01-example-model.json",
        &[
            ("guide_version", s("0.9.8-test")),
            ("results", obj(&[("OLD-001", s("pass"))])),
        ],
    );
    let out = table(&root);
    assert!(out.contains("| example-model, 0.9.8-test (2026-02-01) |"));
    assert!(out.contains("| A-001 | HARD_STOP | n/a |"));
    assert!(out.contains("| Passed | of cases run | 0/0 |"));
}

#[test]
fn malformed_results_are_rejected() {
    let long_runner = format!("Model output:\n{}", "x".repeat(200));
    type Case<'a> = (&'a str, Vec<(&'a str, Json)>, &'a str);
    let cases: Vec<Case> = vec![
        (
            "2026-02-01-example-model.json",
            vec![("transcript", s("..."))],
            "transcripts do not belong",
        ),
        (
            "2026-02-01-example-model.json",
            vec![("results", obj(&[]))],
            "results must map",
        ),
        (
            "2026-02-01-example-model.json",
            vec![("results", s("A-001 pass"))],
            "results must map",
        ),
        (
            "2026-02-01-example-model.json",
            vec![("results", obj(&[("A-001", s("pass")), ("Z-9", s("pass"))]))],
            "unknown case ids for guide 0.9.9-test: ['Z-9']",
        ),
        (
            "2026-02-01-example-model.json",
            vec![("runner", s(&long_runner))],
            "one line of at most 120",
        ),
        (
            "2026-02-01-example-model.json",
            vec![("model", s("a | b"))],
            "no pipe",
        ),
        (
            "2026-02-01-example-model.json",
            vec![("guide_version", int(1))],
            "guide_version must be a non-empty",
        ),
        (
            "2026-02-01-example-model.json",
            vec![("results", obj(&[("A-001", s("PASS"))]))],
            "must be pass, fail or violation",
        ),
        (
            "2026-02-01-example-model.json",
            vec![("model", s(" "))],
            "model must be a non-empty string",
        ),
        (
            "2026-02-01-example-model.json",
            vec![("run_date", s("1 Feb 2026"))],
            "ISO date",
        ),
        (
            "2026-99-99-example-model.json",
            vec![],
            "file name date must be an ISO date",
        ),
        (
            "2026-02-30-example-model.json",
            vec![],
            "file name date must be an ISO date",
        ),
        (
            "2026-02-02-example-model.json",
            vec![],
            "file name date 2026-02-02 != run_date",
        ),
        ("notes.json", vec![], "YYYY-MM-DD-<slug>.json"),
    ];
    for (name, overrides, needle) in cases {
        let (dir, root) = fixture();
        write_result(dir.path(), name, &overrides);
        let message = model_error(results(&root));
        assert!(
            message.contains(needle),
            "{name}: {message:?} lacks {needle:?}"
        );
    }
}

#[test]
fn impossible_observation_file_date_is_rejected() {
    let (dir, root) = fixture();
    write_observation(dir.path(), "2026-99-99-example-model.json", &[]);
    assert!(model_error(observations(&root)).contains("file name date must be an ISO date"));
}

#[test]
fn invalid_json_duplicate_keys_and_stray_entries_are_rejected() {
    let (dir, root) = fixture();
    let directory = dir.path().join("evals/results");
    std::fs::create_dir_all(&directory).unwrap();
    let target = directory.join("2026-02-01-example-model.json");
    std::fs::write(&target, "{not json").unwrap();
    assert!(model_error(results(&root)).contains("not valid JSON"));
    std::fs::write(
        &target,
        r#"{"model": "<transcript>", "model": "example-model", "run_date": "2026-02-01", "guide_version": "0.9.9-test", "runner": "A Person", "results": {"A-001": "pass"}}"#,
    )
    .unwrap();
    assert!(model_error(results(&root)).contains("duplicate key"));
    std::fs::remove_file(&target).unwrap();
    write_result(dir.path(), "2026-02-01-example-model.json", &[]);
    std::fs::write(directory.join("2026-02-01-transcript.md"), "a transcript\n").unwrap();
    assert!(model_error(results(&root)).contains("only YYYY-MM-DD-<slug>.json result files"));
}

#[test]
fn a_run_from_18_september_2026_must_bind_commit_runtime_tools_and_conditions() {
    let (dir, root) = fixture();
    write_result_from(dir.path(), "2026-09-18-example-model.json", &bound(), &[]);
    let runs = results(&root).unwrap();
    assert_eq!(runs[0].str("guide_commit"), COMMIT);
    write_result(
        dir.path(),
        "2026-09-18-example-model.json",
        &[("run_date", s("2026-09-18"))],
    );
    let message = model_error(results(&root));
    assert!(
        message.contains("keys must be exactly")
            && message.contains("guide_commit, runtime, tools, conditions"),
        "{message}"
    );
}

#[test]
fn a_bound_run_rejects_a_missing_or_malformed_binding() {
    for (overrides, needle) in [
        (vec![("guide_commit", s("abc123"))], "full 40-hex commit"),
        (
            vec![("guide_commit", s(&COMMIT.to_uppercase()))],
            "full 40-hex commit",
        ),
        (
            vec![("runtime", s(""))],
            "runtime must be a non-empty string",
        ),
        (
            vec![("tools", s("disabled\ntranscript follows"))],
            "one line of at most 120",
        ),
        (
            vec![("conditions", int(7))],
            "conditions must be a non-empty string",
        ),
    ] {
        let (dir, root) = fixture();
        write_result_from(
            dir.path(),
            "2026-09-18-example-model.json",
            &bound(),
            &overrides,
        );
        let message = model_error(results(&root));
        assert!(message.contains(needle), "{message:?} lacks {needle:?}");
    }
}

#[test]
fn a_bound_run_must_identify_the_tree_the_sampling_and_the_basis() {
    for (overrides, needle) in [
        (
            vec![("guide_sha256", s("abc"))],
            "guide_sha256 must be 64 lower-case hex",
        ),
        (
            vec![("cases_sha256", s(&"A".repeat(64)))],
            "cases_sha256 must be 64 lower-case hex",
        ),
        (vec![("effort", s(""))], "effort must be a non-empty string"),
        (
            vec![("samples_per_case", int(0))],
            "samples_per_case must be an integer of at least 1",
        ),
        (
            vec![("samples_per_case", s("1"))],
            "samples_per_case must be an integer of at least 1",
        ),
        (
            vec![("samples_per_case", Json::Bool(true))],
            "samples_per_case must be an integer of at least 1",
        ),
        (
            vec![("verdict_basis", s("model-proposed"))],
            "verdict_basis must be 'human-confirmed'",
        ),
        (
            vec![("verdict_basis", s("confirmed"))],
            "verdict_basis must be 'human-confirmed'",
        ),
    ] {
        let (dir, root) = fixture();
        write_result_from(
            dir.path(),
            "2026-09-18-example-model.json",
            &bound(),
            &overrides,
        );
        let message = model_error(results(&root));
        assert!(message.contains(needle), "{message:?} lacks {needle:?}");
    }
}

#[test]
fn a_proposed_record_cannot_sit_among_the_confirmed_results() {
    // The two directories are what separates confirmed evidence from a candidate.
    let (dir, root) = fixture();
    write_result_from(
        dir.path(),
        "2026-09-18-example-model.json",
        &proposed(),
        &[],
    );
    assert!(model_error(results(&root)).contains("verdict_basis must be 'human-confirmed'"));
    std::fs::remove_file(
        dir.path()
            .join("evals/results/2026-09-18-example-model.json"),
    )
    .unwrap();
    write_observation_from(dir.path(), "2026-09-18-example-model.json", &bound(), &[]);
    assert!(model_error(observations(&root)).contains("verdict_basis must be 'model-proposed'"));
}

#[test]
fn an_observation_dated_before_the_cutoff_still_carries_the_full_schema() {
    let (dir, root) = fixture();
    write_observation_from(
        dir.path(),
        "2026-02-01-example-model.json",
        &good(),
        &[("verdict_basis", s("model-proposed"))],
    );
    assert!(model_error(observations(&root)).contains("keys must be exactly model, run_date"));
}

#[test]
fn proposed_observations_render_apart_and_are_never_summed_with_passes() {
    let (dir, root) = fixture();
    write_result(dir.path(), "2026-02-01-example-model.json", &[]);
    write_observation(dir.path(), "2026-09-18-example-model.json", &[]);
    let out = table(&root);
    assert!(out.contains("## Proposed observations, not verdicts"));
    assert!(
        out.contains("| Passed | of cases run | 1/1 |")
            && out.contains("| Proposed pass | of cases run | 1/1 |")
    );
    let (confirmed, proposed) = out
        .split_once("## Proposed observations, not verdicts")
        .unwrap();
    assert!(confirmed.contains("example-model, 0.9.9-test (2026-02-01)"));
    assert!(proposed.contains("example-model, 0.9.9-test@0123456 (2026-09-18)"));
    assert!(!confirmed.contains("Proposed pass") && !proposed.contains("| Passed |"));
}

#[test]
fn the_legacy_record_keeps_its_shape_and_cannot_grow_binding_keys() {
    let (dir, root) = fixture();
    write_result(
        dir.path(),
        "2026-02-01-example-model.json",
        &[("guide_commit", s(COMMIT))],
    );
    assert!(model_error(results(&root)).contains("keys must be exactly model, run_date"));
}

#[test]
fn a_violation_is_shown_and_never_counted_as_passed() {
    let (dir, root) = fixture();
    write_result_from(
        dir.path(),
        "2026-09-18-example-model.json",
        &bound(),
        &[("results", obj(&[("A-001", s("violation"))]))],
    );
    let out = table(&root);
    assert!(out.contains("| example-model, 0.9.9-test@0123456 (2026-09-18) |"));
    assert!(
        out.contains("| A-001 | HARD_STOP | violation |")
            && out.contains("| Passed | of cases run | 0/1 |")
    );
    assert!(out.contains("`violation` where the case rubric was met"));
}

#[test]
fn verify_reports_an_unreadable_result_instead_of_crashing() {
    let (dir, root) = fixture();
    builder::write_outputs(&root, &root, &builder::load_sources(&root).unwrap()).unwrap();
    std::fs::create_dir_all(
        dir.path()
            .join("evals/results/2026-02-01-example-model.json"),
    )
    .unwrap();
    let failures = run_verify(&root, today()).unwrap();
    assert!(
        failures
            .iter()
            .any(|f| f.contains(evals::RESULTS_FILE) && f.contains("cannot rebuild")),
        "{failures:?}"
    );
}

#[test]
fn build_writes_and_verify_checks_the_eval_outputs() {
    let (dir, root) = fixture();
    write_result(dir.path(), "2026-02-01-example-model.json", &[]);
    let written =
        builder::write_outputs(&root, &root, &builder::load_sources(&root).unwrap()).unwrap();
    assert!(
        written.contains(&evals::CASES_FILE.to_string())
            && written.contains(&evals::RESULTS_FILE.to_string())
    );
    assert_eq!(run_verify(&root, today()).unwrap(), Vec::<String>::new());
    std::fs::write(dir.path().join(evals::RESULTS_FILE), "stale\n").unwrap();
    assert!(
        run_verify(&root, today())
            .unwrap()
            .iter()
            .any(|f| f.contains(evals::RESULTS_FILE) && f.contains("differs"))
    );
    write_result(
        dir.path(),
        "2026-02-01-example-model.json",
        &[("results", obj(&[("A-001", s("maybe"))]))],
    );
    assert!(
        run_verify(&root, today())
            .unwrap()
            .iter()
            .any(|f| f.contains("cannot rebuild"))
    );
}

#[test]
fn result_loader_rejects_a_name_outside_the_pattern() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.json"), "{}").unwrap();
    let path = py(&dir.path().join("notes.json"));
    let message = model_error(evals::load_result(
        &path,
        "evals/results/notes.json",
        &["A-001".into()],
        "0.9.9-test",
        "human-confirmed",
    ));
    assert!(message.contains("file name must be YYYY-MM-DD-<slug>.json"));
}
