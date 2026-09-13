"""Repository policy checks for source locators, security and releases."""

from __future__ import annotations

import re
import tomllib
from html import unescape
from pathlib import Path
from urllib.parse import unquote

import pytest
from drdebits_build.build import find_root
from drdebits_build.model import load_behaviour_tests, load_metadata

ROOT = find_root(Path(__file__).resolve())
GUIDE_SOURCE = ROOT / "src" / "guide" / "150-apes-110-control-set.md"

LOCATOR_METADATA = {
    "apesb_root_url": "https://apesb.org.au/",
    "apesb_locator_rechecked_at": "2026-08-20",
    "apes_110_navigation": (
        "Standards & Guidance > Current Pronouncements > "
        "Compilation of APES 110 Standard (Jul 2025)"
    ),
    "apes_220_navigation": (
        "Standards & Guidance > Specialist Pronouncements > Taxation Services > "
        "APES 220 Taxation Services (2025) - effective from 1 July 2025"
    ),
    "apes_220_pdf_filename": "APES_220_Jan_2025.pdf",
    "apes_220_technical_update": (
        "2025/5 - APESB issues revised APES 220 Taxation Services"
    ),
    "apes_220_technical_update_date": "2025-01-31",
    "apes_ai_alert_navigation": (
        "Home > Interest Areas > "
        "The ethical use of artificial intelligence by professional accountants"
    ),
    "apes_ai_alert_title": (
        "The ethical use of artificial intelligence by professional accountants"
    ),
    "apes_ai_alert_label": "Technical Alert",
    "apes_ai_alert_date": "2025-10-31",
}
CANONICAL_GUIDE_LOCATOR_SECTIONS = {
    "### Scope": (
        "\n"
        "Parts 1 to 4B apply as a Professional Standard to members of Chartered "
        "Accountants Australia and New Zealand, CPA Australia and the Institute "
        "of Public Accountants within their scope. APES 110 may also be "
        "incorporated into legally enforceable auditing requirements, law, "
        "regulation or engagement terms; paragraph 1.5 notes the legal effect of "
        "ASA 102 for relevant Corporations Act audits and reviews. Part 5 applies "
        "to Sustainability Assurance Practitioners for the services described in "
        "paragraph 5100.2, whether or not the practitioner is a member.\n\n"
        "A non-member providing only tax or BAS services must not be described as "
        "bound by APES 110 unless another applicable requirement incorporates it. "
        "That person remains bound by the TASA framework where applicable, and "
        "DrDebits may adopt APES-aligned project controls without misrepresenting "
        "their source or legal force.\n\n"
        "At the [APESB website](https://apesb.org.au/), follow `Standards & "
        "Guidance` → `Current Pronouncements` → `Compilation of APES 110 Standard "
        "(Jul 2025)`. Confirm the expected files "
        "`Compiled_APES_110_July_25.pdf` and "
        "`Compilation_Details_APES_110_July_25.pdf` before relying on the "
        "compilation. The document is the November 2018 Code as amended through "
        "July 2025; it is not a “2026 edition”. Apply other APES standards when "
        "the service triggers them; APES 110 is not the entire "
        "professional-standards framework.\n\n"
    ),
    "### APES 220 Taxation Services": (
        "\n"
        "For a Member providing a taxation service, apply APES 220 *Taxation "
        "Services* (issued January 2025, effective 1 July 2025) alongside APES "
        "110 and the TASA layer. At the [APESB website](https://apesb.org.au/), "
        "follow `Standards & Guidance` → `Specialist Pronouncements` → `Taxation "
        "Services` → `APES 220 Taxation Services (2025) - effective from 1 July "
        "2025`, and confirm the expected standard filename "
        "`APES_220_Jan_2025.pdf`. The related Technical Update `2025/5`, *APESB "
        "issues revised APES 220 Taxation Services*, is dated 31 January 2025. "
        "APES 220 sets service-level obligations for taxation services, including "
        "tax schemes and arrangements, use of estimates, false or misleading "
        "information, client monies, professional fees and documentation.\n\n"
        "The LLM MUST classify whether the task is a taxation service for a "
        "Member and, if so, check APES 220 in addition to the controls in this "
        "guide. Compliance with one layer is not compliance with another: the "
        "TASA Code, APES 110 and APES 220 each apply within their own scope, and "
        "the strictest applicable obligation governs.\n\n"
    ),
    "### Technology and AI": (
        "\n"
        "Apply the technology revisions with their engagement-specific effective "
        "dates. At the [APESB website](https://apesb.org.au/), follow `Interest "
        "Areas` → `The ethical use of artificial intelligence by professional "
        "accountants`; confirm that it is labelled `Technical Alert` and dated "
        "31 October 2025.\n\n"
        "Publisher navigation and document identifiers were rechecked on 20 "
        "August 2026; the complete substantive source review remains 16 August "
        "2026.\n\n"
        "- The responsible member or Sustainability Assurance Practitioner "
        "remains responsible for analysis, professional judgement and outcomes "
        "within the applicable scope.\n"
        "- Verify AI-generated information with suitable primary evidence and "
        "independent calculation or review where material.\n"
        "- Avoid undue reliance on or influence from technology.\n"
        "- Maintain relevant technology competence and understand limitations, "
        "bias, provenance, security and explainability that affect the activity.\n"
        "- Supervise and review AI-assisted work. Do not describe the model as an "
        "external expert, reviewer or approver.\n"
        "- APESB’s Technical Alert says members should disclose when AI tools are "
        "used and should supervise and review that use. Mandatory disclosure also "
        "applies where required by law, engagement terms or a service-specific "
        "APES standard. Make any disclosure accurate and protect confidential "
        "information; do not expose prompts or data unnecessarily.\n"
        "- Protect confidential information across the complete data lifecycle "
        "and obtain proper authority for uses such as training, product "
        "development, research or benchmarking.\n\n"
    ),
}


