"""Compare recorded evaluations without assigning or promoting verdicts.

Only matched case digests, recorded conditions and human-confirmed verdicts
support fixed/regressed labels. Other comparisons retain the recorded changes
and explain why those labels are unavailable. Nothing calls a model.
"""
from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from pathlib import Path

from . import evals
from .build import BuildError, build_digests, find_root, load_sources
from .model import ModelError

CONDITION_KEYS = ("runtime", "tools", "conditions", "effort", "samples_per_case")


def load_record(root, sources, name):
    """Reuse the result validator for one file in either evaluation directory."""
    root = Path(root).resolve()
    path = Path(name)
    if not path.is_absolute():
        path = root / path
    try:
        relative = path.resolve().relative_to(root)
    except ValueError as exc:
        raise ModelError("comparison records must be inside the repository") from exc
    directory = relative.parent.as_posix()
    if directory not in evals.DIRECTORY_BASIS or path.is_symlink() or not path.is_file():
        raise ModelError("comparison records must be files under evals/results or evals/observations")
    data = evals._load_result(
        path, relative.as_posix(), [case["id"] for case in sources.behaviour],
        sources.meta["guide_version"], evals.DIRECTORY_BASIS[directory])
    # Directory provenance supplies the basis for legacy records in memory only.
    return {"verdict_basis": evals.DIRECTORY_BASIS[directory], **data}


def compare_records(baseline, candidate):
    """Compare validated records; equal digests are recorded identity, not attestation."""
    reasons = []
    before, after = baseline["results"], candidate["results"]
    shared = before.keys() & after.keys()
    if not shared:
        reasons.append("no shared cases were assessed")
    if not baseline.get("cases_sha256") or not candidate.get("cases_sha256"):
        reasons.append("case digests are missing")
    elif baseline["cases_sha256"] != candidate["cases_sha256"]:
        reasons.append("case digests differ; matching ids may describe different tests")
    for key in CONDITION_KEYS:
        if key not in baseline or key not in candidate:
            reasons.append(f"{key} is missing")
        elif baseline[key] != candidate[key]:
            reasons.append(f"{key} differs")
    if any(run.get("verdict_basis") != evals.HUMAN_CONFIRMED for run in (baseline, candidate)):
        reasons.append("both records must be human-confirmed for fixed/regressed labels")
    comparable = not reasons
    cases = []
    for case_id in sorted(before.keys() | after.keys()):
        old, new = before.get(case_id), after.get(case_id)
        if old is None:
            change = "candidate_only"
        elif new is None:
            change = "baseline_only"
        elif old == new:
            change = "unchanged"
        elif comparable and old != "pass" and new == "pass":
            change = "fixed"
        elif comparable and old == "pass" and new != "pass":
            change = "regressed"
        else:
            change = "changed"
        cases.append({"id": case_id, "baseline": old, "candidate": new, "change": change})
    metadata = sorted((baseline.keys() | candidate.keys()) - {"results"})
    return {
        "comparison_supported": comparable,
        "limitations": reasons,
        "metadata_changes": {key: {"baseline": baseline.get(key), "candidate": candidate.get(key)}
                             for key in metadata if baseline.get(key) != candidate.get(key)},
        "baseline_counts": dict(sorted(Counter(before.values()).items())),
        "candidate_counts": dict(sorted(Counter(after.values()).items())),
        "shared_case_count": len(shared),
        "shared_pass_delta": (sum(after[key] == "pass" for key in shared)
                              - sum(before[key] == "pass" for key in shared)) if comparable else None,
        "change_counts": dict(sorted(Counter(case["change"] for case in cases).items())),
        "cases": cases,
    }


def check_regressions(comparison, candidate, sources):
    """Check recorded regressions against complete coverage of the current inputs.

    Existing nonpasses remain visible. A pass means no new recorded regression,
    not that the guide or model is suitable for professional use.
    """
    reasons = list(comparison["limitations"])
    digests = build_digests(sources)
    if any(candidate.get(key) != digest for key, digest in digests.items()):
        reasons.append("candidate does not match the current guide and case digests")
    expected = {case["id"] for case in sources.behaviour}
    missing = sorted(expected - candidate["results"].keys())
    unexpected = sorted(candidate["results"].keys() - expected)
    if missing:
        reasons.append("candidate has unassessed current cases")
    if unexpected:
        reasons.append("candidate has cases outside the current suite")
    if comparison["change_counts"].get("baseline_only"):
        reasons.append("candidate omits cases assessed by the baseline")
    failures = []
    if comparison["comparison_supported"]:
        for case in comparison["cases"]:
            old, new = case["baseline"], case["candidate"]
            if (new in ("fail", "violation") and old in (None, "pass")
                    or new == "violation" and old != "violation"):
                failures.append(case["id"])
    return {
        "status": "incomplete" if reasons else "failed" if failures else "passed",
        "limitations": reasons,
        "regression_cases": failures,
        "missing_current_cases": missing,
        "unexpected_cases": unexpected,
        "candidate_nonpasses": sorted(key for key, value in candidate["results"].items()
                                      if value != "pass"),
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", help="record path relative to --root, or an absolute path")
    parser.add_argument("candidate", help="record path relative to --root, or an absolute path")
    parser.add_argument("--root", default=None)
    parser.add_argument("--check", action="store_true",
                        help="exit 0 for no recorded regression, 1 for regression, "
                             "2 for incomplete or invalid evidence; requires full current coverage")
    args = parser.parse_args(argv)
    try:
        root = Path(args.root) if args.root else find_root(Path.cwd())
        sources = load_sources(root)
        before = load_record(root, sources, args.baseline)
        after = load_record(root, sources, args.candidate)
        result = compare_records(before, after)
        digests = build_digests(sources)
        result["matches_current_inputs"] = {
            label: all(run.get(key) == digest for key, digest in digests.items())
            for label, run in (("baseline", before), ("candidate", after))}
        if args.check:
            result["regression_check"] = check_regressions(result, after, sources)
    except (ModelError, BuildError, OSError, ValueError) as exc:
        print(f"compare: {exc}", file=sys.stderr)
        return 2 if args.check else 1
    print(json.dumps(result, indent=2, ensure_ascii=False, sort_keys=True))
    if args.check:
        return {"passed": 0, "failed": 1, "incomplete": 2}[result["regression_check"]["status"]]
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
