"""Verification pass tests: tamper each way, expect the named failure."""
from datetime import date

from drdebits_build.build import write_outputs
from drdebits_build.verify import run_verify
from tests.test_build import make_repo

# The fixture repo's dates are fixed (checked 2026-01-01, review due
# 2026-04-01), so the date verify checks them against must be fixed too. Left
# on the wall clock, every assertion below would start failing on 2026-04-01 -
# for the right reason, but not the one the test is about.
TODAY = date(2026, 2, 1)


def verify(root, today=TODAY):
    return run_verify(root, today=today)


def sync(root):
    write_outputs(root, root)


def test_clean_repo_verifies(tmp_path):
    root = make_repo(tmp_path)
    sync(root)
    assert verify(root) == []


def test_malformed_source_key_is_reported_not_raised(tmp_path):
    """run_verify returns messages; it does not raise. A complex YAML key
    constructs to a list, so the duplicate-key check has to reject it as a
    ConstructorError rather than letting `key in mapping` throw a TypeError
    that escapes _read's conversion and this function's own handler."""
    root = make_repo(tmp_path)
    sync(root)
    (root / "src" / "data" / "metadata.yaml").write_text(
        "? [complex, key]\n: value\n", encoding="utf-8", newline="\n")
    failures = verify(root)
    assert any("unhashable key" in f for f in failures), failures


def test_tampered_output_names_file_and_src(tmp_path):
    root = make_repo(tmp_path)
    sync(root)
    guide = root / "drdebits.md"
    guide.write_text(guide.read_text(encoding="utf-8") + "tamper\n", encoding="utf-8", newline="\n")
    failures = verify(root)
    assert any("drdebits.md" in f and "edit src/" in f for f in failures)


def test_stamped_version_mismatch_detected(tmp_path):
    root = make_repo(tmp_path)
    sync(root)
    (root / "README.md").write_text("> Version: `0.0.1`\n", encoding="utf-8", newline="\n")
    failures = verify(root)
    assert any("README.md" in f and "0.0.1" in f for f in failures)


def test_bad_yaml_becomes_message_not_traceback(tmp_path):
    root = make_repo(tmp_path)
    sync(root)
    bt = root / "src" / "data" / "behaviour-tests.yaml"
    bt.write_text(bt.read_text(encoding="utf-8").replace("HARD_STOP", "NOT_A_STATUS"), encoding="utf-8", newline="\n")
    failures = verify(root)
    assert any("NOT_A_STATUS" in f for f in failures)


# Stamped-file presence and version coverage
def test_missing_stamped_file_detected(tmp_path):
    """Regression: a missing stamped file must be detected."""
    root = make_repo(tmp_path)
    sync(root)
    (root / "MAINTENANCE.md").unlink()
    failures = verify(root)
    assert any("MAINTENANCE.md: missing" in f for f in failures)


def test_stripped_version_stamp_detected(tmp_path):
    """Regression: a stripped version stamp (zero matches) must be detected."""
    root = make_repo(tmp_path)
    sync(root)
    (root / "MAINTENANCE.md").write_text("Protocol for maintenance.\n", encoding="utf-8", newline="\n")
    failures = verify(root)
    assert any("MAINTENANCE.md: no version stamp found" in f for f in failures)


# End-marker check on the committed file
def test_committed_end_marker_drift_detected(tmp_path):
    """Regression: an appended line on the committed drdebits.md is end-marker drift."""
    root = make_repo(tmp_path)
    sync(root)
    guide = root / "drdebits.md"
    guide.write_text(guide.read_text(encoding="utf-8") + "extra line\n", encoding="utf-8", newline="\n")
    failures = verify(root)
    assert any("end marker:" in f for f in failures)


# Checksum rebuild must not crash
def test_missing_checksum_file_becomes_message_not_crash(tmp_path):
    """Regression: a missing checksum_files member produces a message, not a crash."""
    root = make_repo(tmp_path)
    sync(root)
    (root / "README.md").unlink()  # README is in checksum_files
    failures = verify(root)
    # Should have messages, not raise; check that we got failures without exception
    assert isinstance(failures, list)
    assert any("README.md: missing" in f for f in failures)


