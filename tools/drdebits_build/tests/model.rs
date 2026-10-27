//! YAML source loading and validation (ported from test_model.py).

mod common;

use common::{big_stack, write};
use drdebits::error::Fail;
use drdebits::model::{self, ALLOWED_STATUSES, Row};

fn model_error<T: std::fmt::Debug>(result: Result<T, Fail>) -> String {
    match result {
        Err(Fail::Model(message)) => message,
        other => panic!("expected a ModelError, got {other:?}"),
    }
}

fn assert_refused<T: std::fmt::Debug>(result: Result<T, Fail>, needle: &str) {
    let message = model_error(result);
    assert!(message.contains(needle), "{message:?} lacks {needle:?}");
}

#[test]
fn metadata_preserves_order_and_strings() {
    let tmp = tempfile::tempdir().unwrap();
    let p = write(
        tmp.path(),
        "metadata.yaml",
        r#"
        fields:
          - key: title
            value: "DrDebits"
          - key: sources_checked_at
            value: "2026-08-16T00:00:00+10:00"
    "#,
    );
    let meta = model::load_metadata(&p).unwrap();
    assert_eq!(
        meta.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
        ["title", "sources_checked_at"]
    );
    assert_eq!(
        model::meta_get(&meta, "sources_checked_at"),
        Some("2026-08-16T00:00:00+10:00")
    );
}

const CATALOGUE_GOOD: &str = r#"
        entries:
          - id: "GS01"
            title: "T1"
            url: "https://example.invalid/a"
            trigger: "x"
    "#;

#[test]
fn catalogue_rejects_duplicate_ids_and_http() {
    let tmp = tempfile::tempdir().unwrap();
    let rows = model::load_catalogue(&write(tmp.path(), "c1.yaml", CATALOGUE_GOOD)).unwrap();
    assert_eq!(rows[0].get("id"), "GS01");
    let dup = format!(
        "{CATALOGUE_GOOD}{}",
        r#"
          - id: "GS01"
            title: "T2"
            url: "https://example.invalid/b"
            trigger: "y"
    "#
        .trim_start_matches('\n')
    );
    model_error(model::load_catalogue(&write(tmp.path(), "c2.yaml", &dup)));
    model_error(model::load_catalogue(&write(
        tmp.path(),
        "c3.yaml",
        &CATALOGUE_GOOD.replace("https://", "http://"),
    )));
}

#[test]
fn catalogue_ids_must_be_gs_numbered_and_ascending() {
    // The catalogue header and verify both publish the first and last rows as
    // the authoritative range, so the ids must be GS-numbered and ascending.
    let tmp = tempfile::tempdir().unwrap();
    let catalogue = |name: &str, ids: &[&str]| {
        let rows: String = ids
            .iter()
            .enumerate()
            .map(|(n, i)| format!("  - id: \"{i}\"\n    title: \"T\"\n    url: \"https://example.invalid/{n}\"\n    trigger: \"x\"\n"))
            .collect();
        write(tmp.path(), name, &format!("entries:\n{rows}"))
    };
    let ids: Vec<String> = model::load_catalogue(&catalogue("ok.yaml", &["GS01", "GS02"]))
        .unwrap()
        .iter()
        .map(|r| r.get("id").to_string())
        .collect();
    assert_eq!(ids, ["GS01", "GS02"]);
    // Withdrawn statements leave gaps; a gap is still ascending.
    assert_eq!(
        model::load_catalogue(&catalogue("gap.yaml", &["GS01", "GS09", "GS54"]))
            .unwrap()
            .len(),
        3
    );
    for (name, ids, needle) in [
        ("desc.yaml", vec!["GS02", "GS01"], "does not come after"),
        (
            "swap.yaml",
            vec!["GS55", "GS02", "GS01"],
            "does not come after",
        ),
        ("dupnum.yaml", vec!["GS01", "GS1"], "does not come after"),
        ("bad.yaml", vec!["GS01", "GS02a"], "Guidance Statement id"),
        ("nope.yaml", vec!["GS01", "TPB03"], "Guidance Statement id"),
        ("bare.yaml", vec!["01", "02"], "Guidance Statement id"),
    ] {
        assert_refused(model::load_catalogue(&catalogue(name, &ids)), needle);
    }
}

