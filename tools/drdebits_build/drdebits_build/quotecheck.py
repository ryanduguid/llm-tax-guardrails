"""Offline quote evidence checks; text membership never establishes legal support."""
from __future__ import annotations

import hashlib
import unicodedata


def _normalise(text: str) -> str:
    return " ".join(unicodedata.normalize("NFKC", text).split())


def assess_quote(source_text: str, quote: str, *, characters_used: int | None = None,
                 source_complete: bool = True) -> dict[str, object]:
    """Assess only text the reviewer or model saw, with explicit coverage.

    Offsets refer to the original supplied text and exist only for exact matches.
    A normalised match is a review lead, never verification of a verbatim quote.
    ``source_complete`` concerns the supplied document, not its legal currency.
    """
    if not isinstance(source_text, str) or not isinstance(quote, str) or not quote.strip():
        raise ValueError("Source and a nonblank quote must be text")
    if type(source_complete) is not bool:
        raise ValueError("Source completeness must be explicit true or false")
    used = len(source_text) if characters_used is None else characters_used
    if type(used) is not int or not 0 <= used <= len(source_text):
        raise ValueError("Characters used must be an integer within the supplied text")
    visible = source_text[:used]
    complete = source_complete and used == len(source_text)
    start = visible.find(quote)
    end = start + len(quote) if start >= 0 else None
    if start >= 0:
        status = "EXACT_MATCH"
    elif _normalise(quote) in _normalise(visible):
        status = "NORMALISED_MATCH"
    else:
        status = "NOT_FOUND" if complete else "UNVERIFIED"
    return {
        "status": status,
        "source_sha256": hashlib.sha256(source_text.encode("utf-8")).hexdigest(),
        "visible_sha256": hashlib.sha256(visible.encode("utf-8")).hexdigest(),
        "source_characters": len(source_text),
        "characters_used": used,
        "source_complete": source_complete,
        "coverage_complete": complete,
        "start": start if start >= 0 else None,
        "end": end,
        "boundary": "Text membership only; applicability and claim support require review.",
    }
