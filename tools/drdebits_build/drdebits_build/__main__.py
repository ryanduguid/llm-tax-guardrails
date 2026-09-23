"""CLI: build regenerates committed outputs in place; verify checks them.

`digests` prints the guide and case-export digests for the current tree, which
is what an evaluation record has to carry to identify what a run was run
against. It reads the sources and writes nothing.

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

from . import evals
from .build import (
    GENERATED,
    STAMPED_FILES,
    BuildError,
    build_digests,
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

#: What `verify` checked, so a passing line cannot be read as a wider clearance
#: than the checks give. Source currency is a separate command over the network.
VERIFY_OK_LINE = ("verify: OK (structure, version copies and review window; "
                  "not a source-currency check)")


def main(argv=None):
    parser = argparse.ArgumentParser(prog="drdebits_build")
    parser.add_argument("command", choices=["build", "verify", "digests"])
    parser.add_argument("--root", default=None)
    args = parser.parse_args(argv)
    # Root discovery is the first thing a user hits from outside a DrDebits
    # tree, so it must produce the same clean message as every other error
    # path rather than a traceback. For verify it also lands in the 2 band:
    # no root means the checks never started, which is not the same answer as
    # a check that ran and failed. For build it stays 1, because build is not
    # a gate and a build that cannot start is simply a build that failed.
    try:
        root = Path(args.root) if args.root else find_root(Path.cwd())
    except BuildError as exc:
        print(f"{args.command}: {exc}", file=sys.stderr)
        return EXIT_COULD_NOT_RUN if args.command == "verify" else 1
    if args.command == "build":
        try:
            s = load_sources(root)
            # Render everything that does not read the stamped files, and discard it.
            # The stamped files are written first because SHA256SUMS covers them, so
            # a malformed evaluation run used to fail the command with README.md,
            # MAINTENANCE.md and llms.txt already changed. Failing here leaves the
            # worktree as it was.
            for render in GENERATED.values():
                render(s)
            evals.build_results_md(root, s)
            for rel in STAMPED_FILES:
                p = root / rel
                if p.is_file():
                    p.write_bytes(stamp_version(p.read_text(encoding="utf-8"), s.meta["guide_version"]).encode("utf-8"))
                    print(f"stamped {rel}")
            for rel in write_outputs(root, root, s):
                print(f"wrote {rel}")
        except (ModelError, BuildError, OSError, UnicodeDecodeError) as exc:
            print(f"build: {exc}", file=sys.stderr)
            return 1
        return 0
    if args.command == "digests":
        # The 2 digests a result or observation record has to carry. Printed for
        # the tree as it stands, so a runner can fill them in before the rebuild
        # is committed.
        try:
            for key, digest in build_digests(load_sources(root)).items():
                print(f"{key}: {digest}")
        except (ModelError, BuildError, OSError) as exc:
            print(f"digests: {exc}", file=sys.stderr)
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
    print(VERIFY_OK_LINE)
    return EXIT_OK


if __name__ == "__main__":
    raise SystemExit(main())
