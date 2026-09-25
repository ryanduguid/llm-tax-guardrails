"""Installed bundles retain their approved bytes and never overwrite a copy."""
import hashlib
from pathlib import Path

import pytest
from drdebits_build import bundle


def make_bundle(tmp_path, optional=True):
    source = tmp_path / "source"
    source.mkdir()
    lines = []
    for name in sorted(bundle.REQUIRED | (bundle.OPTIONAL if optional else set())):
        path = source / name
        path.parent.mkdir(parents=True, exist_ok=True)
        raw = f"Synthetic bundle: {name}\n".encode()
        path.write_bytes(raw)
        lines.append(f"{hashlib.sha256(raw).hexdigest()} *{name}\n")
    manifest = "".join(lines).encode()
    (source / "SHA256SUMS").write_bytes(manifest)
    return source, hashlib.sha256(manifest).hexdigest()


@pytest.mark.parametrize("optional", [False, True])
def test_complete_install_preserves_historical_and_current_bundles(tmp_path, optional):
    source, digest = make_bundle(tmp_path, optional)
    target = tmp_path / "installed"
    bundle.install(source, target, digest)
    assert bundle.verified_files(target, digest) == bundle.verified_files(source, digest)
    with pytest.raises(FileExistsError):
        bundle.install(source, target, digest)
    (target / "drdebits.md").write_text("tampered")
    assert bundle.main(["verify", "--source", str(target), "--manifest-sha256", digest]) == 1


@pytest.mark.parametrize("damage", ["digest", "missing", "duplicate", "escape", "unknown"])
def test_untrusted_or_incomplete_manifest_never_creates_destination(tmp_path, damage):
    source, digest = make_bundle(tmp_path)
    manifest = source / "SHA256SUMS"
    lines = manifest.read_text().splitlines(keepends=True)
    if damage == "digest":
        digest = "0" * 64
    else:
        if damage == "missing":
            lines = [line for line in lines if not line.endswith(" *drdebits.md\n")]
        elif damage == "duplicate":
            lines.append(lines[0])
        else:
            lines.append("0" * 64 + " *" + ("../outside" if damage == "escape" else ".env") + "\n")
        manifest.write_bytes("".join(lines).encode())
        digest = hashlib.sha256(manifest.read_bytes()).hexdigest()
    target = tmp_path / "installed"
    with pytest.raises(bundle.BundleError):
        bundle.install(source, target, digest)
    assert not target.exists()


def test_changed_or_missing_file_never_creates_destination(tmp_path):
    source, digest = make_bundle(tmp_path)
    (source / "drdebits.md").write_bytes(b"changed")
    target = tmp_path / "installed"
    with pytest.raises(bundle.BundleError, match="checksum mismatch"):
        bundle.install(source, target, digest)
    assert not target.exists()
    (source / "drdebits.md").unlink()
    with pytest.raises(FileNotFoundError):
        bundle.install(source, target, digest)
    assert not target.exists()


def test_redirected_bundle_member_is_rejected_before_read(tmp_path, monkeypatch):
    source, digest = make_bundle(tmp_path)
    original = Path.is_symlink
    monkeypatch.setattr(Path, "is_symlink", lambda p: p.name == "reference" or original(p))
    with pytest.raises(bundle.BundleError, match="redirected"):
        bundle.verified_files(source, digest)


def test_checked_in_bundle_installs_and_cli_verifies(tmp_path):
    source = Path(__file__).resolve().parents[3]
    digest = hashlib.sha256((source / "SHA256SUMS").read_bytes()).hexdigest()
    target = tmp_path / "current"
    assert bundle.main(["install", "--source", str(source), "--destination", str(target),
                        "--manifest-sha256", digest]) == 0
    assert bundle.main(["verify", "--source", str(target), "--manifest-sha256", digest]) == 0
