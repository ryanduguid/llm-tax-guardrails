//! Verification: tamper each way, expect the named failure (ported from test_verify.py).

mod common;

use std::path::Path;

use common::{fixture, make_repo};
use drdebits::builder;
use drdebits::py::{Date, PyPath};
use drdebits::verify::run_verify;

// The fixture's dates are fixed (checked 2026-01-01, review due 2026-04-01),
// so the date verify checks them against is fixed too.
fn today() -> Date {
    Date::fromisoformat("2026-02-01").unwrap()
}

fn verify_on(root: &PyPath, day: Date) -> Vec<String> {
    run_verify(root, day).expect("the checks run")
}

fn verify(root: &PyPath) -> Vec<String> {
    verify_on(root, today())
}

fn sync(root: &PyPath) {
    let s = builder::load_sources(root).unwrap();
    builder::write_outputs(root, root, &s).unwrap();
}

fn edit(path: &Path, old: &str, new: &str) {
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains(old), "{} lacks {old:?}", path.display());
    std::fs::write(path, text.replacen(old, new, 1)).unwrap();
}

fn append(path: &Path, extra: &str) {
    let text = std::fs::read_to_string(path).unwrap();
    std::fs::write(path, text + extra).unwrap();
}

fn any(failures: &[String], needle: &str) -> bool {
    failures.iter().any(|f| f.contains(needle))
}

#[test]
fn clean_repo_verifies() {
    let (_dir, root) = fixture();
    sync(&root);
    assert_eq!(verify(&root), Vec::<String>::new());
}

#[test]
fn malformed_source_key_is_reported_not_raised() {
    let (dir, root) = fixture();
    sync(&root);
    std::fs::write(
        dir.path().join("src/data/metadata.yaml"),
        "? [complex, key]\n: value\n",
    )
    .unwrap();
    let failures = verify(&root);
    assert!(any(&failures, "unhashable key"), "{failures:?}");
}

#[test]
fn tampered_output_names_file_and_src() {
    let (dir, root) = fixture();
    sync(&root);
    append(&dir.path().join("drdebits.md"), "tamper\n");
    assert!(
        verify(&root)
            .iter()
            .any(|f| f.contains("drdebits.md") && f.contains("edit src/"))
    );
}

#[test]
fn stamped_version_mismatch_detected() {
    let (dir, root) = fixture();
    sync(&root);
    std::fs::write(dir.path().join("README.md"), "> Version: `0.0.1`\n").unwrap();
    assert!(
        verify(&root)
            .iter()
            .any(|f| f.contains("README.md") && f.contains("0.0.1"))
    );
}

#[test]
fn bad_yaml_becomes_message_not_traceback() {
    let (dir, root) = fixture();
    sync(&root);
    let bt = dir.path().join("src/data/behaviour-tests.yaml");
    let text = std::fs::read_to_string(&bt)
        .unwrap()
        .replace("HARD_STOP", "NOT_A_STATUS");
    std::fs::write(&bt, text).unwrap();
    assert!(any(&verify(&root), "NOT_A_STATUS"));
}

#[test]
fn missing_stamped_file_detected() {
    let (dir, root) = fixture();
    sync(&root);
    std::fs::remove_file(dir.path().join("MAINTENANCE.md")).unwrap();
    assert!(any(&verify(&root), "MAINTENANCE.md: missing"));
}

#[test]
fn stripped_version_stamp_detected() {
    let (dir, root) = fixture();
    sync(&root);
    std::fs::write(
        dir.path().join("MAINTENANCE.md"),
        "Protocol for maintenance.\n",
    )
    .unwrap();
    assert!(any(
        &verify(&root),
        "MAINTENANCE.md: no version stamp found"
    ));
}

#[test]
fn committed_end_marker_drift_detected() {
    let (dir, root) = fixture();
    sync(&root);
    append(&dir.path().join("drdebits.md"), "extra line\n");
    assert!(any(&verify(&root), "end marker:"));
}

