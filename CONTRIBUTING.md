# Contributing

Send changes as pull requests against `main`, one purpose per pull request, and
complete the pull request template. Report security vulnerabilities privately as
[SECURITY.md](SECURITY.md) describes, never in an issue or pull request. Keep
client data, credentials and proprietary prompt content out of the repository,
commit messages and pull requests; reproduce a problem with fabricated facts.

## Where to make a change

The guide, the reference files, the behaviour tests, `evals/cases.json` and
`evals/RESULTS.md` are generated. Edit their sources under `src/`, then rebuild:

```bash
uv run --project tools/drdebits_build --locked python -m drdebits_build build --root .
```

The build rewrites the generated files and `SHA256SUMS`. Commit them with the
source change; CI rejects a hand edit to a generated file. The README's
[Contributing and maintenance](README.md#contributing-and-maintenance) section
covers which files are generated and how evaluation records are kept.
[MAINTENANCE.md](MAINTENANCE.md) holds the source-check and release protocol,
including the hand-written copies a release must update.

## Checks a change must pass

[verify.yml](.github/workflows/verify.yml) runs these on Ubuntu and Windows with
Python 3.15. Run them before you push:

```bash
uv run --project tools/drdebits_build --locked --extra dev ruff check tools/drdebits_build/drdebits_build tools/drdebits_build/tests
uv run --project tools/drdebits_build --locked --extra dev mypy --config-file tools/drdebits_build/pyproject.toml tools/drdebits_build/drdebits_build
uv run --project tools/drdebits_build --locked --extra dev coverage run --rcfile=tools/drdebits_build/pyproject.toml --branch --source=tools/drdebits_build/drdebits_build -m pytest tools/drdebits_build/tests -q
uv run --project tools/drdebits_build --locked --extra dev coverage report --rcfile=tools/drdebits_build/pyproject.toml --fail-under=100
uv run --project tools/drdebits_build --locked python -m drdebits_build verify --root .
```

The Windows job runs the tests without coverage. CI also scans the full history
for secrets and client identifiers with gitleaks and
[.gitleaks.toml](.gitleaks.toml). `pre-commit install` runs the lint, type
check, tests, `verify` and gitleaks before each commit, through the same locked
toolchain.

## Tests for the build tooling

New or changed behaviour in `tools/drdebits_build` needs a test in
`tools/drdebits_build/tests`. CI holds the builder at 100% branch coverage, so
an untested branch fails the coverage report. A test for a fix should fail on
the base commit and pass with the change.
