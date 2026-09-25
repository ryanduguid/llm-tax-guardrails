"""Load and validate the YAML sources of truth. All leaf values are strings."""
from __future__ import annotations

import re
from collections.abc import Hashable
from datetime import date

import yaml

# A catalogue id is a TPB Guidance Statement number. Both the generated
# catalogue header and verify's range check publish the first and last rows as
# the authoritative "GSxx to GSyy" range, so the rows have to be what that
# claim assumes: real GS ids in ascending order. Without this, a reordered or
# malformed catalogue rebuilds and verifies clean while publishing an inverted
# or nonsensical range, because the builder and the verifier read the same 2
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
    """SafeConstructor's mapping construction, less the silent overwrite.

    Everything but the duplicate check mirrors PyYAML, deliberately: merge
    keys are flattened first, and an unhashable key (a sequence or mapping
    written as a complex key) raises ConstructorError. Dropping that guard
    would let a `TypeError` out of `key in mapping` instead, which `_read`
    does not convert and `run_verify` cannot report, so a malformed source
    would produce a traceback where the contract promises a finding.

    A merge that overrides a key it imported now fails as a duplicate rather
    than resolving silently. No source uses anchors, and a file that is the
    authority for a generated control should say what it means once.
    """
    if isinstance(node, yaml.nodes.MappingNode):
        loader.flatten_mapping(node)
    mapping = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node, deep=deep)
        if not isinstance(key, Hashable):
            raise yaml.constructor.ConstructorError(
                "while constructing a mapping", node.start_mark,
                "found unhashable key", key_node.start_mark)
        if key in mapping:
            raise yaml.constructor.ConstructorError(
                "while constructing a mapping", node.start_mark,
                f"found duplicate key {key!r}", key_node.start_mark)
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
    except UnicodeDecodeError as e:
        # A path-qualified ModelError, not a traceback: every other read failure
        # here already reports the file it could not read.
        raise ModelError(f"{path}: is not valid UTF-8 ({e})") from e
    except OSError as e:
        raise ModelError(f"{path}: cannot read source file ({e})") from e
    if not isinstance(data, dict):
        raise ModelError(f"{path}: top level must be a mapping")
    return data


def _require_str(path, where, value):
    if not isinstance(value, str) or value == "":
        raise ModelError(f"{path}: {where} must be a non-empty string, got {value!r}")
    return value


def _rows(path, data, key, fields, table_safe=False, optional=()):
    """Load a list of string-valued rows.

    ``optional`` names fields a row may add. Their values are returned
    untouched, because an optional field need not be a string: the caller that
    declared it validates its own shape.
    """
    entries = data.get(key)
    if not isinstance(entries, list) or not entries:
        raise ModelError(f"{path}: '{key}' must be a non-empty list")
    out = []
    for i, row in enumerate(entries):
        allowed = set(fields) | set(optional)
        if not isinstance(row, dict) or not set(fields) <= set(row) <= allowed:
            raise ModelError(
                f"{path}: entry {i} must have exactly fields {fields}"
                + (f", optionally with {tuple(optional)}" if optional else ""))
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
        for f in optional:
            if f in row:
                clean[f] = row[f]
        out.append(clean)
    return out


def _string_list(path, where, value):
    """A non-empty list of distinct one-line strings, pipe-free."""
    if not isinstance(value, list) or not value:
        raise ModelError(f"{path}: {where} must be a non-empty list of strings")
    out = []
    for i, item in enumerate(value):
        item = _require_str(path, f"{where} item {i}", item)
        if any(ch in item for ch in "\r\n|"):
            raise ModelError(f"{path}: {where} item {i} must be one line with no '|'")
        out.append(item)
    if len(out) != len(set(out)):
        raise ModelError(f"{path}: {where} repeats a value")
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


BEHAVIOUR_FIELDS = ("id", "scenario", "expected_status", "required_behaviour",
                    "side_effect_check")