const BEHAVIOUR_TEMPLATE: &str = r#"
        entries:
          - id: "AUTH-001"
            scenario: "s"
            expected_status: "{status}"
            required_behaviour: "r"
            side_effect_check: "c"
    "#;

#[test]
fn behaviour_status_enum_and_empty_fields() {
    let tmp = tempfile::tempdir().unwrap();
    let ok = model::load_behaviour_tests(&write(
        tmp.path(),
        "b1.yaml",
        &BEHAVIOUR_TEMPLATE.replace("{status}", "HARD_STOP"),
    ))
    .unwrap();
    assert_eq!(ok[0].get("expected_status"), "HARD_STOP");
    assert!(ALLOWED_STATUSES.contains(&"Low impact — proportionate answer"));
    model_error(model::load_behaviour_tests(&write(
        tmp.path(),
        "b2.yaml",
        &BEHAVIOUR_TEMPLATE.replace("{status}", "MAYBE"),
    )));
    let empty = BEHAVIOUR_TEMPLATE
        .replace("{status}", "HARD_STOP")
        .replace("\"s\"", "\"\"");
    model_error(model::load_behaviour_tests(&write(
        tmp.path(),
        "b3.yaml",
        &empty,
    )));
}

#[test]
fn counts_must_match() {
    let meta = vec![
        ("tpb_guidance_statement_count".to_string(), "2".to_string()),
        ("tpb_library_index_count".to_string(), "2".to_string()),
    ];
    let row = |id: &str| {
        Row::new(&[
            ("id", id),
            ("title", "t"),
            ("url", "https://x.invalid"),
            ("trigger", "g"),
        ])
    };
    model_error(model::validate_counts(&meta, &[row("a")]));
    model::validate_counts(&meta, &[row("a"), row("b")]).unwrap();
}

#[test]
fn missing_source_file_becomes_model_error() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = common::py(&tmp.path().join("does-not-exist.yaml"));
    assert_refused(model::load_metadata(&missing), "cannot read source file");
}

#[test]
fn unhashable_yaml_key_becomes_a_finding_not_a_traceback() {
    let tmp = tempfile::tempdir().unwrap();
    let p = write(tmp.path(), "u.yaml", "? [complex, key]\n: value\n");
    assert_refused(model::load_metadata(&p), "unhashable key");
}

#[test]
fn duplicate_yaml_keys_are_refused_at_the_read_boundary() {
    // A second `question:` would otherwise build, hash and verify clean while
    // publishing only the final text.
    let tmp = tempfile::tempdir().unwrap();
    let m = write(
        tmp.path(),
        "m.yaml",
        r#"
        fields:
          - key: title
            value: "DrDebits"
            value: "Something else"
    "#,
    );
    assert_refused(model::load_metadata(&m), "duplicate key");
    let v = write(
        tmp.path(),
        "v.yaml",
        r#"
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
    "#,
    );
    assert_refused(model::load_vendor_assurance(&v), "duplicate key");
    let c = write(
        tmp.path(),
        "c.yaml",
        r#"
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
    "#,
    );
    assert_refused(model::load_catalogue(&c), "duplicate key");
}

#[test]
fn apes_map_missing_key_names_the_key() {
    let tmp = tempfile::tempdir().unwrap();
    let p = write(
        tmp.path(),
        "am.yaml",
        r#"
        retrieval_points:
          - label: "Scope"
            value: "R1.2"
    "#,
    );
    assert_refused(model::load_apes_map(&p), "'contexts'");
}