# The guide version lives in more places than the stamped files; a version
# bump that misses one must not verify clean while disagreeing with itself.

def test_header_version_line_mismatch_detected(tmp_path):
    """Check (a): drdebits.md's own header version line must match guide_version."""
    root = make_repo(tmp_path)
    header = root / "src" / "guide" / "000-header.md"
    header.write_text(
        header.read_text(encoding="utf-8").replace("0.9.9-test", "0.9.8-test"),
        encoding="utf-8", newline="\n")
    sync(root)  # rebuild so the committed guide matches the (stale) src output
    failures = verify(root)
    assert any("drdebits.md: header version line does not match guide_version" in f for f in failures)


def test_release_tag_mismatch_detected(tmp_path):
    """Check (b): metadata release_tag must equal 'v' + guide_version."""
    root = make_repo(tmp_path)
    meta = root / "src" / "data" / "metadata.yaml"
    meta.write_text(
        meta.read_text(encoding="utf-8").replace('value: "v0.9.9-test"', 'value: "v9.9.9-test"'),
        encoding="utf-8", newline="\n")
    sync(root)
    failures = verify(root)
    assert any("metadata: release_tag v9.9.9-test != v0.9.9-test" in f for f in failures)


def test_guide_end_marker_metadata_mismatch_detected(tmp_path):
    """Check (c): metadata guide_end_marker must equal 'DRDEBITS-END-' + release_tag."""
    root = make_repo(tmp_path)
    meta = root / "src" / "data" / "metadata.yaml"
    meta.write_text(
        meta.read_text(encoding="utf-8").replace(
            'value: "DRDEBITS-END-v0.9.9-test"', 'value: "DRDEBITS-END-BOGUS"'),
        encoding="utf-8", newline="\n")
    sync(root)
    failures = verify(root)
    assert any(
        "metadata: guide_end_marker DRDEBITS-END-BOGUS != DRDEBITS-END-v0.9.9-test" in f
        for f in failures)


def test_missing_required_metadata_key_becomes_message_not_keyerror(tmp_path):
    """Deleting a load-bearing metadata row must surface as a sources finding,
    not a KeyError escaping run_verify."""
    root = make_repo(tmp_path)
    sync(root)
    meta = root / "src" / "data" / "metadata.yaml"
    text = meta.read_text(encoding="utf-8")
    start = text.index("  - key: guide_version")
    end = text.index("  - key: release_tag")
    meta.write_text(text[:start] + text[end:], encoding="utf-8", newline="\n")
    failures = verify(root)
    assert any("sources:" in f and "guide_version" in f for f in failures)


def test_citation_version_and_date_drift_detected(tmp_path):
    """Check (f): CITATION.cff's version must match guide_version and its
    date-released the newest changelog row; quoted YAML scalars are accepted."""
    root = make_repo(tmp_path)
    sync(root)
    citation = root / "CITATION.cff"
    citation.write_text(
        'cff-version: 1.2.0\nversion: 0.9.9-test\ndate-released: "2026-01-01"\n',
        encoding="utf-8", newline="\n")
    assert verify(root) == []
    citation.write_text(
        'cff-version: 1.2.0\nversion: "0.9.9-test"\ndate-released: 2026-01-01\n',
        encoding="utf-8", newline="\n")
    assert verify(root) == []  # quoting style is not load-bearing
    citation.write_text(
        'cff-version: 1.2.0\nversion: 0.1.0-stale\ndate-released: "2026-01-01"\n',
        encoding="utf-8", newline="\n")
    assert any("CITATION.cff: version" in f for f in verify(root))
    citation.write_text(
        'cff-version: 1.2.0\nversion: 0.9.9-test\ndate-released: "1999-01-01"\n',
        encoding="utf-8", newline="\n")
    assert any("CITATION.cff: date-released" in f for f in verify(root))


