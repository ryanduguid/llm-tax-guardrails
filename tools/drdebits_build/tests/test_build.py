"""Tests for output assembly against a synthetic source tree."""
import textwrap
from pathlib import Path
import pytest
from drdebits_build.build import (
    BuildError, find_root, load_sources, build_guide, build_catalogue_md,
    build_behaviour_md, build_apes_md, build_sha256sums,
    write_outputs, stamp_version, GENERATED,
)


def make_repo(tmp_path: Path) -> Path:
    (tmp_path / "src" / "guide").mkdir(parents=True)
    (tmp_path / "src" / "data").mkdir()
    (tmp_path / "drdebits.md").write_text("placeholder", encoding="utf-8")
    (tmp_path / "LICENSE").write_text("MIT-ish\n", encoding="utf-8", newline="\n")
    # The README GS-range copy MAINTENANCE step 8 names. verify requires the
    # documented copy to be present, so the fixture repo has to carry the same
    # hand-written copy as the real one.
    (tmp_path / "README.md").write_text(
        "> Version: `0.9.9-test`\n\nSources last checked: 2026-01-01\n\n"
        "| catalogue | Complete live TPB Guidance Statement catalogue, GS01 to GS01 |\n",
        encoding="utf-8", newline="\n")
    (tmp_path / "MAINTENANCE.md").write_text(
        "Part of [DrDebits](./drdebits.md) `0.9.9-test`.\n", encoding="utf-8", newline="\n")
    (tmp_path / "src" / "guide" / "000-header.md").write_text(
        "# G\n\n> Version: `0.9.9-test`\n>\n> Sources last checked: `2026-01-01`\n\nIntro.\n",
        encoding="utf-8", newline="\n")
    (tmp_path / "src" / "guide" / "010-rules.md").write_text("## Rules\n\nBe good.\n", encoding="utf-8", newline="\n")
    # The three guide GS-range copies and two count copies MAINTENANCE step 8
    # names: the source-status table row, its prose sentence and the
    # workpaper-record catalogue line.
    (tmp_path / "src" / "guide" / "020-sources.md").write_text(
        "## Sources\n\n"
        "| TPB library | 1 indexed statements, GS01 to GS01 | note |\n\n"
        "At the source-check date, the filtered TPB library index exposed 1 live "
        "Guidance Statements, GS01 to GS01, across one result page.\n\n"
        "- reference/tpb-catalogue.md: complete live TPB Guidance Statement "
        "catalogue, GS01 to GS01\n",
        encoding="utf-8", newline="\n")
    (tmp_path / "src" / "data" / "metadata.yaml").write_text(textwrap.dedent("""\
        fields:
          - key: guide_version
            value: "0.9.9-test"
          - key: release_tag
            value: "v0.9.9-test"
          - key: guide_end_marker
            value: "DRDEBITS-END-v0.9.9-test"
          - key: tpb_guidance_statement_count
            value: "1"
          - key: tpb_library_index_count
            value: "1"
          - key: sources_checked_at
            value: "2026-01-01T00:00:00+10:00"
          - key: review_due
            value: "2026-04-01"
          - key: checksum_files
            value: "LICENSE|README.md|drdebits.md"
    """), encoding="utf-8", newline="\n")
    (tmp_path / "src" / "data" / "tpb-catalogue.yaml").write_text(textwrap.dedent("""\
        entries:
          - id: "GS01"
            title: "T"
            url: "https://x.invalid/a"
            trigger: "g"
    """), encoding="utf-8", newline="\n")
    (tmp_path / "src" / "data" / "behaviour-tests.yaml").write_text(textwrap.dedent("""\
        entries:
          - id: "A-001"
            scenario: "s"
            expected_status: "HARD_STOP"
            required_behaviour: "r"
            side_effect_check: "c"
    """), encoding="utf-8", newline="\n")
    (tmp_path / "src" / "data" / "apes-110-map.yaml").write_text(textwrap.dedent("""\
        contexts:
          - label: "All"
            value: "Part 1"
        retrieval_points:
          - label: "Scope"
            value: "R1.2"
    """), encoding="utf-8", newline="\n")
    (tmp_path / "src" / "data" / "changelog.yaml").write_text(textwrap.dedent("""\
        entries:
          - version: "0.9.9-test"
            date: "2026-01-01"
            status: "Draft"
            change: "init"
    """), encoding="utf-8", newline="\n")
    return tmp_path


def test_find_root(tmp_path):
    root = make_repo(tmp_path)
    nested = root / "src" / "guide"
    assert find_root(nested) == root
    with pytest.raises(BuildError):
        find_root(tmp_path.parent)