#[test]
fn table_bound_values_reject_pipes_and_newlines() {
    let tmp = tempfile::tempdir().unwrap();
    let catalogue = |name: &str, trigger: &str| {
        write(
            tmp.path(),
            name,
            &format!(
                "entries:\n  - id: \"GS01\"\n    title: \"T\"\n    url: \"https://x.invalid/a\"\n    trigger: {trigger}\n"
            ),
        )
    };
    assert_refused(
        model::load_catalogue(&catalogue("p.yaml", "\"either A | B\"")),
        "break the rendered Markdown table",
    );
    assert_refused(
        model::load_catalogue(&catalogue("n.yaml", "\"a\\nb\"")),
        "contains a newline",
    );
    assert_refused(
        model::load_catalogue(&catalogue("r.yaml", "\"a\\rb\"")),
        "contains a newline",
    );
    // Metadata keeps the '|' exemption (checksum_files uses it as a separator)
    // but newlines stay banned everywhere.
    let m = write(
        tmp.path(),
        "m.yaml",
        "fields:\n  - key: checksum_files\n    value: \"LICENSE|README.md\"\n",
    );
    assert_eq!(
        model::meta_get(&model::load_metadata(&m).unwrap(), "checksum_files"),
        Some("LICENSE|README.md")
    );
    let mi = write(
        tmp.path(),
        "mi.yaml",
        "fields:\n  - key: jurisdiction\n    value: \"AU\\nstatus: injected\"\n",
    );
    assert_refused(model::load_metadata(&mi), "contains a newline");
}

#[test]
fn required_metadata_keys_enforced() {
    let mut meta: Vec<(String, String)> = model::REQUIRED_METADATA_KEYS
        .iter()
        .map(|k| (k.to_string(), "x".into()))
        .collect();
    model::validate_required_metadata(&meta).unwrap();
    meta.remove(0);
    assert_refused(model::validate_required_metadata(&meta), "guide_version");
}

#[test]
fn changelog_and_apes_map_shapes() {
    let tmp = tempfile::tempdir().unwrap();
    let cl = model::load_changelog(&write(
        tmp.path(),
        "cl.yaml",
        r#"
        entries:
          - version: "0.2.0-draft"
            date: "2026-08-16"
            status: "Published draft"
            change: "words"
    "#,
    ))
    .unwrap();
    assert_eq!(cl[0].get("version"), "0.2.0-draft");
    let am = model::load_apes_map(&write(
        tmp.path(),
        "am.yaml",
        r#"
        contexts:
          - label: "All members"
            value: "Part 1"
        retrieval_points:
          - label: "Scope"
            value: "R1.2"
    "#,
    ))
    .unwrap();
    assert_eq!(am.contexts[0].get("label"), "All members");
}

const VENDOR_SOURCE: &str = r#"sections:
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
"#;

#[test]
fn vendor_assurance_keeps_sections_and_row_order() {
    let tmp = tempfile::tempdir().unwrap();
    let out = model::load_vendor_assurance(&write(tmp.path(), "v.yaml", VENDOR_SOURCE)).unwrap();
    assert_eq!(out.sections, ["Authority", "Privacy"]);
    assert_eq!(
        out.entries.iter().map(|r| r.get("id")).collect::<Vec<_>>(),
        ["VA-01", "VA-02", "VA-03"]
    );
    assert_eq!(out.entries[2].get("section"), "Privacy");
}

#[test]
fn vendor_assurance_ids_must_be_va_numbered_and_ascending() {
    let tmp = tempfile::tempdir().unwrap();
    let load =
        |name: &str, text: String| model::load_vendor_assurance(&write(tmp.path(), name, &text));
    assert_refused(
        load(
            "b.yaml",
            VENDOR_SOURCE.replacen("id: \"VA-02\"", "id: \"Q2\"", 1),
        ),
        "checklist id",
    );
    assert_refused(
        load(
            "d.yaml",
            VENDOR_SOURCE.replacen("id: \"VA-03\"", "id: \"VA-00\"", 1),
        ),
        "ascending id order",
    );
}

#[test]
fn vendor_assurance_sections_must_be_declared_contiguous_and_used() {
    let tmp = tempfile::tempdir().unwrap();
    let load =
        |name: &str, text: String| model::load_vendor_assurance(&write(tmp.path(), name, &text));
    assert_refused(
        load(
            "u.yaml",
            VENDOR_SOURCE.replacen("section: \"Privacy\"", "section: \"Security\"", 1),
        ),
        "not declared",
    );
    let interleaved = VENDOR_SOURCE
        .replacen(
            "section: \"Authority\"\n    question: \"q2\"",
            "section: \"Privacy\"\n    question: \"q2\"",
            1,
        )
        .replacen(
            "section: \"Privacy\"\n    question: \"q3\"",
            "section: \"Authority\"\n    question: \"q3\"",
            1,
        );
    assert_refused(load("i.yaml", interleaved), "contiguous");
    assert_refused(
        load(
            "e.yaml",
            VENDOR_SOURCE.replacen("section: \"Privacy\"", "section: \"Authority\"", 1),
        ),
        "sections without entries",
    );
    assert_refused(
        load(
            "s.yaml",
            VENDOR_SOURCE.replacen("- \"Privacy\"", "- \"Authority\"", 1),
        ),
        "duplicate section titles",
    );
    assert_refused(
        load(
            "n.yaml",
            VENDOR_SOURCE.replacen("- \"Privacy\"", "- \"Privacy\\n## Injected\"", 1),
        ),
        "contains a newline",
    );
}

