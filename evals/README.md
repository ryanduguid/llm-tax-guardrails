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
