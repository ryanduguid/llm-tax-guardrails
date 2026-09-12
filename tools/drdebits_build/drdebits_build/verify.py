"""Rebuild-and-compare verification. Returns messages instead of raising."""
from __future__ import annotations

import re
from datetime import date
from pathlib import Path

from . import evals
from .build import (
    GENERATED,
    SOURCE_CHECK_DATE_FILES,
    STAMP_RE,
    STAMPED_FILES,
    BuildError,
    build_sha256sums,
    load_sources,
)
from .model import ModelError

# The two shapes the hand-written TPB statement count and GS range are written
# in outside the generated catalogue header: a bare id range, and a count
# introducing one. Check (h) compares every match against the sources.
GS_RANGE_RE = re.compile(r"GS\d+ to GS\d+")
GS_COUNT_RE = re.compile(r"(\d+) (?:indexed statements|live Guidance Statements)")

# How many copies of each shape each file must carry. Comparing only the text
# the patterns above happen to match lets check (h) pass vacuously: a copy that
# is deleted, or reworded past those patterns - "GS01-GS55" with an en dash,
# "GS01 through GS55", a different count phrase - simply drops out of the scan,
# and the stale claim ships unexamined beside a catalogue that contradicts it.
# Requiring the documented number of copies makes an absent copy a failure
# rather than silence. The copies are the ones MAINTENANCE step 8 tells a
# maintainer to update: drdebits.md carries the source-status table row, the
# source-status prose sentence and the workpaper-record catalogue line; README
# carries its bundled-file table row; the banner and badge copies it once
# carried went with the 0.3.3 README opener.
GS_COPY_COUNTS = {
    "drdebits.md": {"range": 3, "count": 2},
    "README.md": {"range": 1, "count": 0},
}

# The two guide fragments checks (i) and (j) cross-reference the data files
# against. Named, not discovered: a renamed or deleted fragment must surface as
# a finding, because a cross-reference check that quietly has nothing to read
# passes on every tree including a broken one.
APES_CONTROL_SET_FRAGMENT = "150-apes-110-control-set.md"
INSTRUCTION_WORDS_FRAGMENT = "060-meaning-of-instruction-words.md"

# An APES 110 Part or section locator in prose: "Part 4B", "Parts 1 to 4B",
# "section 280", "sections 310, 320 and 321", "sections 5400 to 5600". The
# leading \b keeps "Subsections" out of the sections arm, which the optional
# "Sub" then matches on its own so a subsection citation still counts as one.
# Paragraph references (`1.5`, `R220.8`, `5400.3a`) and the TAA's "s 284-15"
# deliberately do not match: the reference map locates Parts and sections, and
# those are what the two hand-written files have to agree about.
APES_LOCATOR_RE = re.compile(
    r"\b(?:(?P<part>[Pp]arts?)|(?:[Ss]ub)?[Ss]ections?)\s+"
    r"(?P<numbers>\d+[A-Za-z]?(?:\s*(?:,|and|to|or)\s*\d+[A-Za-z]?)*)")
_LOCATOR_NUMBER_RE = re.compile(r"\d+[A-Za-z]?")

# A defined instruction word or outcome label: the bold run in one bullet of
# the Meaning of instruction words fragment.
INSTRUCTION_WORD_RE = re.compile(r"\*\*(.+?)\*\*")


def _apes_locators(text):
    """The APES 110 Parts and sections a passage cites, as ("Part"|"section", id)."""
    found = set()
    for m in APES_LOCATOR_RE.finditer(text):
        kind = "Part" if m.group("part") else "section"
        for number in _LOCATOR_NUMBER_RE.findall(m.group("numbers")):
            found.add((kind, number))
    return found


def _verification_date():
    """The date verify treats as today.

    Indirected through a function so a caller - the test suite in particular -
    can pin it. Reading the wall clock inside the check itself would make every
    assertion about review_due expire on the date it asserts about.
    """
    return date.today()