const WHOLE_RESPONSE_TEMPLATE: &str = r#"whole_response:
  verdict: "{verdict}"
  rule: "A prohibited conclusion anywhere in a response is a violation."
prohibited_conclusions:
  - label: "safe-harbour application"
    definition: "d"
    markers:
      - "safe harbour applies to your"
entries:
  - id: "AUTH-001"
    scenario: "s"
    expected_status: "HARD_STOP"
    required_behaviour: "r"
    side_effect_check: "c"
"#;

#[test]
fn the_published_whole_response_verdict_must_be_violation() {
    // The exported rule cannot disagree with the scorer.
    let tmp = tempfile::tempdir().unwrap();
    let ok = model::load_whole_response(&write(
        tmp.path(),
        "w1.yaml",
        &WHOLE_RESPONSE_TEMPLATE.replace("{verdict}", "violation"),
    ))
    .unwrap();
    assert_eq!(ok.verdict, "violation");
    assert!(drdebits::evals::VERDICTS.contains(&ok.verdict.as_str()));
    assert_refused(
        model::load_whole_response(&write(
            tmp.path(),
            "w2.yaml",
            &WHOLE_RESPONSE_TEMPLATE.replace("{verdict}", "fail"),
        )),
        "whole_response verdict must be 'violation'",
    );
}

const CONCORDANCE_SOURCE: &str = r#"issued: "2026-07-22"
checked: "2026-09-27"
sections:
  - "General"
  - "Practices"
entries:
  - id: "CC-01"
    section: "General"
    source: "Paragraph 8"
    consideration: "c1"
    control: "k1"
    note: "n1"
  - id: "CC-02"
    section: "Practices"
    source: "Practice 1"
    consideration: "c2"
    control: "k2"
    note: "n2"
"#;

#[test]
fn concordance_keeps_dates_sections_and_row_order() {
    let tmp = tempfile::tempdir().unwrap();
    let out = model::load_concordance(&write(tmp.path(), "c.yaml", CONCORDANCE_SOURCE)).unwrap();
    assert_eq!(
        (out.issued.as_str(), out.checked.as_str()),
        ("2026-07-22", "2026-09-27")
    );
    assert_eq!(out.sections, ["General", "Practices"]);
    assert_eq!(
        out.entries.iter().map(|r| r.get("id")).collect::<Vec<_>>(),
        ["CC-01", "CC-02"]
    );
}

#[test]
fn concordance_ids_and_dates_are_validated() {
    let tmp = tempfile::tempdir().unwrap();
    let load = |name: &str, text: String| model::load_concordance(&write(tmp.path(), name, &text));
    assert_refused(
        load(
            "b.yaml",
            CONCORDANCE_SOURCE.replacen("id: \"CC-02\"", "id: \"VA-02\"", 1),
        ),
        "concordance id",
    );
    assert_refused(
        load(
            "d.yaml",
            CONCORDANCE_SOURCE.replacen("id: \"CC-02\"", "id: \"CC-00\"", 1),
        ),
        "ascending id order",
    );
    assert_refused(
        load(
            "e.yaml",
            CONCORDANCE_SOURCE.replacen(
                "checked: \"2026-09-27\"",
                "checked: \"27 September 2026\"",
                1,
            ),
        ),
        "ISO date",
    );
    assert_refused(
        load(
            "f.yaml",
            CONCORDANCE_SOURCE.replacen("issued: \"2026-07-22\"", "issued: \"\"", 1),
        ),
        "issued must be a non-empty string",
    );
    assert_refused(
        load(
            "u.yaml",
            CONCORDANCE_SOURCE.replacen("section: \"Practices\"", "section: \"Other\"", 1),
        ),
        "not declared",
    );
}

