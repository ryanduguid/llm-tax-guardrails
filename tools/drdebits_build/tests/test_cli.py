"""CLI behaviour via main() return codes."""
from datetime import date

import pytest

from drdebits_build import verify as verify_module
from drdebits_build.__main__ import main
from drdebits_build.verify import run_verify
from tests.test_build import make_repo

# The fixture repo's review date is fixed (due 2026-04-01), so the date the CLI
# checks it against has to be fixed too, or these tests would start failing on
# their own on that date. main() takes no date argument by design - verify's
# whole job is to read the real clock - so the seam is the module function it
# reads it through.
TODAY = date(2026, 2, 1)


@pytest.fixture(autouse=True)
def _pin_verification_date(monkeypatch):
    monkeypatch.setattr(verify_module, "_verification_date", lambda: TODAY)


def test_build_then_verify_roundtrip(tmp_path, capsys):
    root = make_repo(tmp_path)
    assert main(["build", "--root", str(root)]) == 0
    assert main(["verify", "--root", str(root)]) == 0
    assert "verify: OK" in capsys.readouterr().out


def test_verify_failure_exit_code(tmp_path):
    root = make_repo(tmp_path)
    main(["build", "--root", str(root)])
    (root / "drdebits.md").write_text("tampered", encoding="utf-8", newline="\n")
    assert main(["verify", "--root", str(root)]) == 1


def test_missing_root_reports_cleanly_for_both_commands(tmp_path, capsys, monkeypatch):
    """Regression: running outside a DrDebits tree is the likeliest error path,
    so it must print the module's clean message and exit 1, not a traceback."""
    monkeypatch.chdir(tmp_path)
    for command in ("verify", "build"):
        assert main([command]) == 1
        err = capsys.readouterr().err
        assert err.startswith(f"{command}: no DrDebits root found above ")
        assert "Traceback" not in err


def test_build_stamps_readme_and_maintenance_before_checksums(tmp_path):
    root = make_repo(tmp_path)
    (root / "README.md").write_text(
        "> Version: `0.0.1`\n\nSources last checked: 2026-01-01\n\n"
        "Install uv `0.12.0` before building.\n\n"
        "| catalogue | Complete live TPB Guidance Statement catalogue, GS01 to GS01 |\n",
        encoding="utf-8", newline="\n")
    (root / "MAINTENANCE.md").write_text(
        "Part of [DrDebits](./drdebits.md) `0.0.1`.\n", encoding="utf-8", newline="\n")
    assert main(["build", "--root", str(root)]) == 0
    readme = (root / "README.md").read_text(encoding="utf-8")
    assert "> Version: `0.9.9-test`" in readme
    # An unrelated backticked tool pin survives the stamp and does not fail verify.
    assert "`0.12.0`" in readme
    assert "0.9.9-test" in (root / "MAINTENANCE.md").read_text(encoding="utf-8")
    assert run_verify(root) == []