#[test]
fn missing_checksum_file_becomes_message_not_crash() {
    let (dir, root) = fixture();
    sync(&root);
    std::fs::remove_file(dir.path().join("README.md")).unwrap();
    assert!(any(&verify(&root), "README.md: missing"));
}

#[test]
fn header_version_line_mismatch_detected() {
    let (dir, root) = fixture();
    let header = dir.path().join("src/guide/000-header.md");
    let text = std::fs::read_to_string(&header)
        .unwrap()
        .replace("0.9.9-test", "0.9.8-test");
    std::fs::write(&header, text).unwrap();
    sync(&root);
    assert!(any(
        &verify(&root),
        "drdebits.md: header version line does not match guide_version"
    ));
}

#[test]
fn release_tag_mismatch_detected() {
    let (dir, root) = fixture();
    edit(
        &dir.path().join("src/data/metadata.yaml"),
        "value: \"v0.9.9-test\"",
        "value: \"v9.9.9-test\"",
    );
    sync(&root);
    assert!(any(
        &verify(&root),
        "metadata: release_tag v9.9.9-test != v0.9.9-test"
    ));
}

#[test]
fn guide_end_marker_metadata_mismatch_detected() {
    let (dir, root) = fixture();
    edit(
        &dir.path().join("src/data/metadata.yaml"),
        "value: \"DRDEBITS-END-v0.9.9-test\"",
        "value: \"DRDEBITS-END-BOGUS\"",
    );
    sync(&root);
    assert!(any(
        &verify(&root),
        "metadata: guide_end_marker DRDEBITS-END-BOGUS != DRDEBITS-END-v0.9.9-test"
    ));
}

fn drop_metadata_row(dir: &Path, key: &str, next: &str) {
    let meta = dir.join("src/data/metadata.yaml");
    let text = std::fs::read_to_string(&meta).unwrap();
    let start = text.find(&format!("  - key: {key}")).unwrap();
    let end = text.find(&format!("  - key: {next}")).unwrap();
    std::fs::write(&meta, format!("{}{}", &text[..start], &text[end..])).unwrap();
}

#[test]
fn missing_required_metadata_key_becomes_message_not_keyerror() {
    let (dir, root) = fixture();
    sync(&root);
    drop_metadata_row(dir.path(), "guide_version", "release_tag");
    assert!(
        verify(&root)
            .iter()
            .any(|f| f.contains("sources:") && f.contains("guide_version"))
    );
}

fn set_status(root: &Path, from: &str, to: &str) {
    edit(
        &root.join("src/data/metadata.yaml"),
        &format!("value: \"{from}\""),
        &format!("value: \"{to}\""),
    );
}

fn make_candidate_metadata(root: &Path) {
    append(
        &root.join("src/data/metadata.yaml"),
        "  - key: status\n    value: \"candidate\"\n",
    );
    label_copies(root, true);
}

// The published and candidate wording of each hand-written release-status
// copy, as MAINTENANCE prescribes them.
const RELEASE_COPIES: [(&str, &str, &str); 4] = [
    (
        "README.md",
        "> Release status: published. The signed tag is `v0.9.9-test`.",
        "> Release status: unreleased candidate. The proposed tag is `v0.9.9-test`.",
    ),
    (
        "llms.txt",
        "Release status: published; signed tag `v0.9.9-test`",
        "Release status: unreleased candidate; proposed tag `v0.9.9-test`",
    ),
    (
        "src/guide/000-header.md",
        "> Release status: published; signed tag `v0.9.9-test`",
        "> Release status: unreleased candidate; proposed tag `v0.9.9-test`",
    ),
    (
        "MAINTENANCE.md",
        "Part of [DrDebits](./drdebits.md) `0.9.9-test`.",
        "Part of the unreleased [DrDebits](./drdebits.md) `0.9.9-test` candidate.",
    ),
];

