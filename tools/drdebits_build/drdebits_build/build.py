"""Assemble the runtime files from src/. Deterministic, LF, UTF-8."""
from __future__ import annotations

import hashlib
import re
from dataclasses import dataclass
from datetime import date
from pathlib import Path

from . import evals, model


class BuildError(Exception):
    pass


# Markdown rendering primitives. Explicit line construction only.


def render_frontmatter(meta):
    lines = ["---"]
    lines += [f"{k}: {v}" for k, v in meta.items()]
    lines.append("---")
    return "\n".join(lines) + "\n"


def render_table(headers, aligns, rows):
    out = ["| " + " | ".join(headers) + " |", "|" + "|".join(aligns) + "|"]
    out += ["| " + " | ".join(r) + " |" for r in rows]
    return "\n".join(out) + "\n"


# Matches only the guide-version token in its 2 sanctioned stamp contexts:
# the "Version: `X`" header line and the "Part of [DrDebits](./drdebits.md)
# `X`" satellite reference. The context and backticks are zero-width
# lookaround, not consumed, so a replacement touches only the inner token.
# Anchoring to context keeps stamping and verification away from every other
# version-like token - bare prose ("uv 0.12.0") and unrelated backticked
# versions (a documented tool pin such as `0.12.0`) alike - which both must
# leave alone.
_VERSION_TOKEN = r"[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.]+)?"
STAMP_RE = re.compile(
    rf"(?<=Version: `){_VERSION_TOKEN}(?=`)"
    rf"|(?<=\[DrDebits\]\(\./drdebits\.md\) `){_VERSION_TOKEN}(?=`)"
)

# Hand-written files the build stamps the guide version into, and that verify
# then holds to metadata. llms.txt is the machine-readable index a retrieval
# agent reads instead of README.md, so it has to carry the same version and
# source-check date as the README rather than leaving a consumer to infer them.
STAMPED_FILES = ("README.md", "MAINTENANCE.md", "llms.txt")

# Of those, the files that also carry a hand-written copy of the source-check
# date. MAINTENANCE.md does not, so it is not checked for one.
SOURCE_CHECK_DATE_FILES = ("README.md", "llms.txt")

CHANGELOG_HEADERS = ["Version", "Date", "Status", "Change"]


def find_root(start):
    p = Path(start).resolve()
    for candidate in (p, *p.parents):
        if (candidate / "drdebits.md").is_file() and (candidate / "src").is_dir():
            return candidate
    raise BuildError(f"no DrDebits root found above {start}")


@dataclass
class Sources:
    meta: dict
    fragments: list
    catalogue: list
    behaviour: list
    apes: dict
    vendor: dict
    changelog: list


def load_sources(root):
    root = Path(root)
    data = root / "src" / "data"
    meta = model.load_metadata(data / "metadata.yaml")
    model.validate_required_metadata(meta)
    catalogue = model.load_catalogue(data / "tpb-catalogue.yaml")
    model.validate_counts(meta, catalogue)
    fragments = []
    # Sort by name string, not Path: Path ordering casefolds on Windows but
    # not on POSIX, so a mixed-case fragment name would order differently on
    # the dev box and the CI rebuild.
    for f in sorted((root / "src" / "guide").glob("*.md"), key=lambda p: p.name):
        fragments.append((f.name, f.read_text(encoding="utf-8")))
    if not fragments:
        raise BuildError("src/guide/ has no fragments")
    return Sources(
        meta=meta,
        fragments=fragments,
        catalogue=catalogue,
        behaviour=model.load_behaviour_tests(data / "behaviour-tests.yaml"),
        apes=model.load_apes_map(data / "apes-110-map.yaml"),
        vendor=model.load_vendor_assurance(data / "ai-vendor-assurance.yaml"),
        changelog=model.load_changelog(data / "changelog.yaml"),
    )


def build_changelog_section(s):
    rows = [[r["version"], r["date"], r["status"], r["change"]] for r in s.changelog]
    return "## Change log\n\n" + render_table(
        CHANGELOG_HEADERS, ["---", "---", "---", "---"], rows)


