"""Compilation currency for the pinned Register sources. Report-only.

`linkcheck` answers whether a source URL still resolves. It cannot answer the
question that actually decides this guide's accuracy: whether the compilation
the guide pins is still the current one. A pinned compilation keeps resolving
long after it has been superseded, so link rot and currency are different
failures and only one of them was being checked.

The Federal Register publishes the answer through its API, so this needs no
scraping and no judgement: the guide says the *Tax Agent Services Act 2009* was
read at `C2025C00107`, Compilation No 26, and the API says which compilation is
current today. A difference is reportable; it is not a licence to advance
`sources_checked_at`, because the guide's source date means a competent human
read the material and this only means a number moved.

The pins come out of `src/guide/040-source-status.md`, which is where they are
already maintained. A second copy in a data file would be one more thing to
keep in step, and the two would eventually disagree.

Run it before the `review_due` date rather than re-reading all eleven sources:

    uv run --project tools/drdebits_build --locked python -m drdebits_build.sources --root .
"""
from __future__ import annotations

import argparse
import json
import re
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import NamedTuple

from .build import find_root

SOURCE_STATUS = Path("src") / "guide" / "040-source-status.md"

API = "https://api.prod.legislation.gov.au/v1"
# The API host is not the website host. linkcheck's note about AU government
# sites refusing cloud IP ranges applies to www.legislation.gov.au; if the API
# ever refuses too, the run exits 2 for incomplete verification. A missing
# answer never establishes that a compilation has been superseded.
UA = "llm-tax-guardrails source-check (+https://github.com/ryanduguid/llm-tax-guardrails)"
TIMEOUT = 60
SELECT = "titleId,registerId,compilationNumber,start"

# A Register title id: one letter, a year, one letter, five digits. C-prefixed
# ids are Acts, F-prefixed are legislative instruments; both appear here.
TITLE_ID = r"[A-Z]\d{4}[A-Z]\d{5}"
# Only the source-status table is read. The same file discusses a repealed
# instrument in prose, and that link is a citation, not a pin.
TABLE_ROW = re.compile(
    r"^\|\s*\[(?P<name>[^\]]+)\]\("
    r"https://www\.legislation\.gov\.au/(?P<title_id>" + TITLE_ID + r")/latest\)"
    r"\s*\|(?P<pin>[^|]*)\|",
    re.MULTILINE,
)
PINNED_COMPILATION = re.compile(r"`(?P<register_id>" + TITLE_ID + r")`")
COMPILATION_NUMBER = re.compile(r"Compilation No\s*(?P<number>\d+)", re.IGNORECASE)

CURRENT = "current"
SUPERSEDED = "superseded"
UNPINNED = "unpinned"
UNREACHABLE = "unreachable"


class Pin(NamedTuple):
    """One Register source as the guide records it."""

    name: str
    title_id: str
    register_id: str
    compilation_number: str

    @property
    def pinned(self) -> bool:
        return bool(self.register_id)


class Finding(NamedTuple):
    pin: Pin
    outcome: str
    detail: str


def clean(name: str) -> str:
    """Strip the Markdown emphasis the table uses around statute titles."""
    return name.replace("*", "").strip()


def parse_pins(text: str) -> list[Pin]:
    """Read every Register source out of the source-status table."""
    pins: list[Pin] = []
    for row in TABLE_ROW.finditer(text):
        pin_cell = row.group("pin")
        register = PINNED_COMPILATION.search(pin_cell)
        number = COMPILATION_NUMBER.search(pin_cell)
        pins.append(
            Pin(
                name=clean(row.group("name")),
                title_id=row.group("title_id"),
                register_id=register.group("register_id") if register else "",
                compilation_number=number.group("number") if number else "",
            )
        )
    return pins


def current_version(title_id: str, timeout: int = TIMEOUT) -> dict[str, str] | None:
    """Return the Register's current version of one title, or None.

    `$filter` with `isCurrent eq true` is the documented way to resolve this;
    key lookup and a `/latest/` path are not aliases for it.
    """
    query = urllib.parse.quote(f"titleId eq '{title_id}' and isCurrent eq true")
    url = f"{API}/versions?$top=1&$filter={query}&$select={SELECT}"
    request = urllib.request.Request(url, headers={"User-Agent": UA})
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            document = json.load(response)
    except (urllib.error.URLError, OSError, ValueError):
        return None
    if not isinstance(document, dict):
        return None
    values = document.get("value")
    if not isinstance(values, list) or not values or not isinstance(values[0], dict):
        return None
    version = values[0]
    # A response carrying another Act's record is worse than no response, so
    # it is refused rather than compared.
    if version.get("titleId") not in (None, title_id):
        return None
    if not isinstance(version.get("registerId"), str) or not re.fullmatch(
            TITLE_ID, version["registerId"]):
        return None
    return {key: str(version.get(key) or "") for key in ("registerId", "compilationNumber", "start")}


def classify(pin: Pin, version: dict[str, str] | None) -> Finding:
    if version is None:
        return Finding(pin, UNREACHABLE, "the Register did not answer for this title")
    live = f"{version['registerId']}, Compilation No {version['compilationNumber']}"
    served = f" (in force from {version['start'][:10]})" if version["start"] else ""
    if not pin.pinned:
        return Finding(pin, UNPINNED, f"the guide pins no compilation; current is {live}{served}")
    if version["registerId"] == pin.register_id:
        return Finding(pin, CURRENT, f"still current at {live}")
    return Finding(
        pin,
        SUPERSEDED,
        f"the guide reads {pin.register_id}, Compilation No {pin.compilation_number}; "
        f"current is {live}{served}",
    )


def check(root: Path, timeout: int = TIMEOUT) -> list[Finding]:
    text = (root / SOURCE_STATUS).read_text(encoding="utf-8")
    return [classify(pin, current_version(pin.title_id, timeout)) for pin in parse_pins(text)]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="drdebits_build.sources")
    parser.add_argument("--root", default=None)
    parser.add_argument("--timeout", type=int, default=TIMEOUT)
    arguments = parser.parse_args(argv)
    root = Path(arguments.root) if arguments.root else find_root(Path.cwd())

    findings = check(root, arguments.timeout)
    if not findings:
        print("no Register sources found in the source-status table", flush=True)
        return 1

    for finding in findings:
        print(f"{finding.outcome.upper():<11} {finding.pin.name}: {finding.detail}")

    counts = {name: sum(1 for f in findings if f.outcome == name) for name in
              (CURRENT, SUPERSEDED, UNPINNED, UNREACHABLE)}
    print(
        f"checked {len(findings)}: current {counts[CURRENT]}, "
        f"superseded {counts[SUPERSEDED]}, unpinned {counts[UNPINNED]}, "
        f"unreachable {counts[UNREACHABLE]}"
    )
    if counts[SUPERSEDED]:
        print(
            "A superseded compilation means the guide reads material that is no longer "
            "in force. Label the affected material SOURCE CURRENCY NOT CONFIRMED until a "
            "competent human has reviewed the replacement. Do not advance "
            "sources_checked_at on the strength of this check."
        )
    if counts[SUPERSEDED]:
        return 1
    return 2 if counts[UNREACHABLE] or counts[UNPINNED] else 0


if __name__ == "__main__":
    raise SystemExit(main())
