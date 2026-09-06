# DrDebits maintenance protocol

Part of [DrDebits](./drdebits.md) `0.3.3`.

A source-check date does not guarantee continuing currency. Before professional reliance, retrieve the authority operative for the relevant historical event or period and the current duties governing action today.

For each DrDebits release:

1. Check the latest TASA, TASR and Code Determination compilations on the Federal Register of Legislation, including uncommenced amendments and transitional provisions.
2. Recount and compare every results page in the filtered live TPB Guidance Statement catalogue; inspect issue and last-modified dates, especially statements changed since the previous release.
3. Check the APESB APES 110 landing page, compiled standard, compilation details, technical alerts and effective/transitional dates, and the current APES 220 issue and effective dates.
4. Check the operative AML/CTF Act text and the current AUSTRAC accountant guidance, transitional rules and compliance-officer requirements.
5. Reassess each DrDebits rule against the changed source. Do not update a date without reviewing the substantive effect.
6. Run `uv run --project tools/drdebits_build python -m drdebits_build verify` and review the latest weekly link-check workflow run; investigate any links it reported.
7. Confirm that no client data, credentials, proprietary prompt content or unauthorised APESB text entered the repository.
8. Edit the sources under `src/` (never the generated files), update `guide_version`, `release_tag`, the end marker, `sources_checked_at` and `review_due` in `src/data/metadata.yaml`, and add the release row in `src/data/changelog.yaml`. Then update every hand-written copy the build does not derive: the header version and source-check date lines in `src/guide/000-header.md`; the dates and the statement count and `GS` range in `src/guide/040-source-status.md`; the `GS` range in `src/guide/180-workpaper-record.md`; the review-date sentence in `src/guide/150-apes-110-control-set.md`; the README header date and the README `GS` range; and the `version` and `date-released` lines in `CITATION.cff`. Whenever `sources_checked_at` or that review date moves, update the assertions pinning them in `tools/drdebits_build/tests/test_repository_policy.py`; the suite fails otherwise. Run `uv run --project tools/drdebits_build python -m drdebits_build build`, then re-run `verify`. It cross-checks the version and date copies, requires `review_due` to still be in the future on the day it runs, and counts the `GS` range and statement-count copies in `drdebits.md` and `README.md` against the totals pinned in `tools/drdebits_build/drdebits_build/verify.py`, so adding or retiring one of those copies means updating that pin too.
9. Tag the release, sign the tag and the release commit, and record the material changes and release date in the change-log row. The signed tag itself records the approver and the approved revision.

If freshness cannot be confirmed, the LLM must label the affected material `SOURCE CURRENCY NOT CONFIRMED`, avoid calling it “latest” or “current”, and restrict the output to a draft requiring primary-source verification.

## Evaluation runs

The behaviour tests can be run against a model by hand and the outcome recorded without committing any prompt, output or transcript. The build owns every file under `evals/` except the result files a person writes.

1. `uv run --project tools/drdebits_build python -m drdebits_build build` writes `evals/cases.json`, the behaviour tests exported from `src/data/behaviour-tests.yaml` with the guide version they belong to. Give a case's scenario to the model under the guide loaded as project context and judge the observable output against `expected_status`, `required_behaviour` and `side_effect_check`, as `tests/behaviour-tests.md` requires. The person running the model decides pass or fail; nothing in this repository calls a model.
2. Record the run as `evals/results/YYYY-MM-DD-<slug>.json` with exactly these keys: `model`, `run_date`, `guide_version`, `runner` and `results`, where `results` maps case ids from `evals/cases.json` to `pass` or `fail`. A run may cover a subset of the cases; the table shows `n/a` where it did not. The four text fields are one line of at most 120 characters. Any other key, a duplicate key, a case id the current guide does not have, any other verdict, and any file under `evals/results/` that is not a result file fails the build and `verify`. A run recorded against an earlier guide keeps its record unchanged when a case is later added, renamed or retired. Keep prompts, outputs and transcripts outside the repository; `evals/transcripts/` is ignored for local working copies.
3. Run `build` again to regenerate `evals/RESULTS.md`, one row per case and one column per run headed by the model, the guide version the run recorded and the date, then `verify`. Commit the result file and the regenerated table together. The evaluation files are not part of the verified guide bundle and `SHA256SUMS` does not cover them.

## GitHub release checklist

For each future GitHub release:

1. Start from a clean worktree. Run the complete pytest suite, builder `verify`, a deterministic second build and the reviewed link-check result. Resolve every failure or classify every unreachable link before staging a release.
2. Confirm the verified guide bundle contains `LICENSE`, `README.md`, `CITATION.cff`, `drdebits.md`, `MAINTENANCE.md`, `reference/tpb-catalogue.md`, `reference/apes-110-map.md`, `tests/behaviour-tests.md` and the generated `SHA256SUMS` manifest. The manifest covers the first eight files and travels with them; it does not hash itself.
3. Create a signed release commit and a signed annotated tag that identify the approved revision.
4. Confirm immutable releases are enabled for the repository before creating the release. Create a GitHub draft release, stage every asset, and include the checksum and verification instructions.
5. Download every staged asset into a clean location while the release is still a draft. Independently verify the signed commit and tag, bundle membership, every `SHA256SUMS` entry and any available asset attestations.
6. Publish the release only after all verification succeeds. With immutable releases enabled, publication locks the tag and assets only after that pre-publication verification.
7. After publication, run `gh release verify TAG` and `gh release verify-asset TAG PATH` for each downloaded asset when immutable releases are enabled, and retain the verification record.
8. Do not overwrite or delete a published immutable release, and do not reuse its tag. Correct a released defect with a new version, new tag and a new release.

The existing `v0.1.0-draft`, `v0.2.0-draft`, `v0.3.0-draft` and `v0.3.1-draft` releases are historical published prereleases, not GitHub draft releases. They have no attached distribution assets. `v0.3.1` (published 21 August 2026) reused the `v0.3.1-draft` commit with no assets and no version bump, and rode a lightweight unsigned tag; `v0.3.2` supersedes it through the checklist above. No published tag before `v0.3.2` carries a signature, whatever earlier notes implied. Do not silently rewrite them; any superseding distribution must use a new tag and a new release.

Versioning convention:

- **Major:** a change to the control model, authority handling or human-decision boundaries.
- **Minor:** a new or materially revised source, rule, context module or behaviour test.
- **Patch:** a link repair, citation correction or wording change with no control effect.
