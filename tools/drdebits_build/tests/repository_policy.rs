//! Repository policy: source locators, security, releases and documented
//! counts (ported from test_repository_policy.py).

mod common;

use common::repo_root;
use drdebits::builder;
use drdebits::model::{load_behaviour_tests, load_metadata, meta_get};
use drdebits::py::{self, PyPath};
use regex::Regex;

fn root_py(rel: &str) -> PyPath {
    PyPath::new(&repo_root().join(rel).to_string_lossy())
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(repo_root().join(rel)).unwrap()
}

const GUIDE_SOURCE: &str = "src/guide/150-apes-110-control-set.md";

const LOCATOR_METADATA: [(&str, &str); 11] = [
    ("apesb_root_url", "https://apesb.org.au/"),
    ("apesb_locator_rechecked_at", "2026-10-14"),
    (
        "apes_110_navigation",
        "Standards & Guidance > Current Pronouncements > Compilation of APES 110 Standard (Jul 2025)",
    ),
    (
        "apes_220_navigation",
        "Standards & Guidance > Specialist Pronouncements > Taxation Services > APES 220 Taxation Services (2025) - effective from 1 July 2025",
    ),
    ("apes_220_pdf_filename", "APES_220_Jan_2025.pdf"),
    (
        "apes_220_technical_update",
        "2025/5 - APESB issues revised APES 220 Taxation Services",
    ),
    ("apes_220_technical_update_date", "2025-01-31"),
    (
        "apes_ai_alert_navigation",
        "Home > Interest Areas > The ethical use of artificial intelligence by professional accountants",
    ),
    (
        "apes_ai_alert_title",
        "The ethical use of artificial intelligence by professional accountants",
    ),
    ("apes_ai_alert_label", "Technical Alert"),
    ("apes_ai_alert_date", "2025-10-31"),
];

const SCOPE_SECTION: &str = "\nParts 1 to 4B apply as a Professional Standard to members of Chartered Accountants Australia and New Zealand, CPA Australia and the Institute of Public Accountants within their scope. APES 110 may also be incorporated into legally enforceable auditing requirements, law, regulation or engagement terms; paragraph 1.5 notes the legal effect of ASA 102 for relevant Corporations Act audits and reviews. Part 5 applies to Sustainability Assurance Practitioners for the services described in paragraph 5100.2, whether or not the practitioner is a member.\n\nA non-member providing only tax or BAS services must not be described as bound by APES 110 unless another applicable requirement incorporates it. That person remains bound by the TASA framework where applicable, and DrDebits may adopt APES-aligned project controls without misrepresenting their source or legal force.\n\nAt the [APESB website](https://apesb.org.au/), follow `Standards & Guidance` → `Current Pronouncements` → `Compilation of APES 110 Standard (Jul 2025)`. Confirm the expected files `Compiled_APES_110_July_25.pdf` and `Compilation_Details_APES_110_July_25.pdf` before relying on the compilation. The document is the November 2018 Code as amended through July 2025; it is not a “2026 edition”. Apply other APES standards when the service triggers them; APES 110 is not the entire professional-standards framework.\n\n";

const APES_220_SECTION: &str = "\nFor a Member providing a taxation service, apply APES 220 *Taxation Services* (issued January 2025, effective 1 July 2025) alongside APES 110 and the TASA layer. At the [APESB website](https://apesb.org.au/), follow `Standards & Guidance` → `Specialist Pronouncements` → `Taxation Services` → `APES 220 Taxation Services (2025) - effective from 1 July 2025`, and confirm the expected standard filename `APES_220_Jan_2025.pdf`. The related Technical Update `2025/5`, *APESB issues revised APES 220 Taxation Services*, is dated 31 January 2025. APES 220 sets service-level obligations for taxation services, including tax schemes and arrangements, use of estimates, false or misleading information, client monies, professional fees and documentation.\n\nThe LLM MUST classify whether the task is a taxation service for a Member and, if so, check APES 220 in addition to the controls in this guide. Compliance with one layer is not compliance with another: the TASA Code, APES 110 and APES 220 each apply within their own scope, and the strictest applicable obligation governs.\n\n";