fn label_copies(root: &Path, candidate: bool) {
    for (rel, published, unreleased) in RELEASE_COPIES {
        let (old, new) = if candidate {
            (published, unreleased)
        } else {
            (unreleased, published)
        };
        edit(&root.join(rel), old, new);
    }
}

#[test]
fn unreleased_candidate_omits_release_date_and_requires_candidate_status() {
    for date_key in ["date-released", "\"date-released\"", "'date-released'"] {
        let (dir, root) = fixture();
        edit(
            &dir.path().join("src/data/changelog.yaml"),
            "status: \"Draft\"",
            "status: \"Unreleased\"",
        );
        make_candidate_metadata(dir.path());
        let citation = dir.path().join("CITATION.cff");
        std::fs::write(&citation, "cff-version: 1.2.0\nversion: 0.9.9-test\n").unwrap();
        sync(&root);
        assert_eq!(verify(&root), Vec::<String>::new());

        for value in ["\"2026-01-01\"", "not-a-date", ""] {
            std::fs::write(
                &citation,
                format!("cff-version: 1.2.0\nversion: 0.9.9-test\n{date_key}: {value}\n"),
            )
            .unwrap();
            assert!(
                verify(&root)
                    .iter()
                    .any(|f| f.contains("must omit date-released")),
                "{date_key}: {value}"
            );
        }
        std::fs::write(&citation, "cff-version: 1.2.0\nversion: 0.9.9-test\n").unwrap();
        set_status(dir.path(), "candidate", "current");
        sync(&root);
        assert!(
            verify(&root)
                .iter()
                .any(|f| f.contains("Unreleased changelog requires candidate status"))
        );
    }
}

#[test]
fn publishing_candidate_requires_an_actual_release_date_and_status() {
    let (dir, root) = fixture();
    make_candidate_metadata(dir.path());
    let citation = dir.path().join("CITATION.cff");
    std::fs::write(&citation, "cff-version: 1.2.0\nversion: 0.9.9-test\n").unwrap();
    sync(&root);
    let failures = verify(&root);
    assert!(
        failures
            .iter()
            .any(|f| f.contains("CITATION.cff: date-released"))
    );
    assert!(
        failures
            .iter()
            .any(|f| f.contains("candidate status requires Unreleased changelog"))
    );
    std::fs::write(
        &citation,
        "cff-version: 1.2.0\nversion: 0.9.9-test\ndate-released: 2026-01-01\n",
    )
    .unwrap();
    set_status(dir.path(), "candidate", "current");
    sync(&root);
    let failures = verify(&root);
    for rel in ["drdebits.md", "README.md", "llms.txt"] {
        assert!(
            any(
                &failures,
                &format!("{rel}: release status line does not mark `v0.9.9-test` as published")
            ),
            "{rel}: {failures:?}"
        );
    }
    assert!(
        any(&failures, "MAINTENANCE.md: first 'Part of' line is not"),
        "{failures:?}"
    );
    label_copies(dir.path(), false);
    sync(&root);
    assert_eq!(verify(&root), Vec::<String>::new());
}

#[test]
fn candidate_status_requires_every_copy_to_say_unreleased() {
    let (dir, root) = fixture();
    edit(
        &dir.path().join("src/data/changelog.yaml"),
        "status: \"Draft\"",
        "status: \"Unreleased\"",
    );
    append(
        &dir.path().join("src/data/metadata.yaml"),
        "  - key: status\n    value: \"candidate\"\n",
    );
    sync(&root);
    let failures = verify(&root);
    for rel in ["drdebits.md", "README.md", "llms.txt"] {
        assert!(
            any(
                &failures,
                &format!(
                    "{rel}: release status line does not mark `v0.9.9-test` as an unreleased candidate"
                )
            ),
            "{rel}: {failures:?}"
        );
    }
    assert!(
        any(&failures, "MAINTENANCE.md: first 'Part of' line is not"),
        "{failures:?}"
    );
    label_copies(dir.path(), true);
    sync(&root);
    assert_eq!(verify(&root), Vec::<String>::new());
}

