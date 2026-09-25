"""Install an approved manifest's complete bundle without overwriting a copy.

The approved manifest digest comes from the deployer's trusted release record.
Matching bytes proves integrity, not release approval or host enforcement.
"""
from __future__ import annotations

import argparse
import hashlib
import re
import sys
from pathlib import Path

REQUIRED = frozenset({
    "LICENSE", "README.md", "CITATION.cff", "drdebits.md", "MAINTENANCE.md",
    "reference/tpb-catalogue.md", "reference/apes-110-map.md", "tests/behaviour-tests.md",
})
OPTIONAL = frozenset({"reference/ai-vendor-assurance.md", "DISCLAIMER.md"})


class BundleError(ValueError):
    pass


def _read(root: Path, relative: str) -> bytes:
    path = root / relative
    if not path.resolve().is_relative_to(root.resolve()):
        raise BundleError(f"bundle path escapes source: {relative}")
    # Reject redirected files and directories even when they point inside root.
    for part in (path, *path.parents):
        if part == root:
            break
        if part.is_symlink() or part.is_junction():
            raise BundleError(f"redirected bundle path: {relative}")
    return path.read_bytes()


def verified_files(source: Path, manifest_sha256: str) -> dict[str, bytes]:
    """Validate membership and digests before returning bytes to install."""
    if not re.fullmatch(r"[0-9a-f]{64}", manifest_sha256):
        raise BundleError("approved manifest digest must be 64 lower-case hex characters")
    manifest = _read(source, "SHA256SUMS")
    if hashlib.sha256(manifest).hexdigest() != manifest_sha256:
        raise BundleError("SHA256SUMS does not match the approved manifest digest")
    entries: dict[str, str] = {}
    for line in manifest.decode("utf-8").splitlines():
        match = re.fullmatch(r"([0-9a-f]{64}) [ *](.+)", line)
        if not match:
            raise BundleError("malformed checksum entry")
        digest, relative = match.groups()
        if relative not in REQUIRED | OPTIONAL or relative in entries:
            raise BundleError(f"unexpected or duplicate bundle member: {relative}")
        entries[relative] = digest
    if not REQUIRED <= entries.keys():
        raise BundleError("manifest is missing required bundle members")
    files = {"SHA256SUMS": manifest}
    for relative, digest in entries.items():
        raw = _read(source, relative)
        if hashlib.sha256(raw).hexdigest() != digest:
            raise BundleError(f"checksum mismatch: {relative}")
        files[relative] = raw
    return files


def install(source: Path, destination: Path, manifest_sha256: str) -> None:
    files = verified_files(source, manifest_sha256)
    # The deployer owns the parent directory. Never reuse an existing target,
    # including a broken symlink. A failed copy stays unapproved for inspection.
    destination.mkdir(parents=False, exist_ok=False)
    for relative, raw in files.items():
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        with target.open("xb") as stream:
            stream.write(raw)
    verified_files(destination, manifest_sha256)


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("install", "verify"))
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--manifest-sha256", required=True)
    parser.add_argument("--destination", type=Path)
    args = parser.parse_args(argv)
    if args.command == "install" and args.destination is None:
        parser.error("install requires --destination (an unused directory in an existing parent)")
    try:
        if args.command == "install":
            install(args.source, args.destination, args.manifest_sha256)
        else:
            verified_files(args.source, args.manifest_sha256)
    except (OSError, ValueError) as exc:
        print(f"bundle: {exc}", file=sys.stderr)
        return 1
    print("bundle: OK (approved manifest and file digests; host loading not checked)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
