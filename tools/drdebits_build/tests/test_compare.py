"""Comparisons preserve provenance, partial coverage and unconfirmed outcomes."""
import json

import pytest
from drdebits_build import compare
from drdebits_build.build import build_digests, load_sources
from drdebits_build.model import ModelError

from tests.test_build import make_repo
from tests.test_evals import BOUND, PROPOSED, write_observation, write_result


def test_changes_keep_missing_cases_separate_from_regressions():
    before = {**BOUND, "results": {"fixed": "fail", "broken": "pass", "changed": "fail",
                                  "same": "violation", "old": "pass"}}
    after = {**BOUND, "results": {"fixed": "pass", "broken": "violation", "changed": "violation",
                                 "same": "violation", "new": "pass"}}
    out = compare.compare_records(before, after)
    assert out["comparison_supported"]
    assert out["shared_case_count"] == 4
    assert out["shared_pass_delta"] == 0
    assert {row["id"]: row["change"] for row in out["cases"]} == {
        "fixed": "fixed", "broken": "regressed", "changed": "changed",
        "same": "unchanged", "old": "baseline_only", "new": "candidate_only"}
    assert [row["id"] for row in out["cases"]] == sorted(before["results"].keys() | after["results"].keys())


@pytest.mark.parametrize("field,value", [
    ("cases_sha256", "c" * 64), ("runtime", "Another client"), ("tools", "disabled"),
    ("conditions", "different source snapshot"), ("effort", "low"), ("samples_per_case", 3),
])
def test_changed_conditions_do_not_claim_fixes(field, value):
    before = {**BOUND, "results": {"A-001": "fail"}}
    after = {**BOUND, field: value, "results": {"A-001": "pass"}}
    out = compare.compare_records(before, after)
    assert not out["comparison_supported"]
    assert out["limitations"]
    assert out["shared_pass_delta"] is None
    assert out["cases"][0]["change"] == "changed"
    assert out["metadata_changes"][field]["candidate"] == value


@pytest.mark.parametrize("candidate", [PROPOSED, BOUND])
def test_proposed_results_are_never_promoted(candidate):
    before = {**PROPOSED, "results": {"A-001": "review"}}
    out = compare.compare_records(before, candidate)
    assert not out["comparison_supported"]
    assert out["baseline_counts"] == {"review": 1}
    assert out["shared_pass_delta"] is None
    assert out["cases"][0]["change"] == "changed"


def test_guide_and_model_changes_are_visible_with_identical_test_conditions():
    after = {**BOUND, "model": "candidate model", "guide_sha256": "c" * 64}
    out = compare.compare_records(BOUND, after)
    assert out["comparison_supported"]
    assert set(out["metadata_changes"]) == {"model", "guide_sha256"}


def test_disjoint_subsets_have_no_pass_delta():
    after = {**BOUND, "results": {"A-002": "pass"}}
    out = compare.compare_records(BOUND, after)
    assert out["shared_case_count"] == 0
    assert not out["comparison_supported"]
    assert out["shared_pass_delta"] is None
    assert out["change_counts"] == {"baseline_only": 1, "candidate_only": 1}


def test_cli_reuses_validation_and_never_writes_records(tmp_path, capsys):
    root = make_repo(tmp_path)
    write_result(root, name="2026-09-18-baseline.json", **BOUND)
    write_observation(root, name="2026-09-18-candidate.json", results={"A-001": "review"})
    before = {path: path.read_bytes() for path in (root / "evals").rglob("*.json")}
    args = ["evals/results/2026-09-18-baseline.json", "evals/observations/2026-09-18-candidate.json",
            "--root", str(root)]
    assert compare.main(args) == 0
    out = json.loads(capsys.readouterr().out)
    assert out["matches_current_inputs"] == {"baseline": False, "candidate": False}
    assert not out["comparison_supported"]
    assert {path: path.read_bytes() for path in (root / "evals").rglob("*.json")} == before
    assert compare.main(args) == 0
    assert json.loads(capsys.readouterr().out) == out


def test_legacy_records_are_descriptive_only(tmp_path):
    root = make_repo(tmp_path)
    write_result(root)
    data = compare.load_record(root, load_sources(root), "evals/results/2026-02-01-example-model.json")
    out = compare.compare_records(data, BOUND)
    assert not out["comparison_supported"]
    assert "case digests are missing" in out["limitations"]


@pytest.mark.parametrize("content,match", [
    ('{"model":"one","model":"two"}', "duplicate key"),
    (json.dumps({**BOUND, "results": {"A-001": "pass", "UNKNOWN": "pass"}}), "unknown case"),
    (json.dumps({**BOUND, "results": {"A-001": "review"}}), "must be pass"),
    (json.dumps({**BOUND, "verdict_basis": "model-proposed"}), "verdict_basis"),
])
def test_invalid_records_use_the_existing_rules(tmp_path, content, match):
    root = make_repo(tmp_path)
    path = root / "evals/results/2026-09-18-baseline.json"
    path.parent.mkdir(parents=True)
    path.write_text(content, encoding="utf-8")
    with pytest.raises(ModelError, match=match):
        compare.load_record(root, load_sources(root), path)


