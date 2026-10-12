"""Fuzz the YAML source reader and the decision-status reader with fabricated input."""

from __future__ import annotations

import sys
import tempfile
from pathlib import Path

from drdebits_build import harness, model
from drdebits_build.model import ALLOWED_STATUSES, ModelError

# One scratch file, rewritten for each input: the reader takes a path.
SCRATCH = Path(tempfile.mkdtemp(prefix="drdebits-fuzz-")) / "source.yaml"


def test_one_input(data: bytes) -> None:
    text = data.decode("utf-8", errors="replace")
    for status in harness.decision_statuses(text):
        if status and status not in ALLOWED_STATUSES:
            raise AssertionError(f"decision reader returned unknown status {status!r}")

    SCRATCH.write_bytes(data)
    try:
        loaded = model._read(SCRATCH)
    except ModelError:
        return
    if not isinstance(loaded, dict):
        raise AssertionError("the reader returned a non-mapping without refusing it")


if __name__ == "__main__":
    import atheris

    atheris.instrument_all()
    atheris.Setup(sys.argv, test_one_input)
    atheris.Fuzz()
