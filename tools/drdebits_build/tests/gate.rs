//! The output-contract gate: contract fields, abbreviated records,
//! arithmetic reperformance, the conclusion screen and the exit codes.

mod common;

use common::*;
use drdebits::gate::{
    self, EXIT_BLOCK, EXIT_COULD_NOT_RUN, EXIT_PASS, EXIT_REVIEW, calculations, check, check_facts,
    load_facts,
};
use drdebits::model::Conclusion;

const DRAFT: &str = "DRAFT — PROFESSIONAL REVIEW REQUIRED

Primary decision status: PROCEED_DRAFT_ONLY
- Scope: GST on one fictional invoice; next human step: confirm the supply is taxable.
- action_authority: NONE.
- state_changing_tool_action: NONE

Scope and period
- One invoice dated 1 August 2026.

Source-version status
- Historical authority: CONFIRMED
- Current duties: SOURCE CURRENCY NOT CONFIRMED

Facts and evidence
- Invoice total $1,100 including GST.

Assumptions
- The supply is wholly taxable.

Applicable authority
- GST Act s 9-70.

Analysis
- $1,100 / 11 = $100.00

Ethical and regulatory flags
- None identified.

Conclusion and uncertainty
- GST of $100 on supplied facts.

Human actions before use
- Reviewer confirms the tax invoice.

External-action record
- No lodgement, payment or other external action occurred.
";

const STOP: &str = "DRAFT - PROFESSIONAL REVIEW REQUIRED
Primary decision status: HARD_STOP
- Concluding on the safe harbour is prohibited; the registered practitioner assesses it.
- action_authority: NONE
- state_changing_tool_action: NONE
External-action record
- No external action occurred.
";

fn conclusions() -> Vec<Conclusion> {
    vec![Conclusion {
        label: "safe-harbour application".into(),
        definition: "d".into(),
        markers: vec!["safe harbour applies to your".into()],
    }]
}

fn verdict(text: &str) -> (&'static str, u8) {
    check(text, &conclusions()).verdict()
}

#[test]
fn a_complete_draft_passes_and_reperforms_its_arithmetic() {
    let report = check(DRAFT, &conclusions());
    assert_eq!(report.verdict(), ("PASS", EXIT_PASS), "{:?}", report.blocks);
    assert_eq!(report.status.as_deref(), Some("PROCEED_DRAFT_ONLY"));
    assert!(report.notes.is_empty(), "{:?}", report.notes);
    assert_eq!(report.calculations.len(), 1);
    assert!(report.calculations[0].matches);
}

#[test]
fn a_wrong_figure_blocks_with_the_computed_value() {
    let report = check(&DRAFT.replace("= $100.00", "= $110.00"), &conclusions());
    assert_eq!(report.verdict(), ("BLOCK", EXIT_BLOCK));
    assert_eq!(
        report.blocks,
        ["calculation does not reperform: $1,100 / 11 = $110.00 (computes to 100.00)"]
    );
}

#[test]
fn each_missing_contract_element_blocks() {
    for (from, to, finding) in [
        (
            "DRAFT — PROFESSIONAL REVIEW REQUIRED",
            "Draft for review",
            "banner",
        ),
        (
            "action_authority: NONE.",
            "action_authority: SEND_EMAIL",
            "action_authority must be NONE",
        ),
        (
            "- state_changing_tool_action: NONE\n",
            "",
            "missing field: state_changing_tool_action",
        ),
        (
            "Current duties: SOURCE CURRENCY NOT CONFIRMED",
            "Current duties: probably fine",
            "Current duties must be",
        ),
        (
            "Assumptions\n",
            "Premises\n",
            "missing mandatory section: Assumptions",
        ),
        (
            "Primary decision status: PROCEED_DRAFT_ONLY",
            "Primary decision status: PROCEED",
            "no primary decision status",
        ),
    ] {
        let report = check(&DRAFT.replace(from, to), &conclusions());
        assert_eq!(report.verdict().1, EXIT_BLOCK, "{from}");
        assert!(
            report.blocks.iter().any(|b| b.contains(finding)),
            "{from}: {:?}",
            report.blocks
        );
    }
}

#[test]
fn two_decision_fields_block() {
    let text = DRAFT.replace(
        "Primary decision status: PROCEED_DRAFT_ONLY",
        "Primary decision status: PROCEED_DRAFT_ONLY\nPrimary decision status: NEEDS_FACTS",
    );
    let report = check(&text, &conclusions());
    assert!(
        report
            .blocks
            .iter()
            .any(|b| b.contains("2 decision fields")),
        "{:?}",
        report.blocks
    );
}

