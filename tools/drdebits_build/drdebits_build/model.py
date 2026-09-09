"""Load and validate the YAML sources of truth. All leaf values are strings."""
from __future__ import annotations

import re
from datetime import date

import yaml

# A catalogue id is a TPB Guidance Statement number. Both the generated
# catalogue header and verify's range check publish the first and last rows as
# the authoritative "GSxx to GSyy" range, so the rows have to be what that
# claim assumes: real GS ids in ascending order. Without this, a reordered or
# malformed catalogue rebuilds and verifies clean while publishing an inverted
# or nonsensical range, because the builder and the verifier read the same two
# rows and therefore agree with each other about bad data.
CATALOGUE_ID_RE = re.compile(r"GS(\d+)\Z")

# A vendor assurance id. The checklist renders as one table per section and is
# cited by id, so the same invariants the catalogue needs apply here: numbered
# ids in ascending order, so a duplicated or reordered row is visible instead
# of silently renumbering what a firm recorded against last quarter.
VENDOR_ID_RE = re.compile(r"VA-(\d+)\Z")

ALLOWED_STATUSES = frozenset({
    "HARD_STOP", "ESCALATE", "NEEDS_FACTS", "PROCEED_DRAFT_ONLY",
    "Low impact — proportionate answer",
})


class ModelError(Exception):
    pass


class _UniqueKeyLoader(yaml.SafeLoader):
    """SafeLoader that refuses a repeated mapping key.

    PyYAML keeps the last of a repeated key and drops the earlier one without
    a word. These files are the authority for generated controls, so that
    silence is the whole problem: a second `question:` in one entry would
    build, hash and verify clean while publishing only the final text, and
    the rebuild-and-compare check cannot see a control that never reached the
    output. evals.py already refuses duplicate keys in a result file for the
    same reason; this extends the guarantee to every YAML source.

    Subclassing SafeLoader keeps the safe constructor set: the loader
    resolves the same tags safe_load does, and only mapping construction
    changes.
    """


def _construct_unique_mapping(loader, node, deep=False):
    mapping = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node, deep=deep)
        if key in mapping:
            raise yaml.constructor.ConstructorError(
                None, None, f"duplicate key {key!r}", key_node.start_mark)
        mapping[key] = loader.construct_object(value_node, deep=deep)
    return mapping


_UniqueKeyLoader.add_constructor(
    yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, _construct_unique_mapping)


def _read(path):
    try:
        with open(path, encoding="utf-8") as fh:
            loader = _UniqueKeyLoader(fh.read())
            try:
                data = loader.get_single_data()
            finally:
                loader.dispose()
    except yaml.YAMLError as e:
        raise ModelError(f"{path}: {e}") from e
    except OSError as e:
        raise ModelError(f"{path}: cannot read source file ({e})") from e
    if not isinstance(data, dict):
        raise ModelError(f"{path}: top level must be a mapping")
    return data


def _require_str(path, where, value):
    if not isinstance(value, str) or value == "":
        raise ModelError(f"{path}: {where} must be a non-empty string, got {value!r}")
    return value


def _rows(path, data, key, fields, table_safe=False):
    entries = data.get(key)
    if not isinstance(entries, list) or not entries:
        raise ModelError(f"{path}: '{key}' must be a non-empty list")
    out = []
    for i, row in enumerate(entries):
        if not isinstance(row, dict) or set(row) != set(fields):
            raise ModelError(f"{path}: entry {i} must have exactly fields {fields}")
        clean = {}
        for f in fields:
            value = _require_str(path, f"entry {i} field '{f}'", row[f])
            # No source value may contain a newline (either kind): a table row
            # would split across lines and a metadata value would inject extra
            # frontmatter lines into the guide - and verify would bless both,
            # since the rebuild breaks identically. Pipes are additionally
            # rejected in table-bound values only, because metadata's
            # checksum_files legitimately uses '|' as its separator.
            if "\n" in value or "\r" in value:
                raise ModelError(
                    f"{path}: entry {i} field '{f}' contains a newline")
            if table_safe and "|" in value:
                raise ModelError(
                    f"{path}: entry {i} field '{f}' contains '|', "
                    "which would break the rendered Markdown table")
            clean[f] = value
        out.append(clean)
    return out


def _unique_ids(path, rows):
    ids = [r["id"] for r in rows]
    if len(ids) != len(set(ids)):
        raise ModelError(f"{path}: duplicate ids")


def load_metadata(path):
    rows = _rows(path, _read(path), "fields", ("key", "value"))
    keys = [r["key"] for r in rows]
    if len(keys) != len(set(keys)):
        raise ModelError(f"{path}: duplicate metadata keys")
    meta = {r["key"]: r["value"] for r in rows}
    # The catalogue header derives prose from this value, so a malformed date
    # must surface here as a ModelError (a clean verify finding), not later as
    # a ValueError escaping the builders.
    checked = meta.get("sources_checked_at")
    if checked is not None:
        try:
            date.fromisoformat(checked[:10])
        except ValueError as e:
            raise ModelError(
                f"{path}: sources_checked_at must start with an ISO date (YYYY-MM-DD), "
                f"got {checked!r}") from e
    return meta