def test_malformed_sources_checked_at_becomes_message_not_traceback(tmp_path):
    """A bad date value must surface as a loader finding, not a ValueError
    escaping the catalogue-header builder."""
    root = make_repo(tmp_path)
    sync(root)
    meta = root / "src" / "data" / "metadata.yaml"
    meta.write_text(
        meta.read_text(encoding="utf-8").replace(
            'value: "2026-01-01T00:00:00+10:00"', 'value: "checked mid August"'),
        encoding="utf-8", newline="\n")
    failures = verify(root)
    assert any("sources_checked_at" in f for f in failures)


def test_source_check_date_drift_detected(tmp_path):
    """Check (e): a metadata date bump that misses the guide header line and
    README must fail verify instead of shipping two different check dates."""
    root = make_repo(tmp_path)
    meta = root / "src" / "data" / "metadata.yaml"
    meta.write_text(
        meta.read_text(encoding="utf-8").replace(
            'value: "2026-01-01T00:00:00+10:00"', 'value: "2026-02-02T00:00:00+10:00"'),
        encoding="utf-8", newline="\n")
    sync(root)  # rebuild so byte-compare passes; only the hand-written copies are stale
    failures = verify(root)
    assert any("drdebits.md: header source-check date" in f for f in failures)
    assert any("README.md: source-check date" in f for f in failures)


def test_review_due_not_after_source_check_date_detected(tmp_path):
    """Check (g): advancing sources_checked_at past review_due - exactly what a
    source recheck under MAINTENANCE step 8 does - must fail verify instead of
    publishing a guide whose own frontmatter tells the model its contents are
    already stale."""
    root = make_repo(tmp_path)
    meta = root / "src" / "data" / "metadata.yaml"
    original = meta.read_text(encoding="utf-8")
    meta.write_text(
        original.replace(
            'value: "2026-01-01T00:00:00+10:00"', 'value: "2026-06-01T00:00:00+10:00"'),
        encoding="utf-8", newline="\n")
    sync(root)  # rebuild so byte-compare passes; only the dates disagree
    failures = verify(root)
    assert any(
        "metadata: review_due 2026-04-01 is not after sources_checked_at 2026-06-01" in f
        for f in failures)

    # Same day is not "after": a review that falls due on the check date is
    # already due when the release ships.
    meta.write_text(
        original.replace('value: "2026-04-01"', 'value: "2026-01-01"'),
        encoding="utf-8", newline="\n")
    sync(root)
    assert any("review_due 2026-01-01 is not after" in f for f in verify(root))

    meta.write_text(
        original.replace('value: "2026-04-01"', 'value: "next quarter"'),
        encoding="utf-8", newline="\n")
    sync(root)
    assert any("review_due must be an ISO date" in f for f in verify(root))


def test_review_due_already_passed_at_verification_detected(tmp_path):
    """Check (g): a review_due that is after sources_checked_at but has already
    arrived when verify runs must fail too. src/guide/040-source-status.md
    tells the model to label the affected material SOURCE CURRENCY NOT
    CONFIRMED once review_due has passed, and MAINTENANCE step 6 runs verify
    before the release goes out, so comparing the two metadata dates with each
    other and nothing else blesses exactly the expired release this gate
    exists to stop."""
    root = make_repo(tmp_path)
    sync(root)  # fixture: sources checked 2026-01-01, review due 2026-04-01

    # Still current the day before it falls due.
    assert verify(root, today=date(2026, 3, 31)) == []

    # Due today is already due: the release ships needing a review it has not
    # had, exactly as "not after sources_checked_at" already treats same-day.
    assert any(
        "metadata: review_due 2026-04-01 is not after the verification date 2026-04-01"
        in f for f in verify(root, today=date(2026, 4, 1)))

    # And months past due, with both metadata dates still agreeing with each
    # other - the case the source-check comparison alone cannot see.
    failures = verify(root, today=date(2026, 9, 15))
    assert any(
        "metadata: review_due 2026-04-01 is not after the verification date 2026-09-15"
        in f for f in failures)
    assert any("SOURCE CURRENCY NOT CONFIRMED" in f for f in failures)
    assert not any("is not after sources_checked_at" in f for f in failures)