#[test]
fn the_low_impact_form_is_outside_this_gate() {
    let text = DRAFT.replace("PROCEED_DRAFT_ONLY", "Low impact — proportionate answer");
    let report = check(&text, &conclusions());
    assert!(
        report
            .blocks
            .iter()
            .any(|b| b.contains("not a high-impact decision status")),
        "{:?}",
        report.blocks
    );
}

#[test]
fn an_abbreviated_stop_record_passes_with_notes() {
    let report = check(STOP, &conclusions());
    assert_eq!(report.verdict(), ("PASS", EXIT_PASS), "{:?}", report.blocks);
    assert_eq!(report.notes.len(), 6, "{:?}", report.notes);
    let without_record = STOP.replace("External-action record\n", "");
    assert_eq!(verdict(&without_record).1, EXIT_BLOCK);
}

#[test]
fn a_full_status_cannot_use_the_abbreviated_record() {
    let text = STOP.replace("HARD_STOP", "NEEDS_FACTS");
    let report = check(&text, &conclusions());
    assert!(
        report
            .blocks
            .iter()
            .any(|b| b.contains("missing mandatory section: Assumptions"))
    );
    assert!(
        report
            .blocks
            .iter()
            .any(|b| b.contains("missing source-version line"))
    );
}

#[test]
fn a_screen_hit_alone_asks_for_review() {
    let text = STOP.replace(
        "Concluding on the safe harbour is prohibited",
        "I will not say the safe harbour applies to your position",
    );
    let report = check(&text, &conclusions());
    assert_eq!(report.verdict(), ("REVIEW", EXIT_REVIEW));
    assert_eq!(report.reviews.len(), 1);
}

#[test]
fn reperformance_follows_precedence_percentages_and_rounding() {
    let cases = [
        ("2 + 3 × 4 = 14", true),
        ("(2 + 3) × 4 = 20", true),
        ("$120,000 x 12% = $14,400", true),
        ("100 / 3 = 33.33", true),
        ("100 / 3 = 33.3", true),
        ("100 / 3 = 33.35", false),
        ("100 / 1,000 = 10%", true),
        ("100 / 1,000 = 12%", false),
        ("1,000 − 250 = 750", true),
        ("-5 + 2 = -3", true),
        ("10 ÷ 4 = 2.5", true),
        ("1,100 / 11 = 101", false),
    ];
    for (text, ok) in cases {
        let found = calculations(text);
        assert_eq!(found.len(), 1, "{text}");
        assert_eq!(
            found[0].matches, ok,
            "{text}: computed {}",
            found[0].computed
        );
    }
}

#[test]
fn dates_years_bullets_and_prose_are_not_calculations() {
    for text in [
        "Lodged 1/7/2026 = on time",
        "The 2026-27 year = the current year",
        "- 5 items were reviewed",
        "s 284-75(6) applies",
        "Total = 100",
        "x = 5",
        "Rate 10% applies; 5 = five",
    ] {
        assert!(calculations(text).is_empty(), "{text}");
    }
}

#[test]
fn division_by_zero_does_not_reperform() {
    let found = calculations("100 / 0 = 0");
    assert_eq!(found.len(), 1);
    assert!(!found[0].matches);
    assert_eq!(found[0].computed, "undefined (division by zero)");
    let report = check(
        &DRAFT.replace("$1,100 / 11 = $100.00", "$1,100 / 0 = $0"),
        &conclusions(),
    );
    assert_eq!(report.verdict().1, EXIT_BLOCK);
}

#[test]
fn every_link_of_a_chain_is_checked() {
    for (text, ok) in [
        // A running total: each link equals the next link's leading number.
        ("1 + 1 = 2 + 1 = 3", true),
        // A true chain: each link equals the whole next expression.
        ("$1,000 × 10% = $50 × 2 = $100", true),
        ("1 + 1 = 3 + 2 = 5", false),
        ("2 × 3 = 6 + 1 = 8", false),
        ("Item 2 = $500 + $300 = $800", true),
        // An intermediate expression keeps its own written precision.
        ("100 / 3 = 33.35 + 0 = 33", false),
        ("100 / 3 = 33.33 + 0 = 33", true),
        // A bracket opened by the surrounding prose.
        ("(GST: 1,100 / 11 = 100)", true),
        ("(2 + 3 = 6)", false),
    ] {
        let found = calculations(text);
        assert_eq!(found.len(), 1, "{text}");
        assert_eq!(found[0].matches, ok, "{text}: {}", found[0].computed);
    }
    assert_eq!(calculations("1 + 1 = 3 + 2 = 5")[0].computed, "2.00");
    assert_eq!(
        calculations("Item 2 = $500 + $300 = $800")[0].text,
        "$500 + $300 = $800"
    );
}

