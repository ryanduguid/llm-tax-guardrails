"""Compilation currency: parsing the pins, and what a mismatch is called."""
from __future__ import annotations

import io
import urllib.parse
from datetime import date
from pathlib import Path

from drdebits_build import sources

ROOT = Path(__file__).resolve().parents[3]

TABLE = """## Source status

| Source | Version checked | Status | Role |
|---|---|---|---|
| [*Example Act 2009*](https://www.legislation.gov.au/C2009A00013/latest) \
| `C2025C00107`, Compilation No 26, 21 February 2025 | In force | Primary |
| [*Unpinned Act 1953*](https://www.legislation.gov.au/C1953A00001/latest) \
| Landing page checked; compilation ID not pinned this release | In force | Penalties |
| [TPB library](https://www.tpb.gov.au/policy-and-guidance) | 55 statements | Guidance | Interpretation |

The [repeal](https://www.legislation.gov.au/F2026L00916/asmade/details) is a
citation in prose, not a pinned source.
"""


def test_parse_reads_the_pinned_compilation_and_its_number():
    pins = sources.parse_pins(TABLE)
    pinned = [pin for pin in pins if pin.pinned]
    assert len(pinned) == 1
    assert pinned[0].title_id == "C2009A00013"
    assert pinned[0].register_id == "C2025C00107"
    assert pinned[0].compilation_number == "26"
    assert pinned[0].name == "Example Act 2009"


def test_parse_keeps_an_unpinned_row_rather_than_dropping_it():
    """An unpinned source is still a source, and the guide says so out loud."""
    pins = {pin.title_id: pin for pin in sources.parse_pins(TABLE)}
    assert "C1953A00001" in pins
    assert not pins["C1953A00001"].pinned


def test_parse_ignores_register_links_outside_the_table():
    """A prose citation is not a pin, and checking it would report noise."""
    assert "F2026L00916" not in {pin.title_id for pin in sources.parse_pins(TABLE)}


def test_parse_ignores_sources_that_are_not_on_the_register():
    """Only the Register publishes compilations, so only it can be checked."""
    assert all(pin.title_id.startswith(("C", "F")) for pin in sources.parse_pins(TABLE))
    assert len(sources.parse_pins(TABLE)) == 2


def test_a_newer_compilation_is_superseded_not_merely_different():
    pin = sources.Pin("Example Act 2009", "C2009A00013", "C2025C00107", "26")
    finding = sources.classify(
        pin, {"registerId": "C2026C00500", "compilationNumber": "27", "start": "2026-07-01"}
    )
    assert finding.outcome == sources.SUPERSEDED
    assert "C2026C00500" in finding.detail
    assert "Compilation No 26" in finding.detail


def test_a_matching_compilation_is_current():
    pin = sources.Pin("Example Act 2009", "C2009A00013", "C2025C00107", "26")
    finding = sources.classify(
        pin, {"registerId": "C2025C00107", "compilationNumber": "26", "start": "2025-02-21"}
    )
    assert finding.outcome == sources.CURRENT


def test_no_answer_is_unreachable_and_never_superseded():
    """Checking from a blocked network must not read as a currency failure."""
    pin = sources.Pin("Example Act 2009", "C2009A00013", "C2025C00107", "26")
    assert sources.classify(pin, None).outcome == sources.UNREACHABLE


def test_an_unpinned_row_reports_the_current_compilation_to_pin():
    pin = sources.Pin("Unpinned Act 1953", "C1953A00001", "", "")
    finding = sources.classify(
        pin, {"registerId": "C2026C00393", "compilationNumber": "226", "start": "2026-08-27"}
    )
    assert finding.outcome == sources.UNPINNED
    assert "C2026C00393" in finding.detail


TODAY = date(2026, 9, 27)
PIN = sources.Pin("Example Act 2009", "C2009A00013", "C2025C00107", "26")
# Shaped like the Register's versions records, newest start first.
VERSIONS: list[dict[str, object]] = [
    {"titleId": "C2009A00013", "registerId": None, "start": "2034-10-01T00:00:00",
     "reasons": [{"affect": "Repeal", "markdown": "s 50 of the [Legislation Act 2003](/C2004A01224)"}]},
    {"titleId": "C2009A00013", "registerId": None, "start": "2026-10-01T00:00:00",
     "reasons": [{"affect": "Amend", "markdown": "sch 1 (items 1-69) of the "
                  "[Treasury Laws Amendment (Example) Act 2026](/C2026A00086)"}]},
    {"titleId": "C2009A00013", "registerId": "C2025C00107", "start": "2025-02-21T00:00:00",
     "reasons": []},
]


def test_a_registered_amendment_inside_the_horizon_is_upcoming():
    """The 1 October 2026 TASA amendments sat on the Register while the check read clean."""
    findings = sources.upcoming(PIN, VERSIONS, TODAY)
    assert [finding.outcome for finding in findings] == [sources.UPCOMING]
    assert "2026-10-01" in findings[0].detail
    assert "Treasury Laws Amendment (Example) Act 2026 (C2026A00086)" in findings[0].detail


def test_a_distant_sunset_and_past_versions_are_not_upcoming():
    """Only the amendment is reported: not the 2034 sunset, not the current compilation."""
    assert len(sources.upcoming(PIN, VERSIONS, TODAY)) == 1
    assert sources.upcoming(PIN, VERSIONS, date(2026, 10, 1)) == []


def test_upcoming_changes_are_reported_in_commencement_order():
    staged = [
        {"titleId": "C2009A00013", "start": "2026-12-01T00:00:00", "reasons": []},
        {"titleId": "C2009A00013", "start": "2026-10-01T00:00:00", "reasons": []},
    ]
    details = [finding.detail for finding in sources.upcoming(PIN, staged, TODAY)]
    assert "2026-10-01" in details[0]
    assert "2026-12-01" in details[1]
    assert "the Register records no reason" in details[0]


def test_future_versions_are_requested_nearest_first(monkeypatch):
    """Newest-first with a page limit could push a near amendment off the page."""
    requested: list[str] = []

    def fake_urlopen(request, timeout):
        requested.append(urllib.parse.unquote(request.full_url))
        return io.BytesIO(b'{"value": []}')

    monkeypatch.setattr(sources.urllib.request, "urlopen", fake_urlopen)
    assert sources.registered_versions("C2009A00013", TODAY) == []
    assert "$orderby=start asc" in requested[0]
    assert "start gt 2026-09-27T00:00:00" in requested[0]


def test_no_answer_on_future_versions_is_unreachable_not_clean():
    """A failed query must not read as 'nothing scheduled'."""
    assert [f.outcome for f in sources.upcoming(PIN, None, TODAY)] == [sources.UNREACHABLE]


def test_the_real_source_status_table_still_parses():
    """A parser that silently finds nothing would report a clean check.

    The guide's table is hand-maintained Markdown. If a row is reformatted and
    the pattern stops matching, every source drops out of the run and the
    summary reads `checked 0` rather than failing, so the count is asserted
    here against the file as it actually stands.
    """
    text = (ROOT / sources.SOURCE_STATUS).read_text(encoding="utf-8")
    pins = sources.parse_pins(text)
    assert len(pins) >= 5, "the source-status table lists at least 5 Register sources"
    assert len({pin.title_id for pin in pins}) == len(pins), "no title is pinned twice"
    assert any(pin.pinned for pin in pins), "at least one source pins a compilation"