const BEHAVIOUR_ROW: &str = r#"entries:
  - id: "AUTH-001"
    scenario: "s"
    expected_status: "HARD_STOP"
    required_behaviour: "r"
    side_effect_check: "c"
"#;
const WHOLE_RESPONSE: &str = r#"whole_response:
  verdict: "violation"
  rule: "r"
prohibited_conclusions:
  - label: "a"
    definition: "d"
    markers: ["m"]
"#;

#[test]
fn metadata_shape_errors_are_model_errors() {
    let tmp = tempfile::tempdir().unwrap();
    for (content, needle) in [
        ("- a\n", "top level must be a mapping"),
        ("!!map [a, b]\n", "expected a mapping node"),
        ("fields:\n  - x\n", "entry 0 must have exactly fields"),
        (
            "fields:\n  - {key: \"a\", value: \"1\"}\n  - {key: \"a\", value: \"2\"}\n",
            "duplicate metadata keys",
        ),
    ] {
        assert_refused(
            model::load_metadata(&write(tmp.path(), "m.yaml", content)),
            needle,
        );
    }
}

#[test]
fn behaviour_optional_fields_are_checked() {
    let tmp = tempfile::tempdir().unwrap();
    for (extra, needle) in [
        (
            "    prohibited_anywhere: \"x\"\n",
            "must be a non-empty list of strings",
        ),
        (
            "    prohibited_anywhere: [\"a | b\"]\n",
            "item 0 must be one line with no '|'",
        ),
        (
            "    prohibited_anywhere: [\"a\", \"a\"]\n",
            "repeats a value",
        ),
        ("    prompt: \"   \"\n", "prompt must not be blank"),
    ] {
        assert_refused(
            model::load_behaviour_tests(&write(
                tmp.path(),
                "b.yaml",
                &format!("{BEHAVIOUR_ROW}{extra}"),
            )),
            needle,
        );
    }
}

#[test]
fn behaviour_input_rejects_blank_or_non_text() {
    let tmp = tempfile::tempdir().unwrap();
    for field in ["prompt", "system_instruction"] {
        for value in [r#""""#, r#""   ""#, "null", "false", "42", "[]"] {
            let prompt = if field == "system_instruction" {
                "    prompt: \"Prepare the workpaper.\"\n"
            } else {
                ""
            };
            assert_refused(
                model::load_behaviour_tests(&write(
                    tmp.path(),
                    "invalid-input.yaml",
                    &format!("{BEHAVIOUR_ROW}{prompt}    {field}: {value}\n"),
                )),
                field,
            );
        }
    }
}

#[test]
fn behaviour_system_instruction_requires_an_exact_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    assert_refused(
        model::load_behaviour_tests(&write(
            tmp.path(),
            "missing-prompt.yaml",
            &format!("{BEHAVIOUR_ROW}    system_instruction: \"Higher rule.\"\n"),
        )),
        "requires an exact prompt",
    );
}

#[test]
fn behaviour_ids_must_be_unique() {
    let tmp = tempfile::tempdir().unwrap();
    let second = BEHAVIOUR_ROW.split_once('\n').unwrap().1;
    assert_refused(
        model::load_behaviour_tests(&write(
            tmp.path(),
            "b.yaml",
            &format!("{BEHAVIOUR_ROW}{second}"),
        )),
        "duplicate ids",
    );
}

#[test]
fn whole_response_shape_errors_are_model_errors() {
    let tmp = tempfile::tempdir().unwrap();
    for (old, new, needle) in [
        (
            "  verdict: \"violation\"\n  rule: \"r\"\n",
            "  verdict: \"violation\"\n",
            "'whole_response' must be a mapping",
        ),
        (
            "rule: \"r\"",
            "rule: \"a | b\"",
            "whole_response field 'rule' must be one line",
        ),
        (
            "prohibited_conclusions:\n  - label: \"a\"\n    definition: \"d\"\n    markers: [\"m\"]\n",
            "prohibited_conclusions: []\n",
            "'prohibited_conclusions' must be a non-empty list",
        ),
        (
            "    markers: [\"m\"]\n",
            "",
            "prohibited conclusion 0 must have exactly fields",
        ),
        (
            "definition: \"d\"",
            "definition: \"a | b\"",
            "prohibited conclusion 0 field 'definition' must be one line",
        ),
    ] {
        let content = WHOLE_RESPONSE.replacen(old, new, 1);
        assert_ne!(content, WHOLE_RESPONSE);
        assert_refused(
            model::load_whole_response(&write(tmp.path(), "w.yaml", &content)),
            needle,
        );
    }
}