const AI_SECTION: &str = "\nApply the technology revisions with their engagement-specific effective dates. At the [APESB website](https://apesb.org.au/), follow `Interest Areas` → `The ethical use of artificial intelligence by professional accountants`; confirm that it is labelled `Technical Alert` and dated 31 October 2025.\n\nPublisher navigation, document identifiers and the complete substantive source review were last checked on 14 October 2026.\n\n- The responsible member or Sustainability Assurance Practitioner remains responsible for analysis, professional judgement and outcomes within the applicable scope.\n- Verify AI-generated information with suitable primary evidence and independent calculation or review where material.\n- Avoid undue reliance on or influence from technology.\n- Maintain relevant technology competence and understand limitations, bias, provenance, security and explainability that affect the activity.\n- Supervise and review AI-assisted work. Do not describe the model as an external expert, reviewer or approver.\n- APESB’s Technical Alert says members should disclose when AI tools are used and should supervise and review that use. Mandatory disclosure also applies where required by law, engagement terms or a service-specific APES standard. Make any disclosure accurate and protect confidential information; do not expose prompts or data unnecessarily.\n- Protect confidential information across the complete data lifecycle and obtain proper authority for uses such as training, product development, research or benchmarking.\n\n";

/// A Markdown section, stopping at the next peer or parent heading.
fn section(text: &str, heading: &str) -> String {
    let start = Regex::new(&format!(r"(?m)^{}{}*$", regex::escape(heading), *py::S)).unwrap();
    let m = start
        .find(text)
        .unwrap_or_else(|| panic!("missing Markdown section {heading:?}"));
    let level = heading.len() - heading.trim_start_matches('#').len();
    let tail = &text[m.end()..];
    let end = Regex::new(&format!(r"(?m)^#{{1,{level}}}{}+", *py::S)).unwrap();
    match end.find(tail) {
        Some(e) => tail[..e.start()].to_string(),
        None => tail.to_string(),
    }
}

fn normalise_line_endings(text: &str) -> String {
    let normalised = text.replace("\r\n", "\n");
    assert!(!normalised.contains('\r'));
    normalised
}

fn assert_guide_locators(text: &str) {
    for (heading, canonical) in [
        ("### Scope", SCOPE_SECTION),
        ("### APES 220 Taxation Services", APES_220_SECTION),
        ("### Technology and AI", AI_SECTION),
    ] {
        assert_eq!(
            normalise_line_endings(&section(text, heading)),
            canonical,
            "{heading}"
        );
    }
}

/// `html.unescape` for numeric references and the common named ones.
fn unescape(text: &str) -> String {
    let re = Regex::new(r"&(#[0-9]+|#[xX][0-9a-fA-F]+|amp|lt|gt|quot|apos|nbsp);").unwrap();
    re.replace_all(text, |c: &regex::Captures| {
        let body = &c[1];
        let ch = if let Some(hex) = body.strip_prefix("#x").or_else(|| body.strip_prefix("#X")) {
            u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
        } else if let Some(dec) = body.strip_prefix('#') {
            dec.parse::<u32>().ok().and_then(char::from_u32)
        } else {
            Some(match body {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                _ => '\u{a0}',
            })
        };
        ch.map_or_else(|| c[0].to_string(), |c| c.to_string())
    })
    .into_owned()
}