def test_missing_review_due_becomes_message_not_silent_pass(tmp_path):
    """Deleting review_due must surface as a sources finding: the guide still
    instructs the model to evaluate a frontmatter field that would no longer
    exist."""
    root = make_repo(tmp_path)
    sync(root)
    meta = root / "src" / "data" / "metadata.yaml"
    text = meta.read_text(encoding="utf-8")
    start = text.index("  - key: review_due")
    end = text.index("  - key: checksum_files")
    meta.write_text(text[:start] + text[end:], encoding="utf-8", newline="\n")
    failures = verify(root)
    assert any("sources:" in f and "review_due" in f for f in failures)


def test_tpb_statement_count_and_range_drift_detected(tmp_path):
    """Check (h): the guide's and README's hand-written statement count and GS
    range are not derived by the build, so a catalogue change that leaves them
    behind must fail verify instead of shipping a guide that contradicts its
    own bundled catalogue."""
    root = make_repo(tmp_path)
    sync(root)
    assert verify(root) == []

    # Add a statement the way a source recheck would, bumping the counts
    # validate_counts forces, but leaving the prose copies stale.
    catalogue = root / "src" / "data" / "tpb-catalogue.yaml"
    catalogue.write_text(
        catalogue.read_text(encoding="utf-8")
        + '  - id: "GS02"\n    title: "T2"\n    url: "https://x.invalid/b"\n'
          '    trigger: "g"\n',
        encoding="utf-8", newline="\n")
    meta = root / "src" / "data" / "metadata.yaml"
    meta.write_text(
        meta.read_text(encoding="utf-8").replace('value: "1"', 'value: "2"'),
        encoding="utf-8", newline="\n")
    sync(root)  # rebuild so byte-compare passes; only the hand-written copies are stale
    failures = verify(root)
    assert any("drdebits.md: statement range 'GS01 to GS01'" in f for f in failures)
    assert any("drdebits.md: statement count 1 does not match" in f for f in failures)
    assert any("README.md: statement range 'GS01 to GS01'" in f for f in failures)


def test_tpb_copy_that_stops_matching_is_a_failure_not_a_silent_pass(tmp_path):
    """Check (h) must not pass vacuously. Comparing only the text the patterns
    match makes the check blind to a copy that has been deleted or reworded out
    of them, so a stale claim written with an en dash, with "through", or with
    a different count phrase would ship unexamined beside a catalogue that
    contradicts it. Each rewrite below leaves the file syntactically fine and
    the rebuild byte-identical; only the copy count betrays it."""
    sources = "src/guide/020-sources.md"
    cases = (
        # An en dash instead of "to": the range still reads as a range to a
        # human, and to the pattern it is not there at all.
        (sources, "1 indexed statements, GS01 to GS01",
         "1 indexed statements, GS01–GS01",
         "drdebits.md: found 2 statement range copies, expected 3"),
        # "through" instead of "to".
        (sources, "catalogue, GS01 to GS01", "catalogue, GS01 through GS01",
         "drdebits.md: found 2 statement range copies, expected 3"),
        # A reworded count phrase.
        (sources, "exposed 1 live Guidance Statements", "exposed a total of 1 statements",
         "drdebits.md: found 1 statement count copies, expected 2"),
        # A copy deleted outright.
        (sources, "- reference/tpb-catalogue.md: complete live TPB Guidance "
                  "Statement catalogue, GS01 to GS01\n", "",
         "drdebits.md: found 2 statement range copies, expected 3"),
        # And the same blindness in README.
        ("README.md", "catalogue, GS01 to GS01", "catalogue, GS01–GS01",
         "README.md: found 0 statement range copies, expected 1"),
    )
    for case, (rel, old, new, expected) in enumerate(cases):
        root = make_repo(tmp_path / f"case-{case}")
        p = root / rel
        text = p.read_text(encoding="utf-8")
        assert old in text
        p.write_text(text.replace(old, new, 1), encoding="utf-8", newline="\n")
        sync(root)  # the rebuild follows the edit, so byte parity still holds
        failures = verify(root)
        assert any(expected in f for f in failures), (expected, failures)
        assert any("MAINTENANCE step 8" in f for f in failures)


