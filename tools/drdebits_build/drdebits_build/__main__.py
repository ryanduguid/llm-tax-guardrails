"""CLI: build regenerates committed outputs in place; verify checks them.

`verify` answers in three states, not two:

    0   every check passed
    1   a check failed: the committed outputs do not match their sources
    2   the checks did not run, so nothing was checked

Without the third state an unexpected failure inside the checks exits 1
through Python's own traceback path, which is the status a real finding uses.
A reader of CI, human or not, then treats a checker that fell over as a
checker that found something. Inputs that cannot be read stay findings, as
`run_verify` already decides deliberately; the 2 band is for the checks
themselves failing to run.
"""
from __future__ import annotations

import argparse
import sys
import traceback
from pathlib import Path

from .build import (
    STAMPED_FILES,
    BuildError,
    find_root,
    load_sources,
    stamp_version,
    write_outputs,
)
from .model import ModelError
from .verify import run_verify

#: See the module docstring: 2 is "not checked", never "checked and clean".
EXIT_OK = 0
EXIT_CHECKS_FAILED = 1
EXIT_COULD_NOT_RUN = 2


def main(argv=None):
    parser = argparse.ArgumentParser(prog="drdebits_build")
    parser.add_argument("command", choices=["build", "verify"])
    parser.add_argument("--root", default=None)
    args = parser.parse_args(argv)
    # Root discovery is the first thing a user hits from outside a DrDebits
    # tree, so it must produce the same clean message as every other error
    # path rather than a traceback.
    try:
        root = Path(args.root) if args.root else find_root(Path.cwd())
    except BuildError as exc:
        print(f"{args.command}: {exc}", file=sys.stderr)
        return EXIT_COULD_NOT_RUN if args.command == "verify" else EXIT_CHECKS_FAILED
    if args.command == "build":
        try:
            s = load_sources(root)
            for rel in STAMPED_FILES:
                p = root / rel
                if p.is_file():
                    p.write_bytes(stamp_version(p.read_text(encoding="utf-8"), s.meta["guide_version"]).encode("utf-8"))
                    print(f"stamped {rel}")
            for rel in write_outputs(root, root, s):
                print(f"wrote {rel}")
        except (ModelError, BuildError, OSError) as exc:
            print(f"build: {exc}", file=sys.stderr)
            return 1
        return 0
    try:
        failures = run_verify(root)
    except Exception:
        # Deliberately broad: anything that stopped the checks running is
        # reported as "not checked" rather than as a finding. KeyboardInterrupt
        # and SystemExit are not Exception and still propagate.
        traceback.print_exc()
        print("verify: VERIFIER ERROR: the checks did not run, so these outputs "
              "are unverified, not verified", file=sys.stderr)
        return EXIT_COULD_NOT_RUN
    for f in failures:
        print(f, file=sys.stderr)
    if failures:
        return EXIT_CHECKS_FAILED
    print("verify: OK")
    return EXIT_OK


if __name__ == "__main__":
    raise SystemExit(main())
