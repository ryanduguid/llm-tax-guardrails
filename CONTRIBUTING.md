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
cargo run --manifest-path tools/drdebits_build/Cargo.toml --locked --release -- build --root .
```

The build rewrites the generated files and `SHA256SUMS`. Commit them with the
source change; CI rejects a hand edit to a generated file. The README's
[Contributing and maintenance](README.md#contributing-and-maintenance) section
covers which files are generated and how evaluation records are kept.
[MAINTENANCE.md](MAINTENANCE.md) holds the source-check and release protocol,
including the hand-written copies a release must update.

## Checks a change must pass

[verify.yml](.github/workflows/verify.yml) runs these on Ubuntu and Windows
with the Rust toolchain pinned in [rust-toolchain.toml](rust-toolchain.toml).
Run them from `tools/drdebits_build` before you push:

```bash
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo llvm-cov --locked --fail-under-lines 90
cargo run --locked --release -- verify --root ../..
```

`cargo llvm-cov` runs the tests with line coverage; `cargo test --locked` runs
them without it, as the Windows job does. CI also scans the full history for
secrets and client identifiers with gitleaks and [.gitleaks.toml](.gitleaks.toml).
`pre-commit install` runs formatting, lint, tests, `verify` and gitleaks before
each commit, through the same pinned toolchain.

## Tests for the build tooling

New or changed behaviour in `tools/drdebits_build` needs a test in
`tools/drdebits_build/tests` or beside the code it covers. CI holds the builder
at 90% line coverage, so a large untested change fails the coverage
report. A test for a fix should fail on the base commit and pass with the
change.
