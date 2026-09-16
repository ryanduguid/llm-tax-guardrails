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
    """Running outside a DrDebits tree is the likeliest error path, so it must
    print the module's clean message rather than a traceback.

    The statuses differ on purpose. For verify, no root means the checks never
    started, which belongs in the 2 band with every other way of not checking:
    a caller that reads 1 as "the outputs are wrong" would be told a falsehood
    about outputs nobody looked at. Build is not a gate, so a build that
    cannot start is a build that failed, and stays 1.
    """
    monkeypatch.chdir(tmp_path)
    for command, expected in (("verify", 2), ("build", 1)):
        assert main([command]) == expected
        err = capsys.readouterr().err
        assert err.startswith(f"{command}: no DrDebits root found above ")
        assert "Traceback" not in err


def test_build_stamps_every_hand_written_file_before_checksums(tmp_path):
    root = make_repo(tmp_path)
    (root / "llms.txt").write_text(
        "# G\n\nVersion: `0.0.1`\nSources last checked: 2026-01-01\n",
        encoding="utf-8", newline="\n")
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
    assert "Version: `0.9.9-test`" in (root / "llms.txt").read_text(encoding="utf-8")
    assert run_verify(root) == []

    # The source-check date is hand-written in llms.txt, not stamped, so a
    # metadata bump that misses it has to fail verify rather than ship an index
    # that dates the guide's sources differently from the README.
    (root / "llms.txt").write_text(
        "# G\n\nVersion: `0.9.9-test`\nSources last checked: 2025-12-31\n",
        encoding="utf-8", newline="\n")
    assert run_verify(root) == [
        "llms.txt: source-check date does not match sources_checked_at (2026-01-01)"]


def test_verify_reports_a_broken_checker_as_2_not_1(tmp_path, capsys, monkeypatch):
    """A checker that falls over must not read as a checker that found a fault.

    Both exit 1 by default, because an escaping exception takes Python's own
    traceback path, and the caller reading "non-zero" cannot tell the two
    apart. Inputs that cannot be read stay findings; this band is for the
    checks themselves failing to run.
    """
    root = make_repo(tmp_path)
    main(["build", "--root", str(root)])

    def explode(*_args, **_kwargs):
        raise RuntimeError("the checker itself broke")

    monkeypatch.setattr("drdebits_build.__main__.run_verify", explode)
    assert main(["verify", "--root", str(root)]) == 2
    err = capsys.readouterr().err
    assert "VERIFIER ERROR" in err
    assert "unverified, not verified" in err


def test_the_three_verify_statuses_are_distinct():
    from drdebits_build.__main__ import (
        EXIT_CHECKS_FAILED,
        EXIT_COULD_NOT_RUN,
        EXIT_OK,
    )

    assert sorted({EXIT_OK, EXIT_CHECKS_FAILED, EXIT_COULD_NOT_RUN}) == [0, 1, 2]


def test_unreadable_sources_stay_a_finding(tmp_path):
    # The deliberate existing choice, pinned so the band above does not creep
    # into it: run_verify turns an unreadable sources file into a finding.
    root = make_repo(tmp_path)
    main(["build", "--root", str(root)])
    (root / "src" / "data" / "metadata.yaml").write_text("{ not: [valid", encoding="utf-8")
    assert main(["verify", "--root", str(root)]) == 1