def load_catalogue(path):
    rows = _rows(path, _read(path), "entries", ("id", "title", "url", "trigger"),
                 table_safe=True)
    _unique_ids(path, rows)
    previous = None
    for r in rows:
        match = CATALOGUE_ID_RE.match(r["id"])
        if match is None:
            raise ModelError(
                f"{path}: id {r['id']!r} must be a Guidance Statement id "
                "(GS followed by digits)")
        number = int(match.group(1))
        if previous is not None and number <= previous[0]:
            raise ModelError(
                f"{path}: id {r['id']} does not come after {previous[1]}; entries must "
                "be in ascending id order, because the catalogue header and verify "
                "both publish the first and last rows as the statement range")
        previous = (number, r["id"])
        if not r["url"].startswith("https://"):
            raise ModelError(f"{path}: url for {r['id']} must be https")
    return rows


def load_behaviour_tests(path):
    rows = _rows(path, _read(path), "entries",
                 ("id", "scenario", "expected_status", "required_behaviour", "side_effect_check"),
                 table_safe=True)
    _unique_ids(path, rows)
    for r in rows:
        if r["expected_status"] not in ALLOWED_STATUSES:
            raise ModelError(f"{path}: {r['id']} has unknown status {r['expected_status']!r}")
    return rows


def load_changelog(path):
    return _rows(path, _read(path), "entries", ("version", "date", "status", "change"),
                 table_safe=True)


def load_apes_map(path):
    data = _read(path)
    out = {}
    for key in ("contexts", "retrieval_points"):
        out[key] = _rows(path, {key: data.get(key)}, key, ("label", "value"),
                         table_safe=True)
    return out


VENDOR_FIELDS = ("id", "section", "question", "evidence", "obligation", "if_absent")


def load_vendor_assurance(path):
    """Load the AI vendor assurance checklist: declared sections, then rows.

    The builder renders one heading and one table per declared section, in the
    declared order, and picks each table's rows by matching the section field.
    That render is only faithful if the rows carry the grouping it assumes, so
    the checks below make the two agree at load: every row names a declared
    section, a section's rows sit together, and every declared section has
    rows. Without them a row could drift under a heading that does not
    describe it, or a section could render as a bare heading with no table,
    and the rebuild would reproduce either one byte for byte.
    """
    data = _read(path)
    titles = data.get("sections")
    if not isinstance(titles, list) or not titles:
        raise ModelError(f"{path}: 'sections' must be a non-empty list")
    declared = []
    for i, title in enumerate(titles):
        title = _require_str(path, f"section {i}", title)
        # A heading is rendered from this value, so a newline would inject
        # document structure the sources never declared.
        if "\n" in title or "\r" in title:
            raise ModelError(f"{path}: section {i} contains a newline")
        declared.append(title)
    if len(declared) != len(set(declared)):
        raise ModelError(f"{path}: duplicate section titles")

    rows = _rows(path, data, "entries", VENDOR_FIELDS, table_safe=True)
    _unique_ids(path, rows)
    previous = None
    order = []
    for r in rows:
        match = VENDOR_ID_RE.match(r["id"])
        if match is None:
            raise ModelError(
                f"{path}: id {r['id']!r} must be a checklist id (VA- followed by digits)")
        number = int(match.group(1))
        if previous is not None and number <= previous[0]:
            raise ModelError(
                f"{path}: id {r['id']} does not come after {previous[1]}; entries must "
                "be in ascending id order so a firm's recorded reference keeps meaning "
                "the same row")
        previous = (number, r["id"])
        if r["section"] not in declared:
            raise ModelError(
                f"{path}: {r['id']} names section {r['section']!r}, which is not declared")
        if not order or order[-1] != r["section"]:
            if r["section"] in order:
                raise ModelError(
                    f"{path}: {r['id']} resumes section {r['section']!r} after another "
                    "section; a section's entries must be contiguous, because each "
                    "section renders as one table")
            order.append(r["section"])
    if order != declared:
        missing = [t for t in declared if t not in order]
        raise ModelError(
            f"{path}: sections with entries {order} do not match the declared order "
            f"{declared}; sections without entries: {missing}")
    return {"sections": declared, "entries": rows}


# Every key the builders and verifier dereference unconditionally. A missing
# key must surface as a ModelError finding at load, not a KeyError escaping
# run_verify's returns-messages contract.
REQUIRED_METADATA_KEYS = (
    "guide_version", "release_tag", "guide_end_marker", "sources_checked_at",
    "review_due", "tpb_guidance_statement_count", "tpb_library_index_count",
    "checksum_files",
)


def validate_required_metadata(metadata):
    missing = [k for k in REQUIRED_METADATA_KEYS if k not in metadata]
    if missing:
        raise ModelError(f"metadata: missing required keys {missing}")


def validate_counts(metadata, catalogue):
    n = str(len(catalogue))
    for field in ("tpb_guidance_statement_count", "tpb_library_index_count"):
        if metadata.get(field) != n:
            raise ModelError(f"metadata {field}={metadata.get(field)!r} but catalogue has {n} rows")