def build_guide(s):
    parts = [render_frontmatter({k: v for k, v in s.meta.items() if k != "checksum_files"})]
    parts += [text for _, text in s.fragments]
    parts.append(build_changelog_section(s))
    parts.append("\n" + s.meta["guide_end_marker"] + "\n")
    return "".join(parts)


CATALOGUE_HEADER_TEMPLATE = '# Complete TPB Guidance Statement catalogue\n\nPart of [DrDebits](../drdebits.md) `{version}`. Retrieve this file when routing a task against TPB guidance; verify it against `SHA256SUMS` in the release.\n\n\nThis catalogue covers all final, live TPB Guidance Statements discoverable on {checked}: {count} statements, {first} to {last}, all exposed by the filtered TPB library index. It excludes withdrawn or superseded products, historical versions, exposure drafts, consultation material, factsheets, FAQs and other non-Guidance-Statement webpages. Those sources may still matter to a particular task and must be retrieved separately with their authority and status labelled.\n\nThe “LLM trigger” is an independent DrDebits routing note, not a substitute for reading the linked statement. Statements about education, registration or professional associations might not affect the wording of an ordinary client deliverable, but they remain relevant to capability, authority and service-scope checks. “Complete” here means the checked live Guidance Statement category, not every policy or guidance product ever published by the TPB.\n\n'

_MONTHS = ("January", "February", "March", "April", "May", "June", "July",
           "August", "September", "October", "November", "December")


def _render_checked_date(sources_checked_at):
    """Render metadata's ISO sources_checked_at as prose, for example, "16 August 2026".

    Month names are a fixed English tuple, not strftime, so the output cannot
    vary with the build machine's locale.
    """
    d = date.fromisoformat(sources_checked_at[:10])
    return f"{d.day} {_MONTHS[d.month - 1]} {d.year}"


def build_catalogue_md(s):
    rows = [[f"[{r['title']}]({r['url']})", r["trigger"]] for r in s.catalogue]
    header = CATALOGUE_HEADER_TEMPLATE.format(
        version=s.meta["guide_version"],
        checked=_render_checked_date(s.meta["sources_checked_at"]),
        count=s.meta["tpb_guidance_statement_count"],
        first=s.catalogue[0]["id"],
        last=s.catalogue[-1]["id"],
    )
    return header + render_table(
        ["Statement (concise title and official link)", "LLM trigger"], ["---", "---"], rows)


BEHAVIOUR_HEADER_TEMPLATE = '# DrDebits behaviour tests\n\nPart of [DrDebits](../drdebits.md) `{version}`. These tests travel with the guide; verify this file against `SHA256SUMS` in the release.\n\n## Behaviour tests\n\nAn implementation of DrDebits MUST pass at least these adverse cases. Evaluate observable outputs and actions, not hidden reasoning.\n\n'


def build_behaviour_md(s):
    rows = [[r["id"], r["scenario"], r["expected_status"], r["required_behaviour"], r["side_effect_check"]]
            for r in s.behaviour]
    header = BEHAVIOUR_HEADER_TEMPLATE.format(version=s.meta["guide_version"])
    return header + render_table(
        ["ID", "Scenario", "Expected status", "Required behaviour and human step", "Side-effect check"],
        ["---", "---", "---", "---", "---"], rows)


APES_HEADER_TEMPLATE = '# Primary APES 110 reference map\n\nPart of [DrDebits](../drdebits.md) `{version}`. Retrieve this file when locating APES 110 provisions; verify it against `SHA256SUMS` in the release.\n\n\nThe LLM should retrieve the operative paragraphs from the official compilation rather than rely on this short map.\n\nIn APES 110, `R` and `AUST R` identify requirement paragraphs. `A` paragraphs are application material that must be considered to understand and apply the requirements and conceptual framework, but they are not separate requirements. DrDebits controls may be deliberately more conservative than either category and must remain labelled as project controls.\n\n'
APES_BETWEEN_TABLES = '\nKey paragraph-level retrieval points are:\n\n'