#[test]
fn release_status_line_must_appear_once_and_name_the_tag() {
    let (dir, root) = fixture();
    make_candidate_metadata(dir.path());
    edit(
        &dir.path().join("src/data/changelog.yaml"),
        "status: \"Draft\"",
        "status: \"Unreleased\"",
    );
    sync(&root);
    assert_eq!(verify(&root), Vec::<String>::new());

    let llms = dir.path().join("llms.txt");
    edit(&llms, "proposed tag `v0.9.9-test`", "proposed tag `v0.9.8`");
    assert!(any(
        &verify(&root),
        "llms.txt: release status line does not mark"
    ));
    edit(&llms, "proposed tag `v0.9.8`", "proposed tag `v0.9.9-test`");
    append(
        &llms,
        "Release status: unreleased candidate; proposed tag `v0.9.9-test`\n",
    );
    assert!(any(
        &verify(&root),
        "llms.txt: found 2 'Release status:' lines, expected 1"
    ));
    edit(
        &dir.path().join("README.md"),
        "> Release status: unreleased candidate.",
        "> Status: unreleased candidate.",
    );
    assert!(any(
        &verify(&root),
        "README.md: found 0 'Release status:' lines, expected 1"
    ));
}

#[test]
fn release_status_phrase_must_open_the_value() {
    let (dir, root) = fixture();
    sync(&root);
    let llms = dir.path().join("llms.txt");
    edit(
        &llms,
        "Release status: published;",
        "Release status: unpublished;",
    );
    append(
        &dir.path().join("src/data/metadata.yaml"),
        "  - key: status\n    value: \"current\"\n",
    );
    std::fs::write(
        dir.path().join("CITATION.cff"),
        "cff-version: 1.2.0\nversion: 0.9.9-test\ndate-released: 2026-01-01\n",
    )
    .unwrap();
    sync(&root);
    assert!(any(
        &verify(&root),
        "llms.txt: release status line does not mark `v0.9.9-test` as published"
    ));
    edit(
        &llms,
        "Release status: unpublished;",
        "Release status: unreleased candidate (now published);",
    );
    assert!(any(
        &verify(&root),
        "llms.txt: release status line does not mark `v0.9.9-test` as published"
    ));
    edit(
        &llms,
        "Release status: unreleased candidate (now published);",
        "Release status: published;",
    );
    assert_eq!(verify(&root), Vec::<String>::new());
}

#[test]
fn maintenance_label_is_the_first_part_of_line() {
    let (dir, root) = fixture();
    append(
        &dir.path().join("src/data/metadata.yaml"),
        "  - key: status\n    value: \"current\"\n",
    );
    std::fs::write(
        dir.path().join("CITATION.cff"),
        "cff-version: 1.2.0\nversion: 0.9.9-test\ndate-released: 2026-01-01\n",
    )
    .unwrap();
    sync(&root);
    assert_eq!(verify(&root), Vec::<String>::new());
    let maintenance = dir.path().join("MAINTENANCE.md");
    let text = std::fs::read_to_string(&maintenance).unwrap();
    std::fs::write(
        &maintenance,
        format!(
            "Part of the unreleased [DrDebits](./drdebits.md) `0.9.9-test` candidate.\n\n{text}"
        ),
    )
    .unwrap();
    assert!(any(
        &verify(&root),
        "MAINTENANCE.md: first 'Part of' line is not"
    ));
}

#[test]
fn status_outside_candidate_and_current_skips_the_release_copies() {
    let (dir, root) = fixture();
    append(
        &dir.path().join("src/data/metadata.yaml"),
        "  - key: status\n    value: \"superseded\"\n",
    );
    label_copies(dir.path(), true);
    sync(&root);
    assert!(!any(&verify(&root), "release status"));
}