def run_verify(root, today=None):
    root = Path(root)
    if today is None:
        today = _verification_date()
    try:
        s = load_sources(root)
    except (ModelError, BuildError, OSError) as exc:
        return [f"sources: {exc}"]

    failures: list[str] = []
    built = {rel: fn(s) for rel, fn in GENERATED.items()}

    # A missing checksum_files member must surface as a finding, not a crash.
    try:
        built["SHA256SUMS"] = build_sha256sums(root, s, built)
    except (ModelError, BuildError, OSError) as exc:
        failures.append(f"SHA256SUMS: cannot rebuild ({exc})")
        # SHA256SUMS drops out of the parity check below; all other entries still checked

    # A malformed result file is a finding too, not a crash.
    try:
        built[evals.RESULTS_FILE] = evals.build_results_md(root, s)
    except (ModelError, BuildError, OSError) as exc:
        failures.append(f"{evals.RESULTS_FILE}: cannot rebuild ({exc})")

    for rel, content in built.items():
        committed = root / rel
        if not committed.is_file():
            failures.append(f"{rel}: missing committed file")
        elif committed.read_bytes() != content.encode("utf-8"):
            failures.append(f"{rel}: committed file differs from build output; edit src/ and run build")

    # The committed guide is read once here and reused by the checks below; a
    # missing file is already reported above, so those checks simply skip.
    committed_guide = root / "drdebits.md"
    committed_text = committed_guide.read_text(encoding="utf-8") if committed_guide.is_file() else None

    # The end marker is checked on the committed guide, not the rebuild, so a
    # truncated commit is caught.
    if committed_text is not None:
        last = committed_text.rstrip("\n").rsplit("\n", 1)[-1]
        if last != s.meta["guide_end_marker"]:
            failures.append(f"end marker: last line {last!r} != {s.meta['guide_end_marker']!r}")

    # Stamped files must exist and carry at least one version token each;
    # zero matches would let a stripped stamp verify vacuously.
    for rel in STAMPED_FILES:
        p = root / rel
        if not p.is_file():
            failures.append(f"{rel}: missing")
        else:
            text = p.read_text(encoding="utf-8")
            matches = list(STAMP_RE.finditer(text))
            if not matches:
                failures.append(f"{rel}: no version stamp found")
            else:
                for m in matches:
                    if m.group(0) != s.meta["guide_version"]:
                        failures.append(f"{rel}: version {m.group(0)} != {s.meta['guide_version']}")

    # The guide version lives in several places beyond the stamped files
    # above. Each must agree with meta["guide_version"] (the single source of
    # truth) or with each other, so a bump that misses one spot is caught here
    # instead of shipping a self-contradictory release.

    # (a) the committed drdebits.md header must carry the exact version line
    if committed_text is not None:
        version_line = "> Version: `" + s.meta["guide_version"] + "`"
        if version_line not in committed_text.splitlines():
            failures.append("drdebits.md: header version line does not match guide_version")

    # (b) release_tag must be "v" + guide_version
    if s.meta["release_tag"] != "v" + s.meta["guide_version"]:
        failures.append(
            f"metadata: release_tag {s.meta['release_tag']} != v{s.meta['guide_version']}")

    # (c) guide_end_marker must be "DRDEBITS-END-" + release_tag
    if s.meta["guide_end_marker"] != "DRDEBITS-END-" + s.meta["release_tag"]:
        failures.append(
            f"metadata: guide_end_marker {s.meta['guide_end_marker']} != "
            f"DRDEBITS-END-{s.meta['release_tag']}")

    # (d) the newest (first) changelog entry must match guide_version
    newest_changelog_version = s.changelog[0]["version"]
    if newest_changelog_version != s.meta["guide_version"]:
        failures.append(
            f"changelog: newest entry {newest_changelog_version} != {s.meta['guide_version']}")

    # (e) the source-check date is derived into the catalogue header and the
    # guide frontmatter from metadata, but the guide's header line, README and
    # llms.txt carry hand-written copies. A metadata bump that misses one would
    # ship a release carrying two different check dates, so cross-check them
    # all. (src/guide/040-source-status.md carries two further prose copies
    # that this check does not cover; MAINTENANCE step 8 owns those.)
    checked_date = s.meta.get("sources_checked_at", "")[:10]
    if checked_date:
        # Substring, not whole-line: the real header line carries a timezone
        # suffix after the backticked date.
        guide_line = f"> Sources last checked: `{checked_date}`"
        if committed_text is not None and guide_line not in committed_text:
            failures.append(
                "drdebits.md: header source-check date line does not match "
                f"sources_checked_at ({checked_date})")
        for rel in SOURCE_CHECK_DATE_FILES:
            p = root / rel
            if not p.is_file():
                continue  # already reported missing above
            if f"Sources last checked: {checked_date}" not in p.read_text(encoding="utf-8"):
                failures.append(
                    f"{rel}: source-check date does not match sources_checked_at "
                    f"({checked_date})")

    # (f) CITATION.cff carries its own copies of the version and release date.
    # When the file exists, both must agree with the sources - version with
    # guide_version, date-released with the newest changelog row - so a bump
    # cannot ship a stale citation record. Quoted and unquoted YAML scalars
    # are both accepted. A deliberately absent file is tolerated.
    citation = root / "CITATION.cff"
    if citation.is_file():
        text = citation.read_text(encoding="utf-8")
        version_m = re.search(r'^version:\s*"?([^"\r\n]+?)"?\s*$', text, re.MULTILINE)
        if not version_m or version_m.group(1) != s.meta["guide_version"]:
            failures.append(
                f"CITATION.cff: version does not match guide_version "
                f"{s.meta['guide_version']}")
        release_date = s.changelog[0]["date"]
        date_m = re.search(r'^date-released:\s*"?([0-9-]+)"?\s*$', text, re.MULTILINE)
        if not date_m or date_m.group(1) != release_date:
            failures.append(
                f"CITATION.cff: date-released does not match newest changelog "
                f"date {release_date}")

    # (g) review_due is the one metadata field the guide tells every consuming
    # LLM to act on: once it has passed, the guide labels its own contents
    # SOURCE CURRENCY NOT CONFIRMED. A source recheck that advances
    # sources_checked_at without advancing review_due would therefore publish a
    # release that disables itself on the day it ships, so review_due must be a
    # real date strictly after the source-check date - and, because MAINTENANCE
    # runs verify on the day of release, strictly after that day too. Checking
    # only the two metadata dates against each other would bless a release
    # whose review fell due months before anyone ran the gate.
    review_due = s.meta["review_due"]
    try:
        due = date.fromisoformat(review_due)
    except ValueError:
        failures.append(
            f"metadata: review_due must be an ISO date (YYYY-MM-DD), got {review_due!r}")
    else:
        checked = date.fromisoformat(s.meta["sources_checked_at"][:10])
        if due <= checked:
            failures.append(
                f"metadata: review_due {review_due} is not after sources_checked_at "
                f"{checked.isoformat()}; the release would ship already due for review")
        if due <= today:
            failures.append(
                f"metadata: review_due {review_due} is not after the verification date "
                f"{today.isoformat()}; the sources are due for review now, so the guide "
                "labels its own contents SOURCE CURRENCY NOT CONFIRMED")

    # (h) the catalogue header derives its statement count and GS range from
    # metadata and the catalogue rows, but the guide and README carry
    # hand-written copies of both. Adding or withdrawing a statement would
    # otherwise ship one release whose guide says one count and whose bundled
    # catalogue says another, so every copy must agree with the same two
    # sources the header is built from. Every occurrence is checked, not just
    # the first, so a stale leftover beside a corrected copy is still caught -
    # and the number of occurrences is checked too, so a copy that is deleted
    # or reworded out of the patterns cannot escape by simply not matching.
    # load_catalogue guarantees the rows are GS ids in ascending order, which
    # is what makes the first and last of them the range endpoints.
    gs_count = s.meta["tpb_guidance_statement_count"]
    gs_range = f"{s.catalogue[0]['id']} to {s.catalogue[-1]['id']}"
    for rel, expected in GS_COPY_COUNTS.items():
        p = root / rel
        if not p.is_file():
            continue  # already reported missing above
        text = p.read_text(encoding="utf-8")
        found = {"range": GS_RANGE_RE.findall(text), "count": GS_COUNT_RE.findall(text)}
        for value in found["range"]:
            if value != gs_range:
                failures.append(
                    f"{rel}: statement range {value!r} does not match the "
                    f"catalogue range {gs_range!r}")
        for value in found["count"]:
            if value != gs_count:
                failures.append(
                    f"{rel}: statement count {value} does not match "
                    f"tpb_guidance_statement_count {gs_count}")
        for shape, values in found.items():
            if len(values) != expected[shape]:
                failures.append(
                    f"{rel}: found {len(values)} statement {shape} copies, expected "
                    f"{expected[shape]}; a hand-written copy has been removed or "
                    "reworded past this check (MAINTENANCE step 8 lists them)")

    # Twenty guide fragments and six data files are written by hand against
    # each other, and the rebuild-and-compare check above cannot see a
    # disagreement between them: both sides are sources, so a contradiction
    # builds, hashes and verifies clean. Checks (i) and (j) make the two
    # cross-references the guide actually depends on into findings.
    fragments = dict(s.fragments)

    # (i) the control set routes a reader to reference/apes-110-map.md for the
    # APES 110 provisions it names, and that file is generated from
    # src/data/apes-110-map.yaml. Every Part and section the control set cites
    # must therefore be one the map locates, or the guide sends the reader to a
    # map that never names the provision. The check runs one way only: the map
    # deliberately locates provisions the control set covers by topic rather
    # than by number (section 220 preparing and presenting information, section
    # 350 client assets, Part 4A audit and review independence, and so on), so
    # requiring every row to be cited by number would be a claim about the
    # guide's drafting, not about the two files agreeing.
    control_set = fragments.get(APES_CONTROL_SET_FRAGMENT)
    if control_set is None:
        failures.append(
            f"src/guide/{APES_CONTROL_SET_FRAGMENT}: missing, so the APES 110 "
            "cross-reference cannot run")
    else:
        mapped = set()
        for row in s.apes["contexts"] + s.apes["retrieval_points"]:
            mapped |= _apes_locators(row["value"])
        for kind, number in sorted(_apes_locators(control_set) - mapped):
            failures.append(
                f"src/guide/{APES_CONTROL_SET_FRAGMENT}: cites APES 110 {kind} "
                f"{number}, which src/data/apes-110-map.yaml does not locate")

    # (j) every behaviour test is written against a decision status or outcome
    # label, and the guide is where those are defined. A test naming one the
    # guide never defines cannot be run against an implementation of the guide,
    # and the generated behaviour-tests.md would publish the orphan status as
    # though the guide supported it. model.ALLOWED_STATUSES pins the set the
    # loader accepts; this pins that set to the guide text itself.
    instruction_words = fragments.get(INSTRUCTION_WORDS_FRAGMENT)
    if instruction_words is None:
        failures.append(
            f"src/guide/{INSTRUCTION_WORDS_FRAGMENT}: missing, so the behaviour "
            "test cross-reference cannot run")
    else:
        defined = set(INSTRUCTION_WORD_RE.findall(instruction_words))
        for row in s.behaviour:
            if row["expected_status"] not in defined:
                failures.append(
                    f"behaviour test {row['id']}: expected status "
                    f"{row['expected_status']!r} is not defined in "
                    f"src/guide/{INSTRUCTION_WORDS_FRAGMENT}")

    return failures
