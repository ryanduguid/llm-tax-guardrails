# drdebits_build changes

## 1.0.0 (unreleased)

The builder is licensed under FSL-1.1-ALv2 (Functional Source License, with
Apache-2.0 as the future licence) instead of MIT. Its port of PyYAML stays
under PyYAML's MIT licence, reproduced in `NOTICE`. Released versions keep
their licence.

The builder is now a Rust program. It replaces the Python package
`drdebits-build` 0.1.0, whose import package and Python API are removed. Build
and test it with the toolchain pinned in `rust-toolchain.toml`:

```bash
cargo run --manifest-path tools/drdebits_build/Cargo.toml --locked --release -- verify --root .
```

Every generated file, `SHA256SUMS` and the guide and case digests are byte for
byte what the Python builder wrote, and every finding, status line and exit
code is unchanged, except as listed below. A differential harness ran the
Python builder at llm-tax-guardrails 588784e and this binary over thousands of
generated source trees, records, responses files and bundles, and again over
2,500 source trees at 228dc40 after the port took in that revision's builder
changes; the only differences it found are the documented ones. `tests/data/python_readers.json`
pins how the Python builder read 878 YAML and 41 JSON texts, and the test
suite replays it on every platform.

### Command line

- `python -m drdebits_build build|verify|digests` becomes
  `drdebits_build build|verify|digests`. The module commands become
  subcommands: `compare`, `bundle`, `linkcheck` and `sources`, with the same
  arguments and exit codes.
- The Python library calls become subcommands: `score_from_file` is `score`,
  `assessment_lines` is `assess`, `read_source` is `read-source` and
  `assess_quote` is `quote`. They print what the function returned and exit 1,
  with `<command>: <reason>`, where the function raised.
- Usage and help text come from the Rust argument parser. A usage error still
  exits 2.
- `read-source --mode live` without approval names the
  `--live-retrieval-approved` flag instead of the Python keyword, and the
  result-record digest hint names `drdebits_build digests`.
- New `gate` command, with no Python counterpart: checks one high-impact
  draft against the output contract and recomputes each written `a op b = c`
  calculation in exact decimal arithmetic. A claimed result passes when it is
  within one unit of its last decimal place; each link of a chain must equal
  the next expression or, as in a running total, its leading number; division
  by zero and a lone bracketed amount such as `(120)`, which may be an
  accounting negative, block. A mandatory section must have content. Exit 0 is a
  mechanical pass, 1 a contract or arithmetic breach, 2 a gate that could not
  run and 3, for human review, a prohibited-conclusion screen match or a
  recorded state-changing action. An `ESCALATE` or
  `HARD_STOP` draft may be the abbreviated record the Output contract allows;
  its other mandatory sections are reported as notes.

### Documented divergences

- **Unhandled errors.** Where the Python builder let an exception escape, it
  printed a traceback. The binary prints one `<command>: unexpected error: …`
  line instead, with the same exit code: `verify` still exits 2 and prints its
  `VERIFIER ERROR` line; `build` and `digests` exit 1.
- **Nesting depth.** Python's recursion limit made the deepest readable YAML
  nesting depend on the syntax and the command: 489 to 492 levels in
  measurement. The binary reads up to 490 composed levels and refuses deeper
  documents with the same `is nested too deeply to read` finding. `verify`
  reports a deeper `CITATION.cff` as `cannot parse YAML (maximum recursion
  depth exceeded)`.
- **Lone surrogates.** A double-quoted YAML escape or a JSON `\u` escape for a
  lone UTF-16 surrogate (U+D800 to U+DFFF) is refused when read. Python kept
  the character and failed later, when a generated file was written, except
  where the value never reached an output.
- **Set order.** A `!!set` value quoted in a finding lists its members in
  source order. Python listed them in hash order, which varied between runs.
- **Network classification.** `linkcheck` maps transport failures to the
  nearest Python exception names (`ConnectionRefusedError`,
  `ConnectionResetError`, `timeout`, `SSLError`, `SSLCertVerificationError`,
  `RemoteDisconnected`, otherwise `OSError`). Certificates are checked with the
  platform verifier, and a redirect target is joined with WHATWG URL rules
  before it is checked. Dead, unreachable and manual classifications, the
  destination guard and the exit codes are unchanged.
- **Response size.** `sources` reads at most 64 MiB of a Federal Register
  API response and reports a larger one as unreachable. Python read the whole
  response.
- **Operating-system messages.** File errors are rendered as Python rendered
  them for the common cases (missing file, permission, existing target);
  unusual error codes may be worded differently.

### Checks

- CI runs `cargo fmt --check`, `cargo clippy -- -D warnings`, the test suite on
  Ubuntu and Windows, and line coverage with a 92% floor (92.9% on Ubuntu and
  92.8% on Windows at release). The Python builder was held at 100% branch
  coverage; stable Rust has no branch coverage measure.
- Fuzzing moves from Atheris to cargo-fuzz on a pinned nightly toolchain.
- CodeQL analyses Rust instead of Python. Dependabot updates the Cargo
  dependencies. Snyk tests the Cargo dependencies through a CycloneDX SBOM with
  `snyk sbom test`.
- The link-check and source-currency workflows build the checker in a
  read-only job and run it in the job that may write issues, so that job runs
  no dependency build script.
- `.github/ci/check_gates.py` stays in Python: it is a gate helper shared
  byte for byte with other repositories, run by system Python in CI.
