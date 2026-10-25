//! Exact quotations, conversion differences and incomplete coverage
//! (ported from test_quotecheck.py). Values Python had to reject at run time
//! (a boolean or float character count, a truthy completeness flag) cannot be
//! expressed through the command's typed arguments.

mod common;

use common::run_in;
use drdebits::builder::sha256_hex;
use drdebits::pyjson::Json;
use drdebits::quotecheck::assess_quote;

fn field<'a>(result: &'a Json, key: &str) -> &'a Json {
    result.get(key).unwrap()
}

fn status(source: &str, quote: &str, used: Option<i64>, complete: bool) -> String {
    field(
        &assess_quote(source, quote, used, complete).unwrap(),
        "status",
    )
    .as_str()
    .unwrap()
    .to_string()
}

#[test]
fn exact_match_reports_original_offsets_and_content_identity() {
    let source = "Intro. Synthetic receipts require review. End.";
    let quote = "Synthetic receipts require review.";
    let result = assess_quote(source, quote, None, true).unwrap();
    assert_eq!(field(&result, "status"), &Json::Str("EXACT_MATCH".into()));
    let (Json::Int(start), Json::Int(end)) = (field(&result, "start"), field(&result, "end"))
    else {
        panic!()
    };
    let chars: Vec<char> = source.chars().collect();
    let (start, end): (usize, usize) = (start.try_into().unwrap(), end.try_into().unwrap());
    assert_eq!(chars[start..end].iter().collect::<String>(), quote);
    assert_eq!(
        field(&result, "source_sha256"),
        &Json::Str(sha256_hex(source.as_bytes()))
    );
    assert_eq!(field(&result, "coverage_complete"), &Json::Bool(true));
}

#[test]
fn offsets_count_characters_not_bytes() {
    let result = assess_quote("é quote", "quote", None, true).unwrap();
    assert_eq!(field(&result, "start"), &Json::Int(2.into()));
}

#[test]
fn conversion_difference_is_a_normalised_lead_not_verbatim() {
    for (source, quote) in [
        ("The oﬃce keeps records.", "The office keeps records."),
        (
            "Retain\n  supporting records.",
            "Retain supporting records.",
        ),
    ] {
        let result = assess_quote(source, quote, None, true).unwrap();
        assert_eq!(
            field(&result, "status"),
            &Json::Str("NORMALISED_MATCH".into())
        );
        assert_eq!(
            (field(&result, "start"), field(&result, "end")),
            (&Json::Null, &Json::Null)
        );
    }
}

#[test]
fn changed_words_or_case_are_not_verified() {
    for quote in ["Records are optional.", "records are required."] {
        assert_eq!(
            status("Records are required.", quote, None, true),
            "NOT_FOUND"
        );
    }
}

#[test]
fn omitted_tail_cannot_prove_a_quote_is_absent() {
    let source = "Heading. Supporting evidence appears later.";
    let result = assess_quote(source, "Supporting evidence", Some(8), true).unwrap();
    assert_eq!(field(&result, "status"), &Json::Str("UNVERIFIED".into()));
    assert_eq!(field(&result, "characters_used"), &Json::Int(8.into()));
    assert_eq!(
        field(&result, "source_characters"),
        &Json::Int((source.chars().count() as i64).into())
    );
    assert_eq!(field(&result, "coverage_complete"), &Json::Bool(false));
    assert_ne!(
        field(&result, "visible_sha256"),
        field(&result, "source_sha256")
    );
    assert_eq!(
        status(source, "Supporting evidence", None, true),
        "EXACT_MATCH"
    );
}

#[test]
fn a_supplied_excerpt_is_incomplete_even_without_a_local_cut() {
    let result = assess_quote("One paragraph.", "Another paragraph.", None, false).unwrap();
    assert_eq!(field(&result, "status"), &Json::Str("UNVERIFIED".into()));
    assert_eq!(field(&result, "coverage_complete"), &Json::Bool(false));
}

#[test]
fn partial_coverage_keeps_an_observed_match_and_its_limit() {
    let result = assess_quote(
        "Matched text. Unread tail.",
        "Matched text.",
        Some(13),
        true,
    )
    .unwrap();
    assert_eq!(field(&result, "status"), &Json::Str("EXACT_MATCH".into()));
    assert_eq!(field(&result, "coverage_complete"), &Json::Bool(false));
}

#[test]
fn invalid_coverage_is_rejected() {
    for used in [-1, 50] {
        assert!(assess_quote("Source.", "Source", Some(used), true).is_err());
    }
}

#[test]
fn missing_quote_is_rejected() {
    for quote in ["", " \n\t"] {
        assert!(assess_quote("Source.", quote, None, true).is_err());
    }
}

#[test]
fn empty_or_zero_coverage_is_unverified_when_the_source_is_incomplete() {
    assert_eq!(status("", "Claim", None, false), "UNVERIFIED");
    assert_eq!(status("Claim", "Claim", Some(0), true), "UNVERIFIED");
}

#[test]
fn the_command_reads_files_and_prints_the_assessment() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("s.txt"), "Records are required.").unwrap();
    let out = run_in(
        dir.path(),
        &["quote", "--source", "s.txt", "--quote", "are required"],
        false,
    );
    assert_eq!(out.code, 0);
    assert!(out.stdout.starts_with("{\n  \"status\": \"EXACT_MATCH\","));
    let bad = run_in(
        dir.path(),
        &[
            "quote",
            "--source",
            "s.txt",
            "--quote",
            "x",
            "--characters-used",
            "99",
        ],
        false,
    );
    assert_eq!(
        (bad.code, bad.stderr.as_str()),
        (
            1,
            "quote: Characters used must be an integer within the supplied text\n"
        )
    );
    let typed = run_in(
        dir.path(),
        &[
            "quote",
            "--source",
            "s.txt",
            "--quote",
            "x",
            "--characters-used",
            "1.5",
        ],
        false,
    );
    assert_eq!(typed.code, 2);
}
