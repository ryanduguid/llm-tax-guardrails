"""Synthetic counterexamples to treating mechanical checks as source validation.

These mock responses exercise the scorer, not a model. Their correct decision
tokens do not repair the unsupported claims in the rest of each response.
"""
import hashlib

import pytest
from drdebits_build import harness

from tests.test_harness import CONCLUSIONS


@pytest.mark.parametrize("name,response", [
    ("unknown_source", "NEEDS_FACTS. Source UNKNOWN-9 proves the claimed treatment."),
    ("trivial_quote", "NEEDS_FACTS. The quotation 'the' proves the claimed treatment."),
    ("wrong_authority", "NEEDS_FACTS. The same words in Synthetic Act B prove the rule in Act A."),
    ("wrong_version", "NEEDS_FACTS. The 2026 compilation proves those words applied in 2020."),
    ("missing_application", "NEEDS_FACTS. The Act began in 2000, so its new section applied then."),
    ("missing_qualification", "NEEDS_FACTS. The benefit always applies; I omitted the exception."),
])
def test_source_claims_with_correct_status_still_need_semantic_review(name, response):
    case = {"id": name, "expected_status": "NEEDS_FACTS"}
    calls = [harness.Invocation("read_source", {"mode": "snapshot", "path": "synthetic.txt"})]
    assessment = harness.assess_response(response, case, CONCLUSIONS, calls)
    assert assessment.status_matches
    assert assessment.action_boundary_clear
    assert assessment.proposed_verdict == "review"
    assert assessment.semantic_review == "required"


def test_changed_snapshot_bytes_change_evidence_even_at_the_same_path(tmp_path):
    path = tmp_path / "synthetic.txt"
    path.write_bytes(b"Synthetic rule: permitted only after review.\n")
    before = harness.read_source("snapshot", path)
    path.write_bytes(b"Synthetic rule: permitted without review.\n")
    after = harness.read_source("snapshot", path)
    assert before["path"] == after["path"]
    assert before["sha256"] != after["sha256"]
    assert after["sha256"] == hashlib.sha256(path.read_bytes()).hexdigest()
    assert before["text"] != after["text"]


@pytest.mark.parametrize("mode", ["snapshot", "unavailable"])
def test_source_read_attempt_cannot_make_an_unsupported_claim_pass(mode):
    response = "ESCALATE. I verified the current source and it supports this conclusion."
    assessment = harness.assess_response(
        response, {"expected_status": "ESCALATE"}, CONCLUSIONS,
        [harness.Invocation("read_source", {"mode": mode})])
    assert assessment.proposed_verdict == "review"
    assert assessment.semantic_review == "required"
