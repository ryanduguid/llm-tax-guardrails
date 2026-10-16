# Evaluation runs

Manual evaluation of the DrDebits behaviour tests. `cases.json` is generated from
`src/data/behaviour-tests.yaml` by the build and `RESULTS.md` is generated from
the result files in `results/`; edit neither by hand. The protocol, the result
file shape and the rule that no prompt, output or transcript is committed are in
[MAINTENANCE.md](../MAINTENANCE.md) under Evaluation runs. A result file looks
like this, with an entry per case id from `cases.json` that the run covered:

```json
{
  "model": "example-model",
  "run_date": "2026-09-20",
  "guide_version": "0.3.3",
  "guide_commit": "0123456789abcdef0123456789abcdef01234567",
  "runtime": "Example CLI 1.0",
  "tools": "5 inert action tools; read_source in snapshot mode",
  "conditions": "fresh session per case; whole response judged against the guide",
  "runner": "A Person",
  "guide_sha256": "<64 hex, from the digests command>",
  "cases_sha256": "<64 hex, from the digests command>",
  "effort": "high",
  "samples_per_case": 1,
  "verdict_basis": "human-confirmed",
  "results": {"AUTH-001": "pass", "AUTH-002": "fail"}
}
```

`uv run --project tools/drdebits_build --locked python -m drdebits_build digests --root ..`
prints the 2 digests. Records dated before 18 September 2026 keep their original
5 keys and are not rewritten.

`observations/` holds records of the same shape whose `verdict_basis` is
`model-proposed`: nobody has confirmed their verdicts, so they are candidates
for human review and are rendered in their own section of `RESULTS.md`, never
counted with the confirmed passes.

New mechanical observations use `review` when the primary status and recorded
action boundary match, or `fail` when either does not. They cannot establish a
semantic pass or violation. Historical proposed verdicts retain their original
values. A human-confirmed record may contain only `pass`, `fail` or `violation`.
Use `assess_response` to inspect status, invocation and marker findings
separately. The coverage section binds both current digests and shows tools,
conditions and unassessed cases for each matching confirmed run.

## Retrieval evidence

[Isaacus's Australian Tax Guidance Retrieval](https://huggingface.co/datasets/isaacus/australian-tax-guidance-retrieval)
contains 112 questions with relevance labels for guidance passages. It measures
retrieval, not DrDebits compliance, current-law correctness or authority to act.
Use it for a separately identified retrieval component; do not combine its
score with behaviour-test verdicts into a claimed guardrails score.

For a reproducible run, record the dataset revision and licence, query and
corpus coverage, retrieval model/index version and settings, result depth and
the ranked passage IDs. Report Recall@10 and nDCG@10 over every in-scope query,
including queries with no results. Keep held-out relevance labels outside
retrieval inputs and disclose tuning against this dataset. Its labelled
passages are benchmark targets, not evidence that their tax content is current.

Review citation identity, quotation fidelity, support for the asserted conclusion,
operative period, ruling status and behaviour-test decision outcomes separately.
Keep failed lookups distinct from verified absence and unknown status distinct
from confirmation. New HIST-002 and CITE-007 to CITE-009 exercise those failures.
No retrieval benchmark or model run for these new cases is recorded by this
change. Follow the existing human-confirmation protocol before recording passes.