#[test]
fn maintenance_bundle_counts_match_the_manifest_and_installer() {
    let (dir, root) = fixture();
    sync(&root);
    let maintenance = dir.path().join("MAINTENANCE.md");
    for (old, new, needle) in [
        (
            "covers the first 3 files",
            "covers the first 11 files",
            "checklist manifest count is 11, expected 3",
        ),
        (
            "hashes all 3",
            "hashes all 11",
            "deployment manifest count is 11, expected 3",
        ),
        (
            "accepts the 8\nrequired",
            "accepts the 7\nrequired",
            "installer required count is 7, expected 8",
        ),
        (
            "subset of the 4 optional",
            "subset of the 3 optional",
            "installer optional count is 3, expected 4",
        ),
    ] {
        edit(&maintenance, old, new);
        assert!(any(&verify(&root), needle), "{needle}");
        edit(&maintenance, new, old);
    }
    assert_eq!(verify(&root), Vec::<String>::new());
    edit(&maintenance, "hashes all 3 files", "hashes all 3,000 files");
    assert!(any(
        &verify(&root),
        "found 0 copies of the deployment manifest count"
    ));
    edit(&maintenance, "hashes all 3,000 files", "hashes all 3 files");
    edit(&maintenance, "hashes all 3", "hashes every file");
    assert!(any(
        &verify(&root),
        "found 0 copies of the deployment manifest count"
    ));
    edit(
        &maintenance,
        "hashes every file",
        "hashes all 3 files; the manifest hashes all 3",
    );
    assert!(any(
        &verify(&root),
        "found 2 copies of the deployment manifest count"
    ));
}

#[test]
fn published_citation_accepts_quoted_mapping_keys() {
    for quote in ["\"", "'"] {
        let (dir, root) = fixture();
        std::fs::write(
            dir.path().join("CITATION.cff"),
            format!(
                "cff-version: 1.2.0\n{quote}version{quote}: 0.9.9-test\n{quote}date-released{quote}: \"2026-01-01\"\n"
            ),
        )
        .unwrap();
        sync(&root);
        assert_eq!(verify(&root), Vec::<String>::new(), "{quote}");
    }
}

#[test]
fn an_unreadable_or_too_deep_citation_is_a_finding() {
    let (dir, root) = fixture();
    sync(&root);
    let citation = dir.path().join("CITATION.cff");
    std::fs::write(&citation, b"version: \xff\n").unwrap();
    assert!(
        verify(&root).contains(
            &"CITATION.cff: cannot parse YAML ('utf-8' codec can't decode byte 0xff in position 9: \
              invalid start byte)"
                .to_string()
        ),
        "{:?}",
        verify(&root)
    );
    let depth = 2000;
    std::fs::write(
        &citation,
        format!("version: {}{}\n", "[".repeat(depth), "]".repeat(depth)),
    )
    .unwrap();
    let failures = common::big_stack(move || verify(&root));
    assert!(
        failures
            .contains(&"CITATION.cff: cannot parse YAML (maximum recursion depth exceeded)".into()),
        "{failures:?}"
    );
}

#[test]
fn a_citation_error_python_did_not_catch_stops_verify() {
    // PyYAML's KeyError for an unknown boolean escaped the citation check,
    // so verify reported that it could not run.
    let (dir, root) = fixture();
    sync(&root);
    std::fs::write(dir.path().join("CITATION.cff"), "version: !!bool maybe\n").unwrap();
    assert_eq!(
        run_verify(&root, today()),
        Err(drdebits::error::Fail::Internal("KeyError: 'maybe'".into()))
    );
}

#[test]
fn a_release_date_that_is_neither_text_nor_a_date_does_not_match() {
    let (dir, root) = fixture();
    std::fs::write(
        dir.path().join("CITATION.cff"),
        "version: 0.9.9-test\ndate-released: 7\n",
    )
    .unwrap();
    sync(&root);
    assert!(
        verify(&root)
            .iter()
            .any(|f| f.starts_with("CITATION.cff: date-released does not match")),
        "{:?}",
        verify(&root)
    );
}

