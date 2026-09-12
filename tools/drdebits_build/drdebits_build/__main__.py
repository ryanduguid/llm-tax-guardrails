"""CLI: build regenerates committed outputs in place; verify checks them."""
from __future__ import annotations

import argparse
import sys
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
        return 1
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
    failures = run_verify(root)
    for f in failures:
        print(f, file=sys.stderr)
    if failures:
        return 1
    print("verify: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