def test_guide_shape_and_determinism(tmp_path):
    s = load_sources(make_repo(tmp_path))
    g1, g2 = build_guide(s), build_guide(s)
    assert g1 == g2
    assert g1.startswith("---\nguide_version: 0.9.9-test\n")
    assert g1.rstrip("\n").endswith("DRDEBITS-END-v0.9.9-test")
    assert "## Rules" in g1 and "| 0.9.9-test | 2026-01-01 | Draft | init |" in g1


def test_catalogue_md_header_version_and_table(tmp_path):
    s = load_sources(make_repo(tmp_path))
    out = build_catalogue_md(s)
    assert out.count("0.9.9-test") == 1
    assert out.startswith("# Complete TPB Guidance Statement catalogue\n")
    assert "| Statement (concise title and official link) | LLM trigger |" in out
    assert "| [T](https://x.invalid/a) | g |" in out


def test_catalogue_md_header_derived_from_metadata_and_rows(tmp_path):
    """Count, GS range and check date come from the sources, not template
    literals, so a catalogue change cannot ship a stale header. The fixture's
    "1 statements" is synthetic; the real catalogue is always plural."""
    s = load_sources(make_repo(tmp_path))
    out = build_catalogue_md(s)
    assert "discoverable on 1 January 2026: 1 statements, GS01 to GS01," in out


def test_behaviour_md_header_version_and_table(tmp_path):
    s = load_sources(make_repo(tmp_path))
    out = build_behaviour_md(s)
    assert out.count("0.9.9-test") == 1
    assert out.startswith("# DrDebits behaviour tests\n")
    assert "| ID | Scenario | Expected status | Required behaviour and human step | Side-effect check |" in out
    assert "| A-001 | s | HARD_STOP | r | c |" in out


def test_apes_md_header_version_and_both_tables(tmp_path):
    s = load_sources(make_repo(tmp_path))
    out = build_apes_md(s)
    assert out.count("0.9.9-test") == 1
    assert out.startswith("# Primary APES 110 reference map\n")
    assert "| Context | APES 110 starting points |" in out
    assert "| All | Part 1 |" in out
    assert "Key paragraph-level retrieval points are:" in out
    assert "| Control | APES 110 retrieval points |" in out
    assert "| Scope | R1.2 |" in out


def test_sha256sums_covers_generated_and_static(tmp_path):
    root = make_repo(tmp_path)
    s = load_sources(root)
    out = build_sha256sums(root, s, {rel: fn(s) for rel, fn in GENERATED.items()})
    lines = out.rstrip("\n").split("\n")
    assert lines[0].endswith(" *LICENSE") and lines[2].endswith(" *drdebits.md")
    assert all(len(line.split(" *")[0]) == 64 for line in lines)


def test_write_outputs_lf_only(tmp_path):
    root = make_repo(tmp_path)
    outdir = tmp_path / "out"
    written = write_outputs(root, outdir)
    assert "drdebits.md" in written and "SHA256SUMS" in written
    raw = (outdir / "drdebits.md").read_bytes()
    assert b"\r" not in raw


def test_fragment_order_is_case_sensitive_string_sort(tmp_path):
    """Fragments sort by name string on every platform. Path-object sorting
    casefolds on Windows but not POSIX, which would let a mixed-case fragment
    name order differently on the dev box and the CI rebuild."""
    root = make_repo(tmp_path)
    (root / "src" / "guide" / "500-B.md").write_text("## B section\n", encoding="utf-8", newline="\n")
    (root / "src" / "guide" / "500-a.md").write_text("## a section\n", encoding="utf-8", newline="\n")
    s = load_sources(root)
    guide = build_guide(s)
    assert guide.index("## B section") < guide.index("## a section")


def test_stamp_version():
    assert stamp_version("> Version: `0.2.0-draft`", "0.3.0-draft") == "> Version: `0.3.0-draft`"
    assert stamp_version(
        "Part of [DrDebits](./drdebits.md) `0.2.0-draft`.", "0.3.0-draft"
    ) == "Part of [DrDebits](./drdebits.md) `0.3.0-draft`."
    # Backticked version-like tokens outside the two stamp contexts are prose
    # (e.g. a documented tool pin): left alone.
    assert stamp_version("Install uv `0.12.0` before building.", "2.0.0") == \
        "Install uv `0.12.0` before building."
    # Bare, non-backticked version-like tokens are prose, not a stamp: left alone.
    assert stamp_version("v1.2.3 and 9.9.9-x.1", "2.0.0") == "v1.2.3 and 9.9.9-x.1"
