"""Tests for YAML source loading and validation."""
import textwrap
import pytest
from drdebits_build.model import (
    ModelError, ALLOWED_STATUSES, load_metadata, load_catalogue,
    load_behaviour_tests, load_changelog, load_apes_map, load_vendor_assurance,
    validate_counts, validate_required_metadata,
)


def write(tmp_path, name, content):
    p = tmp_path / name
    p.write_text(textwrap.dedent(content), encoding="utf-8", newline="\n")
    return p


def test_metadata_preserves_order_and_strings(tmp_path):
    p = write(tmp_path, "metadata.yaml", """\
        fields:
          - key: title
            value: "DrDebits"
          - key: sources_checked_at
            value: "2026-08-16T00:00:00+10:00"
    """)
    meta = load_metadata(p)
    assert list(meta.keys()) == ["title", "sources_checked_at"]
    assert meta["sources_checked_at"] == "2026-08-16T00:00:00+10:00"
    assert all(isinstance(v, str) for v in meta.values())


def test_catalogue_rejects_duplicate_ids_and_http(tmp_path):
    good = """\
        entries:
          - id: "GS01"
            title: "T1"
            url: "https://example.invalid/a"
            trigger: "x"
    """
    rows = load_catalogue(write(tmp_path, "c1.yaml", good))
    assert rows[0]["id"] == "GS01"
    dup = good + """\
          - id: "GS01"
            title: "T2"
            url: "https://example.invalid/b"
            trigger: "y"
    """
    with pytest.raises(ModelError):
        load_catalogue(write(tmp_path, "c2.yaml", dup))
    with pytest.raises(ModelError):
        load_catalogue(write(tmp_path, "c3.yaml", good.replace("https://", "http://")))


def test_catalogue_ids_must_be_gs_numbered_and_ascending(tmp_path):
    """The catalogue header and verify both publish the first and last rows as
    the authoritative "GSxx to GSyy" range. Uniqueness alone does not make that
    claim true: a malformed or reordered catalogue would let both agree on an
    inverted or nonsensical range, so the ids must be GS-numbered and in
    ascending order at load."""
    def catalogue(name, ids):
        rows = "".join(
            f'  - id: "{i}"\n    title: "T"\n'
            f'    url: "https://example.invalid/{n}"\n    trigger: "x"\n'
            for n, i in enumerate(ids))
        return write(tmp_path, name, "entries:\n" + rows)

    assert [r["id"] for r in load_catalogue(catalogue("ok.yaml", ["GS01", "GS02"]))] \
        == ["GS01", "GS02"]
    # Withdrawn statements leave gaps; a gap is still ascending.
    assert len(load_catalogue(catalogue("gap.yaml", ["GS01", "GS09", "GS54"]))) == 3

    for name, ids, match in (
        ("desc.yaml", ["GS02", "GS01"], "does not come after"),
        ("swap.yaml", ["GS55", "GS02", "GS01"], "does not come after"),
        ("dupnum.yaml", ["GS01", "GS1"], "does not come after"),
        ("bad.yaml", ["GS01", "GS02a"], "Guidance Statement id"),
        ("nope.yaml", ["GS01", "TPB03"], "Guidance Statement id"),
        ("bare.yaml", ["01", "02"], "Guidance Statement id"),
    ):
        with pytest.raises(ModelError, match=match):
            load_catalogue(catalogue(name, ids))


def test_behaviour_status_enum_and_empty_fields(tmp_path):
    tmpl = """\
        entries:
          - id: "AUTH-001"
            scenario: "s"
            expected_status: "{status}"
            required_behaviour: "r"
            side_effect_check: "c"
    """
    ok = load_behaviour_tests(write(tmp_path, "b1.yaml", tmpl.format(status="HARD_STOP")))
    assert ok[0]["expected_status"] == "HARD_STOP"
    assert "Low impact — proportionate answer" in ALLOWED_STATUSES
    with pytest.raises(ModelError):
        load_behaviour_tests(write(tmp_path, "b2.yaml", tmpl.format(status="MAYBE")))
    with pytest.raises(ModelError):
        load_behaviour_tests(write(tmp_path, "b3.yaml", tmpl.format(status="HARD_STOP").replace('"s"', '""')))