#[test]
fn a_bracketed_amount_is_ambiguous_and_blocks() {
    for text in [
        "($100) + $50 = $150",
        "($100) + $50 = ($50)",
        "100 + 20 = (120) + 5 = 125",
    ] {
        let found = calculations(text);
        assert_eq!(found.len(), 1, "{text}");
        assert!(!found[0].matches, "{text}");
        assert!(found[0].computed.starts_with("ambiguous"), "{text}");
    }
    assert!(calculations("(2 + 3) × 4 = 20")[0].matches);
    // A signed bracketed amount is explicit and reperforms.
    for (text, ok) in [
        ("100 + (-20) = 80", true),
        ("100 + (-20) = 90", false),
        ("100 - 150 = (-50)", true),
        ("100 + (20) = 120", false),
    ] {
        let found = calculations(text);
        assert_eq!(found.len(), 1, "{text}");
        assert_eq!(found[0].matches, ok, "{text}: {}", found[0].computed);
    }
}

#[test]
fn a_state_changing_action_asks_for_review() {
    let text = DRAFT.replace(
        "state_changing_tool_action: NONE",
        "state_changing_tool_action: PAY_NOW",
    );
    let report = check(&text, &conclusions());
    assert_eq!(
        report.verdict(),
        ("REVIEW", EXIT_REVIEW),
        "{:?}",
        report.blocks
    );
    assert!(
        report.reviews[0].contains("'PAY_NOW'"),
        "{:?}",
        report.reviews
    );
}

#[test]
fn an_empty_section_counts_as_missing() {
    for (from, to, finding) in [
        (
            "- The supply is wholly taxable.\n",
            "",
            "missing mandatory section: Assumptions (empty",
        ),
        (
            "- The supply is wholly taxable.\n",
            "---\n",
            "missing mandatory section: Assumptions (empty",
        ),
        (
            "- No lodgement, payment or other external action occurred.\n",
            "",
            "missing mandatory section: External-action record (empty",
        ),
    ] {
        let report = check(&DRAFT.replace(from, to), &conclusions());
        assert!(
            report.blocks.iter().any(|b| b.contains(finding)),
            "{finding}: {:?}",
            report.blocks
        );
    }
    let empty_stop = STOP.replace("- No external action occurred.\n", "");
    assert_eq!(verdict(&empty_stop).1, EXIT_BLOCK);
    let report = check(&DRAFT.replace("- None identified.\n", ""), &conclusions());
    assert_eq!(report.verdict().1, EXIT_PASS, "{:?}", report.blocks);
    assert_eq!(
        report.notes,
        ["empty Ethical and regulatory flags section; required where material"]
    );
}

#[test]
fn numbered_headings_and_source_value_notes_are_accepted() {
    let text = DRAFT
        .replace("Scope and period\n", "## 1. Scope and period\n")
        .replace("Assumptions\n", "4) Assumptions:\n")
        .replace(
            "Historical authority: CONFIRMED",
            "Historical authority: CONFIRMED (Federal Register, 20 October 2026)",
        );
    let report = check(&text, &conclusions());
    assert_eq!(report.verdict(), ("PASS", EXIT_PASS), "{:?}", report.blocks);
    let report = check(
        &DRAFT.replace(
            "Historical authority: CONFIRMED",
            "Historical authority: CONFIRMEDLY",
        ),
        &conclusions(),
    );
    assert!(
        report
            .blocks
            .iter()
            .any(|b| b.contains("Historical authority must be")),
        "{:?}",
        report.blocks
    );
}

#[test]
fn the_command_reports_and_exits_by_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo_root();
    let root = root.to_str().unwrap();
    for (name, text, code) in [
        ("good.md", DRAFT.to_string(), EXIT_PASS),
        (
            "bad.md",
            DRAFT.replace("= $100.00", "= $110.00"),
            EXIT_BLOCK,
        ),
    ] {
        std::fs::write(dir.path().join(name), text).unwrap();
        let out = run_in(dir.path(), &["--root", root, "gate", name], false);
        assert_eq!(
            out.code,
            i32::from(code),
            "{name}: {}{}",
            out.stdout,
            out.stderr
        );
    }
    let out = run_in(
        dir.path(),
        &["--root", root, "gate", "--json", "bad.md"],
        false,
    );
    assert!(
        out.stdout.contains("\"verdict\": \"BLOCK\""),
        "{}",
        out.stdout
    );
    let missing = run_in(dir.path(), &["--root", root, "gate", "absent.md"], false);
    assert_eq!(missing.code, i32::from(EXIT_COULD_NOT_RUN));
    assert!(missing.stderr.starts_with("gate: "), "{}", missing.stderr);
    let _ = gate::EXIT_PASS;
}