def build_apes_md(s):
    header = APES_HEADER_TEMPLATE.format(version=s.meta["guide_version"])
    a = render_table(["Context", "APES 110 starting points"], ["---", "---"],
                            [[r["label"], r["value"]] for r in s.apes["contexts"]])
    b = render_table(["Control", "APES 110 retrieval points"], ["---", "---"],
                            [[r["label"], r["value"]] for r in s.apes["retrieval_points"]])
    return header + a + APES_BETWEEN_TABLES + b


VENDOR_HEADER_TEMPLATE = '# AI tool and vendor assurance checklist\n\nPart of [DrDebits](../drdebits.md) `{version}`. Retrieve this file when a firm is assessing an AI product, vendor or in-house deployment before it reaches client work; verify it against `SHA256SUMS` in the release.\n\n\nThe AI-specific TPB rules in the guide require due diligence over confidentiality, privacy, security, access, retention, training use, subcontractors, location, incident response and exit arrangements. This checklist turns that requirement into questions a firm can send, with the evidence that answers each one, where the underlying obligation sits, and what follows when nobody answers.\n\nThe checklist names no vendor and favours none. Apply it to a hosted product, to a general-purpose assistant, to a firm’s own deployment, and to DrDebits and the runtime it loads into.\n\nThree rules govern its use. An unanswered question is unanswered, not passed. Marketing copy, a certification badge, a partner tier and a security web page answer only the questions they address, and the firm records the rest as open. A completed checklist is a record of what the firm asked and what it received: it is not approval, it is not a compliance certification, and it leaves the practitioner’s accountability where it already sat.\n\nThe obligation column locates the operative text without quoting it. Retrieve the current source before relying on any row, and read the row as a DrDebits project control rather than as a statement of what the law or a standard requires.\n\n'
VENDOR_HEADERS = ["ID", "Question", "Evidence that answers it",
                  "Where the obligation sits", "If nobody answers"]


def build_vendor_assurance_md(s):
    parts = [VENDOR_HEADER_TEMPLATE.format(version=s.meta["guide_version"])]
    for title in s.vendor["sections"]:
        rows = [[r["id"], r["question"], r["evidence"], r["obligation"], r["if_absent"]]
                for r in s.vendor["entries"] if r["section"] == title]
        parts.append(f"## {title}\n\n")
        parts.append(render_table(VENDOR_HEADERS, ["---"] * len(VENDOR_HEADERS), rows))
        parts.append("\n")
    # Every section contributed a trailing blank line; the file ends with one
    # newline, as the other generated files do.
    return "".join(parts).rstrip("\n") + "\n"


GENERATED = {
    "drdebits.md": build_guide,
    "reference/tpb-catalogue.md": build_catalogue_md,
    "tests/behaviour-tests.md": build_behaviour_md,
    "reference/apes-110-map.md": build_apes_md,
    "reference/ai-vendor-assurance.md": build_vendor_assurance_md,
    evals.CASES_FILE: evals.build_cases,
}


def build_sha256sums(root, s, generated):
    lines = []
    for rel in s.meta["checksum_files"].split("|"):
        if rel in generated:
            digest = hashlib.sha256(generated[rel].encode("utf-8")).hexdigest()
        else:
            digest = hashlib.sha256((Path(root) / rel).read_bytes()).hexdigest()
        lines.append(f"{digest} *{rel}")
    return "\n".join(lines) + "\n"


def stamp_version(text, version):
    return STAMP_RE.sub(version, text)


def write_outputs(root, outdir, s=None):
    s = load_sources(root) if s is None else s
    outdir = Path(outdir)
    written = {rel: fn(s) for rel, fn in GENERATED.items()}
    written["SHA256SUMS"] = build_sha256sums(root, s, written)
    # The results table depends on the recorded runs as well as src/.
    written[evals.RESULTS_FILE] = evals.build_results_md(root, s)
    for rel, content in written.items():
        target = outdir / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content.encode("utf-8"))
    return written