def _section(text: str, heading: str) -> str:
    """Return a Markdown section, stopping at the next peer/parent heading."""
    match = re.search(rf"^{re.escape(heading)}\s*$", text, flags=re.MULTILINE)
    assert match is not None, f"missing Markdown section {heading!r}"
    level = len(heading) - len(heading.lstrip("#"))
    tail = text[match.end() :]
    end = re.search(rf"^#{{1,{level}}}\s+", tail, flags=re.MULTILINE)
    return tail[: end.start() if end else None]


def _normalise_line_endings(text: str) -> str:
    """Treat reviewed LF and CRLF content as equivalent, but nothing else."""
    normalised = text.replace("\r\n", "\n")
    assert "\r" not in normalised
    return normalised


def _assert_guide_locators(text: str) -> None:
    for heading, canonical in CANONICAL_GUIDE_LOCATOR_SECTIONS.items():
        actual = _normalise_line_endings(_section(text, heading))
        assert actual == canonical


APESB_TARGET_TOKEN_RE = re.compile(
    r"[^\s<>()\[\]{}\"'`]*apesb\.org\.au[^\s<>()\[\]{}\"'`]*",
    flags=re.IGNORECASE,
)
ALLOWED_APESB_URLS = {"https://apesb.org.au/", "https://www.apesb.org.au/"}


def _decode_url_reference_text(text: str) -> str:
    """Decode entity/percent wrappers to a bounded, stable representation."""
    current = text
    for _ in range(4):
        decoded = unquote(unescape(current))
        if decoded == current:
            return current
        current = decoded
    assert unquote(unescape(current)) == current, "URL text exceeds decoding limit"
    return current


def _apesb_url_targets(text: str) -> list[str]:
    """Inventory APESB targets in inline, autolink, plain and reference forms."""
    decoded = _decode_url_reference_text(text)
    targets = []
    for match in APESB_TARGET_TOKEN_RE.finditer(decoded):
        target = match.group()
        if match.end() == len(decoded) or decoded[match.end()].isspace():
            target = target.rstrip(".,;")
        if ":" in target or target.startswith("//"):
            targets.append(target)
    return targets


def _assert_only_root_apesb_links(text: str) -> None:
    assert set(_apesb_url_targets(text)) <= ALLOWED_APESB_URLS


def _normalise_prose(text: str) -> str:
    return " ".join(text.casefold().split())


