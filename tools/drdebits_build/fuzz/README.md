The Atheris target feeds fabricated bytes to the YAML source reader and the
decision-status reader. It checks that the reader returns a mapping or refuses
with `ModelError`, and that every status the decision reader returns is empty
or one the guide allows. Unexpected exceptions and failed invariants fail the
run. The seed corpus holds the guide's own YAML sources and a few decision
lines.

On Linux with Python 3.14, from `tools/drdebits_build`, run:

```sh
python -m pip install --require-hashes --only-binary :all: -r requirements-fuzz.txt
python -m pip install --no-deps .
python -m fuzz.sources fuzz/corpus -max_total_time=60 -max_len=4096 -timeout=2 -rss_limit_mb=1024
```

CI runs the target after main pushes, weekly and on demand, with read-only
repository access and a five-minute timeout. The test suite replays the seed
corpus on every supported platform. Atheris does not support Windows, so
coverage-guided fuzzing runs on Linux. A successful run is bounded evidence,
not proof that every input is handled.

After changing the pinned version, regenerate its hashes with:

```sh
uv pip compile requirements-fuzz.in --python-version 3.14 --python-platform x86_64-unknown-linux-gnu --generate-hashes --output-file requirements-fuzz.txt
```
