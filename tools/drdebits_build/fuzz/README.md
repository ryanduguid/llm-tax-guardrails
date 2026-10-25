The cargo-fuzz target feeds fabricated bytes to the YAML source reader and the
decision-status reader. It checks that the reader returns a mapping or refuses
with a source finding, and that every status the decision reader returns is
empty or one the guide allows. A panic, an unexpected error or a failed
invariant fails the run. The seed corpus holds the guide's own YAML sources and
a few decision lines.

On Linux, from `tools/drdebits_build`, run:

```sh
rustup toolchain install nightly-2026-10-15 --profile minimal
cargo install cargo-fuzz --locked --version 0.13.2
cargo +nightly-2026-10-15 fuzz run sources fuzz/corpus -- -max_total_time=60 -max_len=4096 -timeout=2 -rss_limit_mb=1024
```

CI runs the target after main pushes, weekly and on demand, with read-only
repository access. The test suite replays the seed corpus on every supported
platform (`tests/fuzz_corpus.rs`). libFuzzer needs a nightly toolchain, so
coverage-guided fuzzing runs on Linux. A successful run is bounded evidence,
not proof that every input is handled.