def test_counts_must_match(tmp_path):
    meta = {"tpb_guidance_statement_count": "2", "tpb_library_index_count": "2"}
    cat = [{"id": "a", "title": "t", "url": "https://x.invalid", "trigger": "g"}]
    with pytest.raises(ModelError):
        validate_counts(meta, cat)
    validate_counts(meta, cat + [{"id": "b", "title": "t", "url": "https://y.invalid", "trigger": "g"}])


def test_missing_source_file_becomes_model_error(tmp_path):
    with pytest.raises(ModelError, match="cannot read source file"):
        load_metadata(tmp_path / "does-not-exist.yaml")


def test_unhashable_yaml_key_becomes_a_finding_not_a_traceback(tmp_path):
    """A complex key constructs to a list, which `key in mapping` cannot test.
    PyYAML's own constructor raises ConstructorError for it, and the duplicate
    check has to keep doing so: _read converts only YAMLError and OSError, and
    run_verify returns messages rather than raising, so a TypeError here would
    surface as a traceback instead of a source-validation finding."""
    with pytest.raises(ModelError, match="unhashable key"):
        load_metadata(write(tmp_path, "u.yaml", """\
            ? [complex, key]
            : value
        """))


def test_duplicate_yaml_keys_are_refused_at_the_read_boundary(tmp_path):
    """PyYAML keeps the last of a repeated key and drops the earlier one in
    silence. These files are the authority for generated controls, so a
    second `question:` in one entry would otherwise build, hash and verify
    clean while publishing only the final text: rebuild-and-compare cannot
    see a control that never reached the output. Every loader shares the read
    boundary, so the guarantee has to sit there."""
    with pytest.raises(ModelError, match="duplicate key"):
        load_metadata(write(tmp_path, "m.yaml", """\
            fields:
              - key: title
                value: "DrDebits"
                value: "Something else"
        """))
    with pytest.raises(ModelError, match="duplicate key"):
        load_vendor_assurance(write(tmp_path, "v.yaml", """\
            sections:
              - "Authority"
            entries:
              - id: "VA-01"
                section: "Authority"
                question: "the control as written"
                question: "the control as replaced"
                evidence: "e"
                obligation: "o"
                if_absent: "a"
        """))
    with pytest.raises(ModelError, match="duplicate key"):
        load_catalogue(write(tmp_path, "c.yaml", """\
            entries:
              - id: "GS01"
                title: "T"
                url: "https://x.invalid/a"
                trigger: "g"
            entries:
              - id: "GS02"
                title: "T2"
                url: "https://x.invalid/b"
                trigger: "h"
        """))


def test_apes_map_missing_key_names_the_key(tmp_path):
    p = write(tmp_path, "am.yaml", """\
        retrieval_points:
          - label: "Scope"
            value: "R1.2"
    """)
    with pytest.raises(ModelError, match="'contexts'"):
        load_apes_map(p)


def test_table_bound_values_reject_pipes_and_newlines(tmp_path):
    tmpl = """\
        entries:
          - id: "GS01"
            title: "T"
            url: "https://x.invalid/a"
            trigger: {trigger}
    """
    with pytest.raises(ModelError, match="break the rendered Markdown table"):
        load_catalogue(write(tmp_path, "p.yaml", tmpl.format(trigger='"either A | B"')))
    with pytest.raises(ModelError, match="contains a newline"):
        load_catalogue(write(tmp_path, "n.yaml", tmpl.format(trigger='"a\\nb"')))
    with pytest.raises(ModelError, match="contains a newline"):
        load_catalogue(write(tmp_path, "r.yaml", tmpl.format(trigger='"a\\rb"')))
    # Metadata keeps the '|' exemption (checksum_files uses it as a separator)
    # but newlines stay banned everywhere - one would inject an extra
    # frontmatter line into the guide.
    m = write(tmp_path, "m.yaml", """\
        fields:
          - key: checksum_files
            value: "LICENSE|README.md"
    """)
    assert load_metadata(m)["checksum_files"] == "LICENSE|README.md"
    with pytest.raises(ModelError, match="contains a newline"):
        load_metadata(write(tmp_path, "mi.yaml", """\
            fields:
              - key: jurisdiction
                value: "AU\\nstatus: injected"
        """))


