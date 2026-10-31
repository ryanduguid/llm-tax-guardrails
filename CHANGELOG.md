# Changelog

All notable changes to this repository are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Guide versions use the
versioning convention in [MAINTENANCE.md](MAINTENANCE.md); the guide's own
change log in `src/data/changelog.yaml` remains the record for each guide
release.

## [Unreleased]

## [v0.5.0] - 2026-10-31

Changes released in `0.5.0`, merged since `v0.4.0` (28 October 2026). Sources were not rechecked: they stay as checked on 14 October 2026.

### Added

- `verify` checks that the release-status lines in the guide header, README and `llms.txt`, and the first line of `MAINTENANCE.md`, agree with the metadata `status` and release tag, and that the bundle file counts `MAINTENANCE.md` states match `checksum_files` and the installer's lists (#211).
- `gate --facts FILE` asks for review, never a block, of each dollar amount or percentage in a draft that is neither a supplied fact nor the result of a calculation the draft shows; it flags untraced amounts and cannot certify traced ones (#215).
- A conditional automated-decision transparency check in the data and privacy gate for APP 1.7 to 1.9 of the *Privacy Act 1988*, which start on 10 December 2026; vendor checklist question VA-37 asking for the APP 1.8 high-level information; and a matching row in the AI register entry. Each leaves the legal conclusion to the firm, and the check does not stop a task (#217).
- Eight behaviour tests, bringing the suite to 67: output-side TFN and cross-client leak cases (DATA-003 to DATA-007) and cases where a draft flags a fact and then assumes it (ASSUME-001 to ASSUME-003) (#219).
- A benchmark canary string and do-not-train line in the evaluation files, and a mapping of the CITE cases to the categories of a public database of AI hallucination cases (#219).

### Changed

- The prohibited-text screen in the builder ignores case, whitespace, hyphen and quote variants, so a marker wrapped across lines is caught (#219).
- README badges are served from the repository so they render through GitHub's image proxy, and the banners show brand version 8 (#216, #218, #220).
- The `commit-msg` hook runs its Python in isolated mode, and its header no longer recommends `core.hooksPath=.githooks`, which would run hooks from whichever branch is checked out (#213).

Not listed, as they have no user-visible effect: Clippy-driven changes to builder parameter types (#212, #214).

## [v0.4.0] - 2026-10-28

Changes released in `0.4.0`, merged since `v0.3.3` (6 September 2026).

### Added

- AI tool and vendor assurance checklist, `reference/ai-vendor-assurance.md` (#46).
- Vendor checklist citations of the Guidance for AI Adoption practices (#104), contract-terms checks (#108), APES 320 and the OAIC AI-product guidance (#116), and ISO/IEC 42001 certificate checks with supplier details for a firm's AI register (#121).
- TPB(GS) 55/2026 concordance, `reference/tpb-gs55-concordance.md`, naming the permission forms and contest process (#112).
- TPB sanction controls, with the s 45 row corrected for 1 October 2026 (#105).
- Behaviour tests: eight calculator cases (#76); five cases with the whole-response rule (#82); permitted workpaper completion alongside refusal boundaries (#84); four citation cases (#108); unpublished-figure, draft-ruling, period, registration and review-risk cases (#170).
- Source-currency check against the Federal Register for every pinned compilation, run weekly (#67).
- Source-currency report of registered amendments that come into force within 120 days (#106).
- Bundle installer and verifier for complete guide installations against an approved digest (#80, #102).
- First full behavioural evaluation record (#41) and Codex evaluation observations (#50).
- Evaluation records bound to the guide commit, runtime, tools and input digests (#74, #85, #91), with an inert tool harness for action-boundary tests (#82).
- Case-by-case comparison of recorded runs (#140) and quotation assessment with explicit source coverage (#139).
- Marker hits printed after scoring, with the screen's literal-phrase limit stated (#109).
- `CONTRIBUTING.md` with the change workflow and the checks CI runs (#165).
- `CLAUDE.md` and project settings that protect generated outputs (#122), and maintenance routing in `AGENTS.md` (#49).
- `CHANGELOG.md`, this file (#178).
- TPB Annual Report 2025-26 context in the source-status chapter, kept apart from the dated Guidance Statement count (#202).
- Builder `gate` command that checks a high-impact draft against the output contract, including the decision status, action authority, mandatory sections and reperformed arithmetic (#208).
- Builder test that a model returning `HARD_STOP` for everything fails every case expecting another status (#201).

### Changed

- Decision statuses and evidence requirements clarified after evaluation failures (#42).
- Instruction handling and source verification clarified (#54); source mappings and evidence boundaries clarified (#56).
- Prohibited conclusions held in hypotheticals (#80).
- The 1 October 2026 TASA and Code of Professional Conduct amendments flagged in the guide (#103), with a warning for `v0.3.3` users (#107).
- Pinned and reviewed compilations: Code Determination Compilation 4 (#125, #132), TASA Compilation 27 (#141), TASR Compilation 5 (#148), AML/CTF Act Compilation 63 (#177) and Taxation Administration Act Compilation 227 (#187).
- Sources reviewed on 14 October 2026 (#168); `0.4.0` prepared as an unreleased candidate (#169, #174).
- AUSTRAC guidance reviewed monthly, with the next review date named (#110).
- Private source review register requirement removed (#129).
- Verifier separates a failed check (exit 1) from a verifier that could not run (exit 2) (#68).
- Guide data cross-checked and `llms.txt` stamped from the build (#55, #126).
- Source provenance and quotation checks strengthened (#172).
- Prose aligned with the Australian Government Style Manual (#60), hand-written prose tightened (#40) and redundant disclaimers trimmed (#176).
- Builder ported to Rust: `drdebits_build` 1.0.0 replaces the Python package with the same generated files, findings and exit codes, apart from the divergences listed in `tools/drdebits_build/CHANGELOG.md`. CI runs `cargo fmt`, `clippy`, the tests on Ubuntu and Windows and line coverage (#192, #205, #206, #209). It replaces the interim Python 3.14 requirement (#133, #145).
- Release checklist status of `v0.3.2` stated accurately (#61); maintenance guidance for evaluation tools and bundle contents corrected (#114).
- README and diagrams recoloured to the website palette (#136, #137, #142, #143, #155, #160, #173), with README badge styling flattened and centred (#188, #191), a phone layout for the banner (#196), a brand footer (#200) and an opening line naming the guide and repository together (#190).
- Contributor workflow: pull requests and pushed commits carrying an AI authorship credit are refused (#47, #70, #72).
- The build tooling under `tools/` and the CI helper scripts under `.github/ci/` are licensed under FSL-1.1-ALv2 (Functional Source License, with Apache-2.0 as the future licence) instead of MIT, and the guide under FSL-1.1-ALv2 instead of CC BY 4.0. A root NOTICE joins the verified bundle. Released versions keep their licence (#183, #193, #199).

### Fixed

- AUSTRAC link repaired (#103).
- Link checker checks destinations before requesting them, and the build fails before it writes (#95).
- Link checker closes HTTP error responses (#131).
- Links on hosts that block GitHub's runners are listed for checking by hand instead of making the check incomplete (#120).
- Register records without a compilation reported as incomplete rather than misread (#124, #146).
- Evaluation evidence corrected (#102); impossible evaluation dates reported as source findings (#147).
- Disclaimer set phrase and its checksum corrected (#127).
- Attribution title matching (#52).
- Flaky concurrency probe in the test suite (#64, #65).
- The ATO advice-and-guidance page, which refuses GitHub's runners, is listed for checking by hand instead of making the link check incomplete (#178).

### Security

- Secrets and client identifiers gated locally and in CI with gitleaks and `.gitleaks.toml` (#67).
- Commit identities validated before merging (#51).
- YAML source reader fuzzed, and deeply nested YAML refused (#161).
- Two regular-expression backtracking risks removed (#162).
- Hash-pinned installs and a security policy link (#156); issue-writing jobs install only locked wheels (#164).
- Verify and CodeQL enforced through self-checking aggregate jobs (#115); required gate checks kept when a workflow or gate is renamed (#203), using the canonical gate checker, which fails on drift (#204).
- uv raised to 0.13.5 in CI for GHSA-hq85-5cpj-mr62 (#197).
- Locked dependencies tested with Snyk (#153), OpenSSF Scorecard results published (#151) and release assets scanned with VirusTotal (#150).

Not listed, as they have no user-visible effect: dependency and action pin
updates (#43, #44, #66, #86, #87, #88, #89, #90, #93, #117, #118, #119, #171, #179, #180, #181, #182, #194, #195),
attribution-policy pin moves (#53, #59, #78, #79, #100, #175), review-bot
configuration (#39, #57, #58, #62, #63, #97, #98, #128), badges and coverage
reporting (#99, #144, #149, #152, #154, #157, #158, #166, #167), and CI and
documentation housekeeping (#48, #101, #111, #135, #184, #185, #186, #189, #198,
#207).

[Unreleased]: https://github.com/ryanduguid/llm-tax-guardrails/compare/v0.5.0...HEAD
[v0.5.0]: https://github.com/ryanduguid/llm-tax-guardrails/compare/v0.4.0...v0.5.0
[v0.4.0]: https://github.com/ryanduguid/llm-tax-guardrails/compare/v0.3.3...v0.4.0