SUPPORTED_VERSIONS_POLICY = (
    "security fixes are applied to the latest version on the default branch."
)
REPORTING_POLICY = (
    "please use this repository's private vulnerability reporting feature. "
    "do not open a public issue or pull request for a suspected security "
    "vulnerability. include a clear description, reproduction steps, impact, "
    "and any suggested mitigation. a valid report will be acknowledged within "
    "7 days, and the fix and disclosure timeline will be agreed with the "
    "reporter."
)
REPRODUCTION_DATA_POLICY = (
    "use fabricated or synthetic reproduction data only. never include client, "
    "taxpayer, employee or payroll data, credentials, access tokens, `.env` "
    "files, proprietary prompts or other sensitive data in a report, attachment, "
    "issue or pull request."
)


DISCLAIMER_RESPONSIBILITY_LEAD_IN = (
    "drdebits requires, consistent with the responsibilities that the *tax "
    "agent services act 2009 (tasa)* and *apes 110 code of ethics for "
    "professional accountants* place on the practitioner, that a registered "
    "tax agent, bas agent, or qualified professional accountant:"
)
STATUTORY_ATTRIBUTION_LEAD_IN = "under the *tax agent services act 2009 (tasa)*"


def _assert_disclaimer_responsibility_framing(text: str) -> None:
    """The section states 3 requirements and cites no provision of either
    instrument for any of them, so it must not open by telling the reader that
    those instruments are what require them. Establishing which duties the
    TASA and APES 110 in fact impose is the practitioner's job, not this
    file's, which is the same line 060 draws about the guide's own MUST."""
    section = _normalise_prose(_section(text, "## 3. Human practitioner responsibility"))
    assert section.startswith(DISCLAIMER_RESPONSIBILITY_LEAD_IN)
    assert STATUTORY_ATTRIBUTION_LEAD_IN not in section


def _assert_security_policy(text: str) -> None:
    supported = _normalise_prose(_section(text, "## Supported versions"))
    reporting = _normalise_prose(_section(text, "## Reporting a vulnerability"))
    data = _normalise_prose(_section(text, "## Reproduction and sensitive data"))

    assert supported == SUPPORTED_VERSIONS_POLICY
    assert reporting == REPORTING_POLICY
    assert data == REPRODUCTION_DATA_POLICY


CHECKSUM_FILES = (
    "LICENSE",
    "README.md",
    "CITATION.cff",
    "drdebits.md",
    "MAINTENANCE.md",
    "reference/tpb-catalogue.md",
    "reference/apes-110-map.md",
    "reference/ai-vendor-assurance.md",
    "tests/behaviour-tests.md",
)
PROHIBITED_CONCLUSION_TESTS = ("IND-001", "SAFE-001", "CERT-001")
HISTORICAL_PRERELEASES = (
    "v0.1.0-draft",
    "v0.2.0-draft",
    "v0.3.0-draft",
    "v0.3.1-draft",
)


# Step 8 of the release protocol is the exhaustive "what to edit" list. Every
# hand-written copy the build does not derive, and every assertion that pins
# one, has to be named there: a maintainer who follows the step literally must
# not end up with a self-contradictory release or a red suite.
RELEASE_PROTOCOL_STEP_8_TOKENS = (
    "`review_due`",
    "`src/guide/000-header.md`",
    "`src/guide/040-source-status.md`",
    "`src/guide/150-apes-110-control-set.md`",
    "`src/guide/180-workpaper-record.md`",
    "`CITATION.cff`",
    "`GS` range",
    "`tools/drdebits_build/tests/test_repository_policy.py`",
    "`tools/drdebits_build/drdebits_build/verify.py`",
)


def _release_protocol_step(text: str, number: int) -> str:
    """Return one numbered step of the per-release protocol, which precedes the
    GitHub release checklist's own numbered list."""
    protocol = text.split("## GitHub release checklist", 1)[0]
    match = re.search(rf"^{number}\. (.+)$", protocol, flags=re.MULTILINE)
    assert match is not None, f"missing release protocol step {number}"
    return match.group(1)


def _assert_release_protocol_names_every_hand_written_copy(text: str) -> None:
    step = _release_protocol_step(text, 8)
    for token in RELEASE_PROTOCOL_STEP_8_TOKENS:
        assert token in step, token