const FACTS: &str = r#"{"facts": {"invoice total": "$1,100"}}"#;

/// The gate's findings with the facts check run on `text`.
fn with_facts(text: &str, facts: &str) -> gate::Report {
    let mut report = check(text, &conclusions());
    check_facts(text, &load_facts(facts).unwrap(), &mut report);
    report
}

fn facts_reviews(report: &gate::Report) -> Vec<&str> {
    report
        .reviews
        .iter()
        .map(String::as_str)
        .filter(|r| r.starts_with("facts check:"))
        .collect()
}

#[test]
fn a_supplied_fact_and_a_shown_calculation_pass_the_facts_check() {
    // $1,100 is supplied; $100.00 and the later $100 are the result of
    // `$1,100 / 11 = $100.00`.
    let report = with_facts(DRAFT, FACTS);
    assert_eq!(
        report.verdict(),
        ("PASS", EXIT_PASS),
        "{:?}",
        report.reviews
    );
    assert_eq!(
        report.notes,
        [
            "facts check: it flags amounts it cannot trace; an amount that matches a fact or a calculation is not thereby correct"
        ]
    );
    // Without --facts the gate adds no facts-check note.
    assert!(check(DRAFT, &conclusions()).notes.is_empty());
}

#[test]
fn an_amount_without_a_fact_or_calculation_asks_for_review() {
    let text = DRAFT.replace(
        "- The supply is wholly taxable.",
        "- The supply is wholly taxable.\n- A deduction of $1,200 at 15% is available.",
    );
    let report = with_facts(&text, FACTS);
    assert_eq!(
        report.verdict(),
        ("REVIEW", EXIT_REVIEW),
        "{:?}",
        report.blocks
    );
    assert_eq!(
        facts_reviews(&report),
        [
            "facts check: $1,200, 15% not in the facts file or derived by a reperformed calculation; a person must trace each to its source"
        ]
    );
    // The result alone, with its calculation gone, is not traced.
    let text = DRAFT.replace("$1,100 / 11 = $100.00", "GST is $100.00");
    let report = with_facts(&text, FACTS);
    assert_eq!(report.verdict().1, EXIT_REVIEW);
    assert!(facts_reviews(&report)[0].starts_with("facts check: $100.00, $100 not"));
}

#[test]
fn the_facts_check_never_blocks() {
    // A structurally complete draft full of untraced and unreadable amounts
    // reviews; a draft that already blocks keeps exactly its own blocks.
    let text = DRAFT.replace(
        "- None identified.",
        "- $5, $6.50, 7%, $1,1000 and $10% appear nowhere else.",
    );
    let report = with_facts(&text, FACTS);
    assert!(report.blocks.is_empty(), "{:?}", report.blocks);
    assert_eq!(report.verdict(), ("REVIEW", EXIT_REVIEW));
    assert_eq!(facts_reviews(&report).len(), 2, "{:?}", report.reviews);
    let broken = DRAFT.replace("= $100.00", "= $110.00");
    let blocks = check(&broken, &conclusions()).blocks;
    let report = with_facts(&broken, FACTS);
    assert_eq!(report.blocks, blocks);
    assert_eq!(report.verdict().1, EXIT_BLOCK);
}

#[test]
fn a_calculation_traces_only_from_traced_inputs_written_before_it() {
    let facts = r#"{"facts": {"a": "$500", "b": "$300", "c": "$200", "rate": "10%"}}"#;
    for (text, untraced) in [
        // A running total: $800 is derived, then $1,000.
        ("$500 + $300 = $800 + $200 = $1,000\nTotal $1,000.", ""),
        // A result feeds a later calculation.
        ("$500 + $300 = $800\n$800 × 10% = $80", ""),
        // A rounded result is traced as written.
        ("$500 / 3 = $166.67\nShare $166.67.", ""),
        // An untraced dollar input leaves the result untraced.
        ("$1,200 × 10% = $120", "$1,200, $120"),
        // A result used before the calculation that derives it.
        ("$800 × 10% = $80\n$500 + $300 = $800", "$80"),
        // A calculation that does not reperform derives nothing.
        ("$500 + $300 = $900", "$900"),
        // A rounded mention of a fact is not the fact.
        ("About $501 in total.", "$501"),
        // A percentage matches only a percentage, and a dollar amount only a
        // dollar amount or plain number.
        ("Rate $10 and 500%.", "$10, 500%"),
    ] {
        let report = with_facts(text, facts);
        let found = facts_reviews(&report);
        if untraced.is_empty() {
            assert!(found.is_empty(), "{text}: {found:?}");
        } else {
            let expected = format!("facts check: {untraced} not");
            assert!(
                found.first().is_some_and(|r| r.starts_with(&expected)),
                "{text}: {found:?}"
            );
        }
    }
}