#[test]
fn malformed_or_duplicate_citation_is_a_finding_not_a_crash() {
    for document in [
        "[not, a, mapping]\n",
        "version: [unfinished\n",
        "version: 0.9.9-test\nversion: 0.9.9-test\n",
        "version: 0.9.9-test\ndate-released: \"1999-01-01\"\n\"date-released\": \"2026-01-01\"\n",
        "version: 0.9.9-test\ndate-released: 2026-02-30\n",
    ] {
        let (dir, root) = fixture();
        std::fs::write(dir.path().join("CITATION.cff"), document).unwrap();
        sync(&root);
        assert!(
            verify(&root).iter().any(|f| f.starts_with("CITATION.cff:")),
            "{document:?}"
        );
    }
}

#[test]
fn citation_version_and_date_drift_detected() {
    let (dir, root) = fixture();
    sync(&root);
    let citation = dir.path().join("CITATION.cff");
    std::fs::write(
        &citation,
        "cff-version: 1.2.0\nversion: 0.9.9-test\ndate-released: \"2026-01-01\"\n",
    )
    .unwrap();
    assert_eq!(verify(&root), Vec::<String>::new());
    std::fs::write(
        &citation,
        "cff-version: 1.2.0\nversion: \"0.9.9-test\"\ndate-released: 2026-01-01\n",
    )
    .unwrap();
    assert_eq!(
        verify(&root),
        Vec::<String>::new(),
        "quoting style is not load-bearing"
    );
    std::fs::write(
        &citation,
        "cff-version: 1.2.0\nversion: 0.1.0-stale\ndate-released: \"2026-01-01\"\n",
    )
    .unwrap();
    assert!(any(&verify(&root), "CITATION.cff: version"));
    std::fs::write(
        &citation,
        "cff-version: 1.2.0\nversion: 0.9.9-test\ndate-released: \"1999-01-01\"\n",
    )
    .unwrap();
    assert!(any(&verify(&root), "CITATION.cff: date-released"));
}

#[test]
fn malformed_sources_checked_at_becomes_message_not_traceback() {
    let (dir, root) = fixture();
    sync(&root);
    edit(
        &dir.path().join("src/data/metadata.yaml"),
        "value: \"2026-01-01T00:00:00+10:00\"",
        "value: \"checked mid August\"",
    );
    assert!(any(&verify(&root), "sources_checked_at"));
}

#[test]
fn source_check_date_drift_detected() {
    let (dir, root) = fixture();
    edit(
        &dir.path().join("src/data/metadata.yaml"),
        "value: \"2026-01-01T00:00:00+10:00\"",
        "value: \"2026-02-02T00:00:00+10:00\"",
    );
    sync(&root);
    let failures = verify(&root);
    assert!(any(&failures, "drdebits.md: header source-check date"));
    assert!(any(&failures, "README.md: source-check date"));
}

#[test]
fn review_due_not_after_source_check_date_detected() {
    let (dir, root) = fixture();
    let meta = dir.path().join("src/data/metadata.yaml");
    let original = std::fs::read_to_string(&meta).unwrap();
    std::fs::write(
        &meta,
        original.replace(
            "value: \"2026-01-01T00:00:00+10:00\"",
            "value: \"2026-06-01T00:00:00+10:00\"",
        ),
    )
    .unwrap();
    sync(&root);
    assert!(any(
        &verify(&root),
        "metadata: review_due 2026-04-01 is not after sources_checked_at 2026-06-01"
    ));
    // Same day is not "after".
    std::fs::write(
        &meta,
        original.replace("value: \"2026-04-01\"", "value: \"2026-01-01\""),
    )
    .unwrap();
    sync(&root);
    assert!(any(&verify(&root), "review_due 2026-01-01 is not after"));
    std::fs::write(
        &meta,
        original.replace("value: \"2026-04-01\"", "value: \"next quarter\""),
    )
    .unwrap();
    sync(&root);
    assert!(any(&verify(&root), "review_due must be an ISO date"));
}

