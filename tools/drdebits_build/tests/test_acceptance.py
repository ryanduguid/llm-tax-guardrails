"""Acceptance: the real repo verifies clean; hand edits fail; version bump propagates."""
import shutil
from datetime import date
from pathlib import Path

import pytest

from drdebits_build import model, verify
from drdebits_build.__main__ import main
from drdebits_build.build import find_root

REAL = find_root(Path(__file__).resolve())
COPY_ITEMS = ["src", "drdebits.md", "reference", "tests", "evals", "SHA256SUMS",
              "README.md", "MAINTENANCE.md", "LICENSE", "CITATION.cff", "llms.txt"]

REAL_META = model.load_metadata(REAL / "src" / "data" / "metadata.yaml")
# The date verify checks review_due against. Every test here drives verify
# through main(), which takes no date argument, so an unpinned
# _verification_date puts the wall clock into check (g): from the repo's own
# review_due onward, test_real_repo_verifies and test_version_bump_propagates
# would go red on a tree nobody touched. Pinning today to the repo's
# source-check date restores the date independence these tests had before
# check (g) existed, and check (g) itself requires review_due to be after that
# date, so the pin can never bless a release the gate rejects. The
# verification-date arm of check (g) is exercised with explicit dates in
# tests/test_verify.py, which is where it belongs.
PINNED_TODAY = date.fromisoformat(REAL_META["sources_checked_at"][:10])


@pytest.fixture(autouse=True)
def pin_verification_date(monkeypatch):
    monkeypatch.setattr(verify, "_verification_date", lambda: PINNED_TODAY)


def test_verification_date_is_pinned_not_read_from_the_wall_clock():
    """Guard for the fixture above. It fails if the pin is removed, and it
    fails if a source recheck ever advances sources_checked_at to or past
    review_due, which is the one way the pinned date could hide a real
    expiry from the tests below."""
    assert verify._verification_date() == PINNED_TODAY
    assert PINNED_TODAY < date.fromisoformat(REAL_META["review_due"])


def clone_repo(tmp_path):
    for item in COPY_ITEMS:
        src = REAL / item
        dst = tmp_path / item
        if src.is_dir():
            shutil.copytree(src, dst)
        else:
            shutil.copy2(src, dst)
    return tmp_path


def test_real_repo_verifies(tmp_path):
    assert main(["verify", "--root", str(clone_repo(tmp_path))]) == 0


def test_hand_edit_fails(tmp_path):
    root = clone_repo(tmp_path)
    p = root / "reference" / "tpb-catalogue.md"
    p.write_text(p.read_text(encoding="utf-8").replace("TPB", "TBP", 1), encoding="utf-8", newline="\n")
    assert main(["verify", "--root", str(root)]) == 1


def test_version_bump_propagates(tmp_path):
    """The true bump procedure (MAINTENANCE.md step 8): metadata.yaml, the guide's
    own header version line, the changelog's newest entry and CITATION.cff all
    move together - whatever the current version is."""
    root = clone_repo(tmp_path)
    current = model.load_metadata(root / "src" / "data" / "metadata.yaml")["guide_version"]
    # Bump the patch digit so the result matches the stamp token pattern for
    # suffixed and bare versions alike (an appended ".bump" would not).
    import re
    m = re.match(r"(\d+)\.(\d+)\.(\d+)(.*)", current)
    bumped = f"{m.group(1)}.{m.group(2)}.{int(m.group(3)) + 1}{m.group(4)}"
    for rel in ("src/data/metadata.yaml", "src/guide/000-header.md",
                "src/data/changelog.yaml", "CITATION.cff"):
        p = root / rel
        text = p.read_text(encoding="utf-8").replace(current, bumped)
        p.write_text(text, encoding="utf-8", newline="\n")
    assert main(["build", "--root", str(root)]) == 0
    assert main(["verify", "--root", str(root)]) == 0
    assert bumped in (root / "README.md").read_text(encoding="utf-8")
    assert (root / "drdebits.md").read_text(encoding="utf-8").rstrip("\n").endswith(bumped)
