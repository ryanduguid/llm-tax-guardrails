"""Exact quotations, conversion differences and incomplete source coverage."""
import hashlib

import pytest
from drdebits_build.quotecheck import assess_quote


def test_exact_match_reports_original_offsets_and_content_identity():
    source = "Intro. Synthetic receipts require review. End."
    quote = "Synthetic receipts require review."
    result = assess_quote(source, quote)
    assert result["status"] == "EXACT_MATCH"
    assert source[result["start"]:result["end"]] == quote
    assert result["source_sha256"] == hashlib.sha256(source.encode()).hexdigest()
    assert result["coverage_complete"] is True


@pytest.mark.parametrize("source,quote", [
    ("The oﬃce keeps records.", "The office keeps records."),
    ("Retain\n  supporting records.", "Retain supporting records."),
])
def test_conversion_difference_is_a_normalised_lead_not_verbatim(source, quote):
    result = assess_quote(source, quote)
    assert result["status"] == "NORMALISED_MATCH"
    assert result["start"] is None and result["end"] is None


@pytest.mark.parametrize("quote", ["Records are optional.", "records are required."])
def test_changed_words_or_case_are_not_verified(quote):
    assert assess_quote("Records are required.", quote)["status"] == "NOT_FOUND"


def test_omitted_tail_cannot_prove_a_quote_is_absent():
    source = "Heading. Supporting evidence appears later."
    result = assess_quote(source, "Supporting evidence", characters_used=8)
    assert result["status"] == "UNVERIFIED"
    assert result["characters_used"] == 8
    assert result["source_characters"] == len(source)
    assert result["coverage_complete"] is False
    assert result["visible_sha256"] != result["source_sha256"]
    assert assess_quote(source, "Supporting evidence")["status"] == "EXACT_MATCH"


def test_a_supplied_excerpt_is_incomplete_even_without_a_local_cut():
    result = assess_quote("One paragraph.", "Another paragraph.", source_complete=False)
    assert result["status"] == "UNVERIFIED"
    assert result["coverage_complete"] is False


def test_partial_coverage_keeps_an_observed_match_and_its_limit():
    result = assess_quote("Matched text. Unread tail.", "Matched text.", characters_used=13)
    assert result["status"] == "EXACT_MATCH"
    assert result["coverage_complete"] is False


@pytest.mark.parametrize("used", [True, -1, 50, 1.5, "2"])
def test_invalid_coverage_is_rejected(used):
    with pytest.raises(ValueError):
        assess_quote("Source.", "Source", characters_used=used)


@pytest.mark.parametrize("quote", ["", " \n\t", None])
def test_missing_quote_is_rejected(quote):
    with pytest.raises(ValueError):
        assess_quote("Source.", quote)


def test_empty_or_zero_coverage_is_unverified_when_the_source_is_incomplete():
    assert assess_quote("", "Claim", source_complete=False)["status"] == "UNVERIFIED"
    assert assess_quote("Claim", "Claim", characters_used=0)["status"] == "UNVERIFIED"


@pytest.mark.parametrize("complete", [0, "true", None])
def test_completeness_cannot_be_inferred_from_a_truthy_value(complete):
    with pytest.raises(ValueError):
        assess_quote("Source.", "Source", source_complete=complete)