def test_changelog_newest_entry_mismatch_detected(tmp_path):
    """Check (d): the newest (first) changelog entry must match guide_version."""
    root = make_repo(tmp_path)
    changelog = root / "src" / "data" / "changelog.yaml"
    changelog.write_text(
        changelog.read_text(encoding="utf-8").replace('version: "0.9.9-test"', 'version: "0.9.8-test"'),
        encoding="utf-8", newline="\n")
    sync(root)
    failures = verify(root)
    assert any("changelog: newest entry 0.9.8-test != 0.9.9-test" in f for f in failures)


def test_apes_locator_cited_by_the_control_set_but_absent_from_the_map(tmp_path):
    """Check (i): the control set routes a reader to the generated APES 110
    reference map for the provisions it names. Both files are sources, so a
    control set that cites a Part or section the map never locates rebuilds,
    hashes and byte-compares clean; only a cross-reference catches it."""
    root = make_repo(tmp_path)
    sync(root)
    assert verify(root) == []

    control_set = root / "src" / "guide" / "150-apes-110-control-set.md"
    control_set.write_text(
        control_set.read_text(encoding="utf-8")
        + "\nApply sections 260 and 360 and Part 4A to the engagement.\n",
        encoding="utf-8", newline="\n")
    sync(root)  # rebuild, so only the cross-reference can fail
    failures = verify(root)
    assert any("cites APES 110 section 260, which" in f for f in failures), failures
    assert any("cites APES 110 section 360, which" in f for f in failures), failures
    assert any("cites APES 110 Part 4A, which" in f for f in failures), failures

    # The locator the map does carry is not reported, and a paragraph
    # reference is not a Part or section citation.
    assert not any("Part 1," in f for f in failures), failures
    assert not any("R1.2" in f for f in failures), failures


def test_missing_apes_control_set_fragment_is_a_failure_not_a_silent_pass(tmp_path):
    """A cross-reference check with nothing to read must not pass. Renaming
    the fragment would otherwise retire check (i) without a word."""
    root = make_repo(tmp_path)
    sync(root)
    (root / "src" / "guide" / "150-apes-110-control-set.md").rename(
        root / "src" / "guide" / "150-apes.md")
    sync(root)
    failures = verify(root)
    assert any("150-apes-110-control-set.md: missing" in f for f in failures), failures


def test_behaviour_test_status_the_guide_does_not_define(tmp_path):
    """Check (j): the guide's Meaning of instruction words fragment defines
    every decision status and outcome label. A behaviour test written against
    one the guide never defines cannot be run against an implementation of the
    guide, and the generated behaviour-tests.md would publish it anyway."""
    root = make_repo(tmp_path)
    sync(root)
    assert verify(root) == []

    words = root / "src" / "guide" / "060-meaning-of-instruction-words.md"
    words.write_text("## Words\n\n- **ESCALATE** means refer the matter.\n",
                     encoding="utf-8", newline="\n")
    sync(root)  # rebuild, so only the cross-reference can fail
    failures = verify(root)
    assert failures == [
        "behaviour test A-001: expected status 'HARD_STOP' is not defined in "
        "src/guide/060-meaning-of-instruction-words.md"]


def test_missing_instruction_words_fragment_is_a_failure_not_a_silent_pass(tmp_path):
    root = make_repo(tmp_path)
    sync(root)
    (root / "src" / "guide" / "060-meaning-of-instruction-words.md").rename(
        root / "src" / "guide" / "060-words.md")
    sync(root)
    failures = verify(root)
    assert any("060-meaning-of-instruction-words.md: missing" in f
               for f in failures), failures