def _assert_release_checklist(text: str) -> None:
    checklist = _section(text, "## GitHub release checklist")
    lowered = checklist.lower()

    for filename in (*CHECKSUM_FILES, "SHA256SUMS"):
        assert filename in checklist
    for evidence in (
        "complete pytest suite",
        "builder `verify`",
        "deterministic second build",
        "reviewed link-check result",
        "signed release commit",
        "signed annotated tag",
        "github draft release",
        "checksum and verification instructions",
        "download every staged asset into a clean location",
        "while the release is still a draft",
        "publish the release only after all verification succeeds",
        "`gh release verify tag`",
        "`gh release verify-asset tag path`",
        "do not overwrite or delete",
        "do not reuse its tag",
        "published prereleases, not github draft releases",
        "do not silently rewrite them",
        "new tag and a new release",
    ):
        assert evidence in lowered
    for tag in HISTORICAL_PRERELEASES:
        assert tag in checklist


def test_locator_metadata_is_exact_without_advancing_full_review_date():
    metadata = load_metadata(ROOT / "src" / "data" / "metadata.yaml")
    assert metadata["sources_checked_at"] == "2026-08-16T00:00:00+10:00"
    assert {
        key: metadata[key]
        for key in (
            "apes_110_compilation",
            "apes_110_pdf_filename",
            "apes_110_compilation_details_filename",
            "apes_110_pdf_sha256",
            "apes_220_issued",
            "apes_220_effective",
        )
    } == {
        "apes_110_compilation": "July 2025",
        "apes_110_pdf_filename": "Compiled_APES_110_July_25.pdf",
        "apes_110_compilation_details_filename": (
            "Compilation_Details_APES_110_July_25.pdf"
        ),
        "apes_110_pdf_sha256": (
            "B6937B93B0A6F7F3F32667CFE8880F8F60CBD7777BCE0C2D57F2DD6F13D3A300"
        ),
        "apes_220_issued": "January 2025",
        "apes_220_effective": "2025-07-01",
    }
    assert {key: metadata[key] for key in LOCATOR_METADATA} == LOCATOR_METADATA


def test_guide_binds_each_locator_to_its_subject_section():
    _assert_guide_locators(GUIDE_SOURCE.read_text(encoding="utf-8"))


def test_all_apesb_links_stay_on_the_permitted_publisher_root():
    paths = [*sorted((ROOT / "src" / "guide").glob("*.md")), ROOT / "drdebits.md"]
    observed_urls = set()
    for path in paths:
        text = path.read_text(encoding="utf-8")
        _assert_only_root_apesb_links(text)
        observed_urls.update(_apesb_url_targets(text))
    assert observed_urls

    guide = GUIDE_SOURCE.read_text(encoding="utf-8")
    for prohibited_target in (
        "http://apesb.org.au/wp-content/uploads/evil.pdf",
        "https://apesb.org.au/wp-content/uploads/evil.pdf",
        "https://apesb.org.au/?download=evil.pdf",
        "https://apesb.org.au/#current-pronouncements",
        "javascript:https://apesb.org.au/",
        "prefix-https://apesb.org.au/",
    ):
        mutated = guide.replace("https://apesb.org.au/", prohibited_target, 1)
        assert mutated != guide
        with pytest.raises(AssertionError):
            _assert_only_root_apesb_links(mutated)

    for permitted_target in sorted(ALLOWED_APESB_URLS):
        permitted_forms = (
            f"[APESB]({permitted_target})",
            f"<{permitted_target}>",
            permitted_target,
            f"{permitted_target}.",
            f"{permitted_target},",
            f"{permitted_target};",
            f"[APESB][publisher]\n\n[publisher]: {permitted_target}",
        )
        for form in permitted_forms:
            assert _apesb_url_targets(form) == [permitted_target]
            _assert_only_root_apesb_links(form)

    encoded_and_alternate_forms = (
        "[APESB](https://apesb%2eorg%2eau/wp-content/uploads/evil.pdf)",
        "[APESB](https://apesb.org.au/%77p-content/uploads/evil.pdf)",
        "[APESB](https://apesb&#46;org&#46;au/wp-content/uploads/evil.pdf)",
        "[APESB](https://apesb.org.au/&#x77;p-content/uploads/evil.pdf)",
        "<https://apesb.org.au/wp-content/uploads/evil.pdf>",
        "https://apesb.org.au/wp-content/uploads/evil.pdf",
        "[APESB website][apesb]\n\n"
        "[apesb]: https://apesb.org.au/wp-content/uploads/evil.pdf",
    )
    for mutated in encoded_and_alternate_forms:
        with pytest.raises(AssertionError):
            _assert_only_root_apesb_links(mutated)


