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
  "run_date": "2026-01-31",
  "guide_version": "0.3.3",
  "runner": "A Person",
  "results": {"AUTH-001": "pass", "AUTH-002": "fail"}
}
```