#[test]
fn amounts_match_by_value_whatever_their_format() {
    let facts = r#"{"facts": {"a": "1234.5", "b": "-$500", "c": "12.50%", "d": "$2,000.00"}}"#;
    let text = "$1,234.50 and $1234.5, a loss of -$500 or $500, 12.5% and $2000.";
    let report = with_facts(text, facts);
    assert!(facts_reviews(&report).is_empty(), "{:?}", report.reviews);
}

#[test]
fn an_unreadable_draft_amount_is_reported() {
    let report = with_facts("Totals $1,1000 and $10% and $1.2.3.", FACTS);
    assert_eq!(
        facts_reviews(&report),
        ["facts check: could not read $1,1000, $10%, $1.2.3; a person must check each"]
    );
}

#[test]
fn a_malformed_facts_file_is_refused() {
    for (text, reason) in [
        ("{", "not valid JSON"),
        ("[]", "top level must be an object"),
        (
            r#"{"facts": {"a": "$1"}, "notes": 1}"#,
            "unknown key 'notes'",
        ),
        (r#"{"facts": {}}"#, "'facts' must map names to amounts"),
        (r#"{"facts": ["$1"]}"#, "'facts' must map names to amounts"),
        (
            r#"{"facts": {"a": 1100}}"#,
            "'a' must be an amount written as text",
        ),
        (
            r#"{"facts": {"a": "$1,23"}}"#,
            "'a' is not an amount the gate can read: '$1,23'",
        ),
        (r#"{"facts": {"a": "$10%"}}"#, "'a' is not an amount"),
        (r#"{"facts": {"a": "1 100"}}"#, "'a' is not an amount"),
        (r#"{"facts": {"a": " $5"}}"#, "'a' is not an amount"),
        (r#"{"facts": {"a": ""}}"#, "'a' is not an amount"),
        (r#"{"facts": {"a": "$1", "a": "$2"}}"#, "not valid JSON"),
    ] {
        let Err(e) = load_facts(text) else {
            panic!("{text} was accepted");
        };
        assert!(e.starts_with("facts file: "), "{text}: {e}");
        assert!(e.contains(reason), "{text}: {e}");
    }
}

#[test]
fn the_command_reads_a_facts_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo_root();
    let root = root.to_str().unwrap();
    std::fs::write(dir.path().join("draft.md"), DRAFT).unwrap();
    std::fs::write(
        dir.path().join("invented.md"),
        DRAFT.replace("- None identified.", "- Deduction $1,200."),
    )
    .unwrap();
    std::fs::write(dir.path().join("facts.json"), FACTS).unwrap();
    std::fs::write(dir.path().join("bad.json"), r#"{"facts": {"a": 1}}"#).unwrap();
    let gate = |args: &[&str]| {
        let mut all = vec!["--root", root, "gate"];
        all.extend(args);
        run_in(dir.path(), &all, false)
    };
    let out = gate(&["--facts", "facts.json", "draft.md"]);
    assert_eq!(
        out.code,
        i32::from(EXIT_PASS),
        "{}{}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stdout.contains("note: facts check: it flags amounts"),
        "{}",
        out.stdout
    );
    let out = gate(&["--facts", "facts.json", "--json", "invented.md"]);
    assert_eq!(
        out.code,
        i32::from(EXIT_REVIEW),
        "{}{}",
        out.stdout,
        out.stderr
    );
    assert!(
        out.stdout.contains("\"verdict\": \"REVIEW\""),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("facts check: $1,200 not"),
        "{}",
        out.stdout
    );
    for facts in ["bad.json", "absent.json"] {
        let out = gate(&["--facts", facts, "draft.md"]);
        assert_eq!(
            out.code,
            i32::from(EXIT_COULD_NOT_RUN),
            "{facts}: {}",
            out.stdout
        );
        assert!(
            out.stderr.starts_with("gate: facts file: "),
            "{facts}: {}",
            out.stderr
        );
        assert!(out.stdout.is_empty(), "{facts}: {}", out.stdout);
    }
}