def test_cli_refuses_outside_paths_and_invalid_dates(tmp_path, capsys):
    root = make_repo(tmp_path)
    args = [str(tmp_path.parent / "outside.json"), "unused", "--root", str(root)]
    assert compare.main(args) == 1
    assert "inside the repository" in capsys.readouterr().err
    write_result(root, name="2026-99-99-invalid.json", **BOUND)
    args[0] = "evals/results/2026-99-99-invalid.json"
    assert compare.main(args) == 1
    assert "compare:" in capsys.readouterr().err


@pytest.mark.parametrize("old,new,status", [
    ("pass", "pass", "passed"), ("fail", "pass", "passed"),
    ("violation", "pass", "passed"), ("fail", "fail", "passed"),
    ("violation", "violation", "passed"), ("violation", "fail", "passed"),
    ("pass", "fail", "failed"), ("pass", "violation", "failed"),
    ("fail", "violation", "failed"),
])
def test_check_keeps_individual_regressions_and_existing_nonpasses(tmp_path, old, new, status):
    sources = load_sources(make_repo(tmp_path))
    before = {**BOUND, **build_digests(sources), "results": {"A-001": old}}
    after = {**before, "results": {"A-001": new}}
    out = compare.check_regressions(compare.compare_records(before, after), after, sources)
    assert out["status"] == status
    assert out["regression_cases"] == (["A-001"] if status == "failed" else [])
    assert out["candidate_nonpasses"] == ([] if new == "pass" else ["A-001"])


def test_gains_cannot_cancel_a_regression(tmp_path):
    sources = load_sources(make_repo(tmp_path))
    sources.behaviour.append({**sources.behaviour[0], "id": "A-002"})
    before = {**BOUND, **build_digests(sources), "results": {"A-001": "pass", "A-002": "fail"}}
    after = {**before, "results": {"A-001": "fail", "A-002": "pass"}}
    comparison = compare.compare_records(before, after)
    assert comparison["shared_pass_delta"] == 0
    out = compare.check_regressions(comparison, after, sources)
    assert out["status"] == "failed"
    assert out["regression_cases"] == ["A-001"]


@pytest.mark.parametrize("field,value", [
    ("guide_sha256", "d" * 64), ("cases_sha256", "d" * 64),
    ("verdict_basis", "model-proposed"), ("conditions", "different conditions"),
])
def test_check_cannot_pass_with_unmatched_or_unconfirmed_evidence(tmp_path, field, value):
    sources = load_sources(make_repo(tmp_path))
    before = {**BOUND, **build_digests(sources)}
    after = {**before, field: value}
    out = compare.check_regressions(compare.compare_records(before, after), after, sources)
    assert out["status"] == "incomplete"
    assert out["limitations"]


def test_partial_comparison_cannot_pass_the_check(tmp_path):
    sources = load_sources(make_repo(tmp_path))
    sources.behaviour.append({**sources.behaviour[0], "id": "A-002"})
    before = {**BOUND, **build_digests(sources), "results": {"A-001": "pass", "A-002": "fail"}}
    after = {**before, "results": {"A-001": "pass"}}
    comparison = compare.compare_records(before, after)
    assert comparison["comparison_supported"]
    out = compare.check_regressions(comparison, after, sources)
    assert out["status"] == "incomplete"
    assert out["missing_current_cases"] == ["A-002"]
    assert out["regression_cases"] == []


@pytest.mark.parametrize("verdict,status", [("pass", "passed"), ("fail", "failed"),
                                           ("violation", "failed")])
def test_newly_assessed_cases_are_not_hidden_by_baseline_coverage(tmp_path, verdict, status):
    sources = load_sources(make_repo(tmp_path))
    sources.behaviour.append({**sources.behaviour[0], "id": "A-002"})
    before = {**BOUND, **build_digests(sources)}
    after = {**before, "results": {"A-001": "pass", "A-002": verdict}}
    out = compare.check_regressions(compare.compare_records(before, after), after, sources)
    assert out["status"] == status
    assert out["regression_cases"] == ([] if verdict == "pass" else ["A-002"])


@pytest.mark.parametrize("verdict,exit_code", [("pass", 0), ("fail", 1), ("violation", 1)])
def test_cli_check_returns_machine_readable_status(tmp_path, capsys, verdict, exit_code):
    root = make_repo(tmp_path)
    bound = {**BOUND, **build_digests(load_sources(root))}
    write_result(root, name="2026-09-18-baseline.json", **bound)
    write_result(root, name="2026-09-18-candidate.json", **{**bound, "results": {"A-001": verdict}})
    args = ["evals/results/2026-09-18-baseline.json", "evals/results/2026-09-18-candidate.json",
            "--root", str(root), "--check"]
    assert compare.main(args) == exit_code
    assert json.loads(capsys.readouterr().out)["regression_check"]["status"] == (
        "failed" if exit_code else "passed")
    args[0] = "evals/results/2026-09-18-missing.json"
    assert compare.main(args) == 2
    assert "compare:" in capsys.readouterr().err


def test_cli_check_legacy_records_are_incomplete(tmp_path, capsys):
    root = make_repo(tmp_path)
    write_result(root)
    record = "evals/results/2026-02-01-example-model.json"
    assert compare.main([record, record, "--root", str(root), "--check"]) == 2
    assert json.loads(capsys.readouterr().out)["regression_check"]["status"] == "incomplete"