def test_required_metadata_keys_enforced():
    meta = {k: "x" for k in (
        "guide_version", "release_tag", "guide_end_marker", "sources_checked_at",
        "review_due", "tpb_guidance_statement_count", "tpb_library_index_count",
        "checksum_files")}
    validate_required_metadata(meta)
    del meta["guide_version"]
    with pytest.raises(ModelError, match="guide_version"):
        validate_required_metadata(meta)


def test_changelog_and_apes_map_shapes(tmp_path):
    cl = load_changelog(write(tmp_path, "cl.yaml", """\
        entries:
          - version: "0.2.0-draft"
            date: "2026-08-16"
            status: "Published draft"
            change: "words"
    """))
    assert cl[0]["version"] == "0.2.0-draft"
    am = load_apes_map(write(tmp_path, "am.yaml", """\
        contexts:
          - label: "All members"
            value: "Part 1"
        retrieval_points:
          - label: "Scope"
            value: "R1.2"
    """))
    assert am["contexts"][0]["label"] == "All members"


VENDOR_SOURCE = """\
    sections:
      - "Authority"
      - "Privacy"
    entries:
      - id: "VA-01"
        section: "Authority"
        question: "q1"
        evidence: "e1"
        obligation: "o1"
        if_absent: "a1"
      - id: "VA-02"
        section: "Authority"
        question: "q2"
        evidence: "e2"
        obligation: "o2"
        if_absent: "a2"
      - id: "VA-03"
        section: "Privacy"
        question: "q3"
        evidence: "e3"
        obligation: "o3"
        if_absent: "a3"
"""


def test_vendor_assurance_keeps_sections_and_row_order(tmp_path):
    out = load_vendor_assurance(write(tmp_path, "v.yaml", VENDOR_SOURCE))
    assert out["sections"] == ["Authority", "Privacy"]
    assert [r["id"] for r in out["entries"]] == ["VA-01", "VA-02", "VA-03"]
    assert out["entries"][2]["section"] == "Privacy"


def test_vendor_assurance_ids_must_be_va_numbered_and_ascending(tmp_path):
    """A firm records an answer against an id. Ascending, VA-numbered ids make
    a duplicated or reordered row a load failure rather than a silent
    renumbering of what somebody already answered."""
    with pytest.raises(ModelError, match="checklist id"):
        load_vendor_assurance(write(tmp_path, "b.yaml", VENDOR_SOURCE.replace(
            'id: "VA-02"', 'id: "Q2"', 1)))
    with pytest.raises(ModelError, match="ascending id order"):
        load_vendor_assurance(write(tmp_path, "d.yaml", VENDOR_SOURCE.replace(
            'id: "VA-03"', 'id: "VA-00"', 1)))


def test_vendor_assurance_sections_must_be_declared_contiguous_and_used(tmp_path):
    """Each declared section renders as one heading and one table whose rows
    are selected by the section field. An undeclared section would drop its
    rows from the file, a resumed section would move rows under a heading that
    does not describe them, and an unused section would render as a heading
    with no table."""
    with pytest.raises(ModelError, match="not declared"):
        load_vendor_assurance(write(tmp_path, "u.yaml", VENDOR_SOURCE.replace(
            'section: "Privacy"', 'section: "Security"', 1)))
    interleaved = VENDOR_SOURCE.replace(
        'section: "Authority"\n        question: "q2"',
        'section: "Privacy"\n        question: "q2"', 1).replace(
        'section: "Privacy"\n        question: "q3"',
        'section: "Authority"\n        question: "q3"', 1)
    with pytest.raises(ModelError, match="contiguous"):
        load_vendor_assurance(write(tmp_path, "i.yaml", interleaved))
    with pytest.raises(ModelError, match="sections without entries"):
        load_vendor_assurance(write(tmp_path, "e.yaml", VENDOR_SOURCE.replace(
            'section: "Privacy"', 'section: "Authority"', 1)))
    with pytest.raises(ModelError, match="duplicate section titles"):
        load_vendor_assurance(write(tmp_path, "s.yaml", VENDOR_SOURCE.replace(
            '- "Privacy"', '- "Authority"', 1)))
    with pytest.raises(ModelError, match="contains a newline"):
        load_vendor_assurance(write(tmp_path, "n.yaml", VENDOR_SOURCE.replace(
            '- "Privacy"', '- "Privacy\\n## Injected"', 1)))