/// `urllib.parse.unquote`: percent escapes decoded as UTF-8, invalid bytes replaced.
fn unquote(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let (Some(h), Some(l)) = (
                bytes.get(i + 1).and_then(|b| (*b as char).to_digit(16)),
                bytes.get(i + 2).and_then(|b| (*b as char).to_digit(16)),
            )
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Decode entity and percent wrappers to a bounded, stable representation.
fn decode_url_reference_text(text: &str) -> String {
    let mut current = text.to_string();
    for _ in 0..4 {
        let decoded = unquote(&unescape(&current));
        if decoded == current {
            return current;
        }
        current = decoded;
    }
    assert_eq!(
        unquote(&unescape(&current)),
        current,
        "URL text exceeds decoding limit"
    );
    current
}

/// APESB targets in inline, autolink, plain and reference forms.
fn apesb_url_targets(text: &str) -> Vec<String> {
    let decoded = decode_url_reference_text(text);
    let s = py::class_body(drdebits::pyunicode::SPACE);
    let token = Regex::new(&format!(
        r#"[^{s}<>()\[\]{{}}"'`]*{}[^{s}<>()\[\]{{}}"'`]*"#,
        py::ignorecase_literal("apesb.org.au")
    ))
    .unwrap();
    let mut targets = Vec::new();
    for m in token.find_iter(&decoded) {
        let mut target = m.as_str();
        let at_end =
            m.end() == decoded.len() || decoded[m.end()..].chars().next().is_some_and(py::is_space);
        if at_end {
            target = target.trim_end_matches(['.', ',', ';']);
        }
        if target.contains(':') || target.starts_with("//") {
            targets.push(target.to_string());
        }
    }
    targets
}

const ALLOWED_APESB_URLS: [&str; 2] = ["https://apesb.org.au/", "https://www.apesb.org.au/"];

fn only_root_apesb_links(text: &str) -> bool {
    apesb_url_targets(text)
        .iter()
        .all(|t| ALLOWED_APESB_URLS.contains(&t.as_str()))
}

fn normalise_prose(text: &str) -> String {
    py::split_whitespace(&py::casefold(text)).join(" ")
}

#[test]
fn locator_metadata_is_exact_without_advancing_full_review_date() {
    let meta = load_metadata(&root_py("src/data/metadata.yaml")).unwrap();
    let get = |k: &str| meta_get(&meta, k).unwrap_or_else(|| panic!("{k}"));
    assert_eq!(get("sources_checked_at"), "2026-10-14T00:00:00+11:00");
    for (k, v) in [
        ("apes_110_compilation", "July 2025"),
        ("apes_110_pdf_filename", "Compiled_APES_110_July_25.pdf"),
        (
            "apes_110_compilation_details_filename",
            "Compilation_Details_APES_110_July_25.pdf",
        ),
        (
            "apes_110_pdf_sha256",
            "B6937B93B0A6F7F3F32667CFE8880F8F60CBD7777BCE0C2D57F2DD6F13D3A300",
        ),
        ("apes_220_issued", "January 2025"),
        ("apes_220_effective", "2025-07-01"),
    ] {
        assert_eq!(get(k), v, "{k}");
    }
    for (k, v) in LOCATOR_METADATA {
        assert_eq!(get(k), v, "{k}");
    }
}

#[test]
fn guide_binds_each_locator_to_its_subject_section() {
    assert_guide_locators(&read(GUIDE_SOURCE));
}

#[test]
fn all_apesb_links_stay_on_the_permitted_publisher_root() {
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(repo_root().join("src/guide"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    paths.sort();
    paths.push(repo_root().join("drdebits.md"));
    let mut observed = Vec::new();
    for path in paths {
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(only_root_apesb_links(&text), "{}", path.display());
        observed.extend(apesb_url_targets(&text));
    }
    assert!(!observed.is_empty());

    let guide = read(GUIDE_SOURCE);
    for prohibited in [
        "http://apesb.org.au/wp-content/uploads/evil.pdf",
        "https://apesb.org.au/wp-content/uploads/evil.pdf",
        "https://apesb.org.au/?download=evil.pdf",
        "https://apesb.org.au/#current-pronouncements",
        "javascript:https://apesb.org.au/",
        "prefix-https://apesb.org.au/",
    ] {
        let mutated = guide.replacen("https://apesb.org.au/", prohibited, 1);
        assert_ne!(mutated, guide);
        assert!(!only_root_apesb_links(&mutated), "{prohibited}");
    }
    for permitted in ALLOWED_APESB_URLS {
        for form in [
            format!("[APESB]({permitted})"),
            format!("<{permitted}>"),
            permitted.to_string(),
            format!("{permitted}."),
            format!("{permitted},"),
            format!("{permitted};"),
            format!("[APESB][publisher]\n\n[publisher]: {permitted}"),
        ] {
            assert_eq!(apesb_url_targets(&form), [permitted], "{form}");
            assert!(only_root_apesb_links(&form));
        }
    }
    for mutated in [
        "[APESB](https://apesb%2eorg%2eau/wp-content/uploads/evil.pdf)",
        "[APESB](https://apesb.org.au/%77p-content/uploads/evil.pdf)",
        "[APESB](https://apesb&#46;org&#46;au/wp-content/uploads/evil.pdf)",
        "[APESB](https://apesb.org.au/&#x77;p-content/uploads/evil.pdf)",
        "<https://apesb.org.au/wp-content/uploads/evil.pdf>",
        "https://apesb.org.au/wp-content/uploads/evil.pdf",
        "[APESB website][apesb]\n\n[apesb]: https://apesb.org.au/wp-content/uploads/evil.pdf",
    ] {
        assert!(!only_root_apesb_links(mutated), "{mutated}");
    }
}

#[test]
fn security_policy_uses_private_reporting_and_safe_reproduction_data() {
    let policy = read("SECURITY.md");
    assert_eq!(
        normalise_prose(&section(&policy, "## Supported versions")),
        "security fixes are applied to the latest version on the default branch."
    );
    assert_eq!(
        normalise_prose(&section(&policy, "## Reporting a vulnerability")),
        "report privately at <https://github.com/ryanduguid/llm-tax-guardrails/security/advisories/new>. please use this \
         repository's private vulnerability reporting feature. do not open a public issue or pull request for a suspected \
         security vulnerability. include a clear description, reproduction steps, impact, and any suggested mitigation. a \
         valid report will be acknowledged within 7 days, and the fix and disclosure timeline will be agreed with the reporter."
    );
    assert_eq!(
        normalise_prose(&section(&policy, "## Reproduction and sensitive data")),
        "use fabricated or synthetic reproduction data only. never include client, taxpayer, employee or payroll data, \
         credentials, access tokens, `.env` files, proprietary prompts or other sensitive data in a report, attachment, \
         issue or pull request."
    );
}

#[test]
fn builder_dependencies_are_locked_and_the_toolchain_pinned() {
    // Dependabot reads the manifests; the locks keep CI on reviewed versions.
    for crate_dir in ["tools/drdebits_build", "tools/drdebits_build/fuzz"] {
        assert!(
            repo_root().join(crate_dir).join("Cargo.lock").is_file(),
            "{crate_dir}"
        );
    }
    let toolchain = read("rust-toolchain.toml");
    assert!(toolchain.contains("channel = \"1.99\""));
    let dependabot = read(".github/dependabot.yml");
    assert!(dependabot.contains("package-ecosystem: cargo"));
}

const CHECKSUM_FILES: [&str; 12] = [
    "LICENSE",
    "README.md",
    "CITATION.cff",
    "drdebits.md",
    "MAINTENANCE.md",
    "reference/tpb-catalogue.md",
    "reference/apes-110-map.md",
    "reference/ai-vendor-assurance.md",
    "reference/tpb-gs55-concordance.md",
    "tests/behaviour-tests.md",
    "DISCLAIMER.md",
    "NOTICE",
];

#[test]
fn checksum_manifest_membership_matches_the_documented_bundle() {
    let meta = load_metadata(&root_py("src/data/metadata.yaml")).unwrap();
    assert_eq!(
        meta_get(&meta, "checksum_files")
            .unwrap()
            .split('|')
            .collect::<Vec<_>>(),
        CHECKSUM_FILES
    );
}

#[test]
fn prohibited_conclusion_behaviour_tests_require_one_status() {
    // These five rows describe the same act, so all five require HARD_STOP.
    let ids = ["IND-001", "SAFE-001", "SAFE-002", "SAFE-003", "CERT-001"];
    let entries = load_behaviour_tests(&root_py("src/data/behaviour-tests.yaml")).unwrap();
    let statuses: Vec<(&str, &str)> = entries
        .iter()
        .filter(|r| ids.contains(&r.get("id")))
        .map(|r| (r.get("id"), r.get("expected_status")))
        .collect();
    assert_eq!(statuses.len(), ids.len());
    assert!(
        statuses.iter().all(|(_, s)| *s == "HARD_STOP"),
        "{statuses:?}"
    );
}

#[test]
fn disclaimer_attributes_project_controls_to_the_project() {
    let disclaimer = section(
        &read("DISCLAIMER.md"),
        "## 3. Human practitioner responsibility",
    );
    let normalised = normalise_prose(&disclaimer);
    assert!(normalised.starts_with(
        "drdebits requires, consistent with the responsibilities that the *tax agent services act 2009 (tasa)* and *apes 110 \
         code of ethics for professional accountants* place on the practitioner, that a registered tax agent, bas agent, or \
         qualified professional accountant:"
    ));
    assert!(!normalised.contains("under the *tax agent services act 2009 (tasa)*"));
}

// Step 8 of the release protocol is the exhaustive "what to edit" list.
const STEP_8_TOKENS: [&str; 9] = [
    "`review_due`",
    "`src/guide/000-header.md`",
    "`src/guide/040-source-status.md`",
    "`src/guide/150-apes-110-control-set.md`",
    "`src/guide/180-workpaper-record.md`",
    "`CITATION.cff`",
    "`GS` range",
    "`tools/drdebits_build/tests/repository_policy.rs`",
    "`tools/drdebits_build/src/verify.rs`",
];

fn release_protocol_step(text: &str, number: u32) -> String {
    let protocol = text.split("## GitHub release checklist").next().unwrap();
    let re = Regex::new(&format!(r"(?m)^{number}\. (.+)$")).unwrap();
    re.captures(protocol)
        .unwrap_or_else(|| panic!("missing release protocol step {number}"))[1]
        .to_string()
}

fn names_every_hand_written_copy(text: &str) -> bool {
    let step = release_protocol_step(text, 8);
    STEP_8_TOKENS.iter().all(|t| step.contains(t))
}

#[test]
fn release_protocol_step_8_lists_every_unguarded_copy() {
    let maintenance = read("MAINTENANCE.md");
    assert!(names_every_hand_written_copy(&maintenance));
    // A token elsewhere in the protocol cannot satisfy step 8.
    let moved = maintenance.replacen(
        "`tools/drdebits_build/tests/repository_policy.rs`",
        "the suite",
        1,
    ) + "\n`tools/drdebits_build/tests/repository_policy.rs`\n";
    assert!(!names_every_hand_written_copy(&moved));
}

#[test]
fn release_checklist_stages_and_verifies_before_publication() {
    let checklist = section(&read("MAINTENANCE.md"), "## GitHub release checklist");
    let lowered = checklist.to_lowercase();
    for filename in CHECKSUM_FILES.iter().chain(&["SHA256SUMS"]) {
        assert!(checklist.contains(filename), "{filename}");
    }
    for evidence in [
        "complete test suite",
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
    ] {
        assert!(lowered.contains(evidence), "{evidence}");
    }
    for tag in [
        "v0.1.0-draft",
        "v0.2.0-draft",
        "v0.3.0-draft",
        "v0.3.1-draft",
    ] {
        assert!(checklist.contains(tag), "{tag}");
    }
}

fn number_word(n: usize) -> Option<&'static str> {
    const WORDS: [(usize, &str); 30] = [
        (27, "Twenty-seven"),
        (28, "Twenty-eight"),
        (29, "Twenty-nine"),
        (30, "Thirty"),
        (31, "Thirty-one"),
        (32, "Thirty-two"),
        (33, "Thirty-three"),
        (34, "Thirty-four"),
        (35, "Thirty-five"),
        (36, "Thirty-six"),
        (37, "Thirty-seven"),
        (38, "Thirty-eight"),
        (39, "Thirty-nine"),
        (40, "Forty"),
        (41, "Forty-one"),
        (42, "Forty-two"),
        (43, "Forty-three"),
        (44, "Forty-four"),
        (45, "Forty-five"),
        (46, "Forty-six"),
        (47, "Forty-seven"),
        (48, "Forty-eight"),
        (49, "Forty-nine"),
        (50, "Fifty"),
        (51, "Fifty-one"),
        (52, "Fifty-two"),
        (53, "Fifty-three"),
        (54, "Fifty-four"),
        (55, "Fifty-five"),
        (59, "Fifty-nine"),
    ];
    WORDS.iter().find(|(k, _)| *k == n).map(|(_, w)| *w)
}

#[test]
fn the_behaviour_header_routes_to_generated_run_coverage() {
    let sources = builder::load_sources(&PyPath::new(&repo_root().to_string_lossy())).unwrap();
    let published = read("tests/behaviour-tests.md");
    assert!(published.contains(&format!("The {} cases below", sources.behaviour.len())));
    assert!(published.contains("recorded coverage, conditions and verdicts in `evals/RESULTS.md`"));
    assert!(!published.contains("single recorded run covered"));
}

#[test]
fn the_readme_and_rerun_instruction_count_the_behaviour_cases_correctly() {
    // Adding a case has to move every copy of the number.
    let total = load_behaviour_tests(&root_py("src/data/behaviour-tests.yaml"))
        .unwrap()
        .len();
    let word = number_word(total)
        .unwrap_or_else(|| panic!("the suite has {total} cases, which number_word does not spell"));
    assert!(read("README.md").contains(&format!("{word} such cases are in")));
    let notes = read("EVALUATION-NOTES.md");
    assert!(notes.contains(&format!("Rerun all {total} scenarios")));
    assert!(notes.contains(&format!("submit all {total} scenarios now in the suite")));
}