def test_security_policy_uses_private_reporting_and_safe_reproduction_data():
    policy = (ROOT / "SECURITY.md").read_text(encoding="utf-8")
    _assert_security_policy(policy)


def test_uv_manifest_uses_canonical_dependency_names_for_dependabot():
    manifest = tomllib.loads(
        (ROOT / "tools" / "drdebits_build" / "pyproject.toml").read_text(
            encoding="utf-8"
        )
    )
    requirements = [
        *manifest["project"]["dependencies"],
        *manifest["project"]["optional-dependencies"]["dev"],
        *manifest["build-system"]["requires"],
    ]

    for requirement in requirements:
        declared_name = re.split(r"[<>=!~;\[]", requirement, maxsplit=1)[0]
        canonical_name = re.sub(r"[-_.]+", "-", declared_name).lower()
        assert declared_name == canonical_name, requirement


def test_checksum_manifest_membership_matches_the_documented_bundle():
    """`SHA256SUMS` membership is driven entirely by this metadata string, and
    the generated manifest is byte-compared by verify, so pinning the string
    pins the manifest. Without it a bad edit or merge could shrink the bundle
    while README, MAINTENANCE and the guide's own integrity instruction keep
    promising digests for all 8 files."""
    metadata = load_metadata(ROOT / "src" / "data" / "metadata.yaml")
    assert metadata["checksum_files"].split("|") == list(CHECKSUM_FILES)


def test_prohibited_conclusion_behaviour_tests_require_one_status():
    """These 3 rows describe the same act: refuse a conclusion the guide
    flatly prohibits, collect the facts, refer the decision to the authorised
    human. Non-negotiable stops names reaching or certifying such a conclusion
    as an absolute stop and the output contract permits exactly one status, so
    all 3 must require HARD_STOP. Rows demanding different statuses for the
    same behaviour cannot all be passed by any implementation that does not
    hard-code the scenario names, which defeats a conformance suite."""
    entries = load_behaviour_tests(ROOT / "src" / "data" / "behaviour-tests.yaml")
    statuses = {
        row["id"]: row["expected_status"]
        for row in entries
        if row["id"] in PROHIBITED_CONCLUSION_TESTS
    }
    assert sorted(statuses) == sorted(PROHIBITED_CONCLUSION_TESTS)
    assert set(statuses.values()) == {"HARD_STOP"}, statuses


def test_disclaimer_attributes_project_controls_to_the_project():
    """The disclaimer's own job is legal carefulness, so it must not present
    DrDebits controls as duties imposed by TASA or APES 110 - the move the
    guide declines to make about its own instruction words."""
    disclaimer = (ROOT / "DISCLAIMER.md").read_text(encoding="utf-8")
    _assert_disclaimer_responsibility_framing(disclaimer)


def test_release_protocol_step_8_lists_every_unguarded_copy():
    maintenance = (ROOT / "MAINTENANCE.md").read_text(encoding="utf-8")
    _assert_release_protocol_names_every_hand_written_copy(maintenance)

    # A token elsewhere in the protocol cannot satisfy step 8: a maintainer
    # working through that step must find it there.
    moved = maintenance.replace(
        "`tools/drdebits_build/tests/test_repository_policy.py`", "the suite", 1)
    moved += "\n`tools/drdebits_build/tests/test_repository_policy.py`\n"
    with pytest.raises(AssertionError):
        _assert_release_protocol_names_every_hand_written_copy(moved)


def test_release_checklist_stages_and_verifies_before_publication():
    maintenance = (ROOT / "MAINTENANCE.md").read_text(encoding="utf-8")
    _assert_release_checklist(maintenance)