#[test]
fn review_due_already_passed_at_verification_detected() {
    let (_dir, root) = fixture();
    sync(&root);
    let day = |s: &str| Date::fromisoformat(s).unwrap();
    // Still current the day before it falls due.
    assert_eq!(verify_on(&root, day("2026-03-31")), Vec::<String>::new());
    // Due today is already due.
    assert!(any(
        &verify_on(&root, day("2026-04-01")),
        "metadata: review_due 2026-04-01 is not after the verification date 2026-04-01"
    ));
    // And months past due, with both metadata dates still agreeing.
    let failures = verify_on(&root, day("2026-09-15"));
    assert!(any(
        &failures,
        "metadata: review_due 2026-04-01 is not after the verification date 2026-09-15"
    ));
    assert!(any(&failures, "SOURCE CURRENCY NOT CONFIRMED"));
    assert!(!any(&failures, "is not after sources_checked_at"));
}

#[test]
fn missing_review_due_becomes_message_not_silent_pass() {
    let (dir, root) = fixture();
    sync(&root);
    drop_metadata_row(dir.path(), "review_due", "checksum_files");
    assert!(
        verify(&root)
            .iter()
            .any(|f| f.contains("sources:") && f.contains("review_due"))
    );
}

#[test]
fn tpb_statement_count_and_range_drift_detected() {
    let (dir, root) = fixture();
    sync(&root);
    assert_eq!(verify(&root), Vec::<String>::new());
    append(
        &dir.path().join("src/data/tpb-catalogue.yaml"),
        "  - id: \"GS02\"\n    title: \"T2\"\n    url: \"https://x.invalid/b\"\n    trigger: \"g\"\n",
    );
    let meta = dir.path().join("src/data/metadata.yaml");
    let text = std::fs::read_to_string(&meta)
        .unwrap()
        .replace("value: \"1\"", "value: \"2\"");
    std::fs::write(&meta, text).unwrap();
    sync(&root);
    let failures = verify(&root);
    assert!(any(
        &failures,
        "drdebits.md: statement range 'GS01 to GS01'"
    ));
    assert!(any(
        &failures,
        "drdebits.md: statement count 1 does not match"
    ));
    assert!(any(&failures, "README.md: statement range 'GS01 to GS01'"));
}

#[test]
fn tpb_copy_that_stops_matching_is_a_failure_not_a_silent_pass() {
    // Check (h) must not pass vacuously: only the copy count betrays a copy
    // reworded out of the patterns.
    let sources = "src/guide/020-sources.md";
    let cases = [
        (
            sources,
            "1 indexed statements, GS01 to GS01",
            "1 indexed statements, GS01–GS01",
            "drdebits.md: found 2 statement range copies, expected 3",
        ),
        (
            sources,
            "catalogue, GS01 to GS01",
            "catalogue, GS01 through GS01",
            "drdebits.md: found 2 statement range copies, expected 3",
        ),
        (
            sources,
            "exposed 1 live Guidance Statements",
            "exposed a total of 1 statements",
            "drdebits.md: found 1 statement count copies, expected 2",
        ),
        (
            sources,
            "- reference/tpb-catalogue.md: complete live TPB Guidance Statement catalogue, GS01 to GS01\n",
            "",
            "drdebits.md: found 2 statement range copies, expected 3",
        ),
        (
            "README.md",
            "catalogue, GS01 to GS01",
            "catalogue, GS01–GS01",
            "README.md: found 0 statement range copies, expected 1",
        ),
    ];
    for (rel, old, new, expected) in cases {
        let dir = tempfile::tempdir().unwrap();
        let root = make_repo(dir.path());
        edit(&dir.path().join(rel), old, new);
        sync(&root);
        let failures = verify(&root);
        assert!(any(&failures, expected), "{expected}: {failures:?}");
        assert!(any(&failures, "MAINTENANCE step 8"));
    }
}

#[test]
fn changelog_newest_entry_mismatch_detected() {
    let (dir, root) = fixture();
    edit(
        &dir.path().join("src/data/changelog.yaml"),
        "version: \"0.9.9-test\"",
        "version: \"0.9.8-test\"",
    );
    sync(&root);
    assert!(any(
        &verify(&root),
        "changelog: newest entry 0.9.8-test != 0.9.9-test"
    ));
}