#: The optional per-case field emphasising the prohibited conclusions that case
#: turns on. Every declared conclusion makes a response a `violation` wherever
#: in it it appears, so this field narrows nothing. The labels are defined once
#: under `prohibited_conclusions`; verify holds every value here to that list.
PROHIBITED_ANYWHERE = "prohibited_anywhere"
WHOLE_RESPONSE_FIELDS = ("verdict", "rule")
#: The only verdict the whole-response rule can carry. The scorer records a
#: whole-response breach as `violation` and the results table counts only
#: `pass`, so a source that published any other verdict would export a rule the
#: harness and the table both contradict.
WHOLE_RESPONSE_VERDICT = "violation"
PROHIBITED_CONCLUSION_FIELDS = ("label", "definition", "markers")


def load_behaviour_tests(path):
    rows = _rows(path, _read(path), "entries", BEHAVIOUR_FIELDS, table_safe=True,
                 optional=(PROHIBITED_ANYWHERE, "prompt"))
    _unique_ids(path, rows)
    for r in rows:
        if r["expected_status"] not in ALLOWED_STATUSES:
            raise ModelError(f"{path}: {r['id']} has unknown status {r['expected_status']!r}")
        if "prompt" in r:
            _require_str(path, f"{r['id']} prompt", r["prompt"])
            if not r["prompt"].strip():
                raise ModelError(f"{path}: {r['id']} prompt must not be blank")
        if PROHIBITED_ANYWHERE in r:
            r[PROHIBITED_ANYWHERE] = _string_list(
                path, f"{r['id']} {PROHIBITED_ANYWHERE}", r[PROHIBITED_ANYWHERE])
    return rows


def load_whole_response(path):
    """Load the whole-response rule and the prohibited-conclusion labels.

    The rule is machine-readable on purpose: a runner reads it from
    `evals/cases.json` rather than inferring it from prose. It is loaded
    separately from the cases so that the case loader keeps its own contract,
    and each label is defined exactly once here, which is what makes a
    per-case `prohibited_anywhere` value checkable.
    """
    data = _read(path)
    rule = data.get("whole_response")
    if not isinstance(rule, dict) or set(rule) != set(WHOLE_RESPONSE_FIELDS):
        raise ModelError(
            f"{path}: 'whole_response' must be a mapping with exactly fields "
            f"{WHOLE_RESPONSE_FIELDS}")
    out = {}
    for field in WHOLE_RESPONSE_FIELDS:
        value = _require_str(path, f"whole_response field {field!r}", rule[field])
        if any(ch in value for ch in "\r\n|"):
            raise ModelError(f"{path}: whole_response field {field!r} must be one line with no '|'")
        out[field] = value
    if out["verdict"] != WHOLE_RESPONSE_VERDICT:
        raise ModelError(
            f"{path}: whole_response verdict must be {WHOLE_RESPONSE_VERDICT!r}, got "
            f"{out['verdict']!r}; the exported rule cannot disagree with the scorer")

    entries = data.get("prohibited_conclusions")
    if not isinstance(entries, list) or not entries:
        raise ModelError(f"{path}: 'prohibited_conclusions' must be a non-empty list")
    conclusions = []
    for i, row in enumerate(entries):
        if not isinstance(row, dict) or set(row) != set(PROHIBITED_CONCLUSION_FIELDS):
            raise ModelError(
                f"{path}: prohibited conclusion {i} must have exactly fields "
                f"{PROHIBITED_CONCLUSION_FIELDS}")
        clean = {}
        for field in ("label", "definition"):
            value = _require_str(path, f"prohibited conclusion {i} field {field!r}", row[field])
            if any(ch in value for ch in "\r\n|"):
                raise ModelError(
                    f"{path}: prohibited conclusion {i} field {field!r} must be one line "
                    "with no '|'")
            clean[field] = value
        clean["markers"] = _string_list(
            path, f"prohibited conclusion {clean['label']!r} markers", row["markers"])
        conclusions.append(clean)
    labels = [r["label"] for r in conclusions]
    if len(labels) != len(set(labels)):
        raise ModelError(
            f"{path}: duplicate prohibited-conclusion label; each label is defined once")
    out["prohibited_conclusions"] = conclusions
    return out


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
    the checks below make the 2 agree at load: every row names a declared
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
    order: list[str] = []
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