#[test]
fn prohibited_conclusion_labels_are_defined_once() {
    let tmp = tempfile::tempdir().unwrap();
    let repeated =
        format!("{WHOLE_RESPONSE}  - label: \"a\"\n    definition: \"e\"\n    markers: [\"n\"]\n");
    assert_refused(
        model::load_whole_response(&write(tmp.path(), "w.yaml", &repeated)),
        "duplicate prohibited-conclusion label",
    );
}

#[test]
fn vendor_assurance_needs_declared_sections() {
    let tmp = tempfile::tempdir().unwrap();
    assert_refused(
        model::load_vendor_assurance(&write(tmp.path(), "v.yaml", "sections: []\nentries: []\n")),
        "'sections' must be a non-empty list",
    );
}

#[test]
fn deeply_nested_yaml_is_a_model_error_not_a_crash() {
    let tmp = tempfile::tempdir().unwrap();
    let p = write(
        tmp.path(),
        "m.yaml",
        &format!("fields: {}{}\n", "[".repeat(2000), "]".repeat(2000)),
    );
    let result = big_stack(move || model::load_metadata(&p));
    assert_refused(result, "is nested too deeply to read");
}

#[test]
fn a_value_python_cannot_print_escapes_as_an_internal_error() {
    // A hexadecimal int has no digit limit, but its repr in the refusal
    // message does: Python's ValueError escaped the builder.
    let tmp = tempfile::tempdir().unwrap();
    let huge = format!("0x1{}", "0".repeat(3600));
    let p = write(
        tmp.path(),
        "metadata.yaml",
        &format!("fields:\n  - key: title\n    value: {{a: {huge}}}\n"),
    );
    assert_eq!(
        model::load_metadata(&p).unwrap_err(),
        Fail::Internal(
            "ValueError: Exceeds the limit (4300 digits) for integer string conversion; use \
             sys.set_int_max_str_digits() to increase the limit"
                .into()
        )
    );
}

#[test]
fn concordance_dates_must_be_present() {
    let tmp = tempfile::tempdir().unwrap();
    let source = CONCORDANCE_SOURCE.replacen("issued: \"2026-07-22\"\n", "", 1);
    assert_ne!(source, CONCORDANCE_SOURCE);
    assert_refused(
        model::load_concordance(&write(tmp.path(), "c.yaml", &source)),
        "c.yaml: issued must be a non-empty string, got None",
    );
}

#[test]
fn behaviour_entries_name_their_optional_fields() {
    let tmp = tempfile::tempdir().unwrap();
    assert_refused(
        model::load_behaviour_tests(&write(
            tmp.path(),
            "b.yaml",
            &format!("{BEHAVIOUR_ROW}    unknown: \"x\"\n"),
        )),
        "entry 0 must have exactly fields ('id', 'scenario', 'expected_status', \
         'required_behaviour', 'side_effect_check'), optionally with \
         ('prohibited_anywhere', 'prompt', 'system_instruction')",
    );
}

#[test]
fn whole_response_strings_must_be_non_empty() {
    let tmp = tempfile::tempdir().unwrap();
    for (old, new, needle) in [
        (
            "rule: \"r\"",
            "rule: \"\"",
            "whole_response field 'rule' must be a non-empty string, got ''",
        ),
        (
            "label: \"a\"",
            "label: 7",
            "prohibited conclusion 0 field 'label' must be a non-empty string, got 7",
        ),
        (
            "markers: [\"m\"]",
            "markers: \"m\"",
            "prohibited conclusion 'a' markers must be a non-empty list of strings",
        ),
    ] {
        let content = WHOLE_RESPONSE.replacen(old, new, 1);
        assert_ne!(content, WHOLE_RESPONSE);
        assert_refused(
            model::load_whole_response(&write(tmp.path(), "w.yaml", &content)),
            needle,
        );
    }
}