#[test]
fn apes_locator_cited_by_the_control_set_but_absent_from_the_map() {
    let (dir, root) = fixture();
    sync(&root);
    assert_eq!(verify(&root), Vec::<String>::new());
    append(
        &dir.path().join("src/guide/150-apes-110-control-set.md"),
        "\nApply sections 260 and 360 and Part 4A to the engagement.\n",
    );
    sync(&root);
    let failures = verify(&root);
    for needle in [
        "cites APES 110 section 260, which",
        "cites APES 110 section 360, which",
        "cites APES 110 Part 4A, which",
    ] {
        assert!(any(&failures, needle), "{needle}: {failures:?}");
    }
    // The locator the map carries is not reported, and a paragraph reference
    // is not a Part or section citation.
    assert!(
        !any(&failures, "Part 1,") && !any(&failures, "R1.2"),
        "{failures:?}"
    );
}

#[test]
fn missing_apes_control_set_fragment_is_a_failure_not_a_silent_pass() {
    let (dir, root) = fixture();
    sync(&root);
    std::fs::rename(
        dir.path().join("src/guide/150-apes-110-control-set.md"),
        dir.path().join("src/guide/150-apes.md"),
    )
    .unwrap();
    sync(&root);
    assert!(any(&verify(&root), "150-apes-110-control-set.md: missing"));
}

#[test]
fn behaviour_test_status_the_guide_does_not_define() {
    let (dir, root) = fixture();
    sync(&root);
    assert_eq!(verify(&root), Vec::<String>::new());
    std::fs::write(
        dir.path()
            .join("src/guide/060-meaning-of-instruction-words.md"),
        "## Words\n\n- **ESCALATE** means refer the matter.\n",
    )
    .unwrap();
    sync(&root);
    assert_eq!(
        verify(&root),
        [
            "behaviour test A-001: expected status 'HARD_STOP' is not defined in src/guide/060-meaning-of-instruction-words.md"
        ]
    );
}

#[test]
fn missing_instruction_words_fragment_is_a_failure_not_a_silent_pass() {
    let (dir, root) = fixture();
    sync(&root);
    std::fs::rename(
        dir.path()
            .join("src/guide/060-meaning-of-instruction-words.md"),
        dir.path().join("src/guide/060-words.md"),
    )
    .unwrap();
    sync(&root);
    assert!(any(
        &verify(&root),
        "060-meaning-of-instruction-words.md: missing"
    ));
}

#[test]
fn a_prohibited_conclusion_label_the_sources_do_not_define() {
    let (dir, root) = fixture();
    sync(&root);
    assert_eq!(verify(&root), Vec::<String>::new());
    append(
        &dir.path().join("src/data/behaviour-tests.yaml"),
        "    prohibited_anywhere:\n      - \"invented label\"\n",
    );
    sync(&root);
    assert_eq!(
        verify(&root),
        [
            "behaviour test A-001: prohibited_anywhere names 'invented label', which src/data/behaviour-tests.yaml does not \
          define under prohibited_conclusions"
        ]
    );
}

#[test]
fn a_missing_committed_guide_is_reported_once() {
    let (dir, root) = fixture();
    sync(&root);
    std::fs::remove_file(dir.path().join("drdebits.md")).unwrap();
    let failures = verify(&root);
    assert!(failures.contains(&"drdebits.md: missing committed file".to_string()));
    // The guide's own content checks skip a file that is not there.
    assert!(
        !failures
            .iter()
            .any(|f| f.starts_with("end marker") || f.contains("header"))
    );
}

#[test]
fn the_real_repository_verifies_inside_its_review_window() {
    let tree = common::Tree::new();
    assert_eq!(
        verify_on(&tree.root(), common::inside_window()),
        Vec::<String>::new()
    );
}
