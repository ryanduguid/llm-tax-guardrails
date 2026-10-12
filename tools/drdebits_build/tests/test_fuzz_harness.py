"""The fuzz target's seed corpus replays on every platform, and its checks bite."""
from pathlib import Path
from unittest import mock

import pytest
from fuzz import sources

CORPUS = Path(__file__).resolve().parents[1] / "fuzz" / "corpus"


@pytest.mark.parametrize("seed", sorted(CORPUS.iterdir()), ids=lambda path: path.name)
def test_seed_corpus(seed):
    sources.test_one_input(seed.read_bytes())


def test_an_unknown_decision_status_is_a_failure():
    with mock.patch.object(sources.harness, "decision_statuses", return_value=["MAYBE"]):
        with pytest.raises(AssertionError, match="unknown status 'MAYBE'"):
            sources.test_one_input(b"x")


def test_a_reader_returning_a_non_mapping_is_a_failure():
    with mock.patch.object(sources.model, "_read", return_value=[]):
        with pytest.raises(AssertionError, match="non-mapping"):
            sources.test_one_input(b"- a\n")


def test_unexpected_reader_errors_escape():
    with mock.patch.object(sources.model, "_read", side_effect=RuntimeError("bug")):
        with pytest.raises(RuntimeError, match="bug"):
            sources.test_one_input(b"a: 1\n")
