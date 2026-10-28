//! Command-line behaviour through the binary (ported from test_cli.py).
//!
//! The binary reads the real clock, so these tests move the fixture's review
//! date far ahead instead of pinning the date verify uses.

mod common;

use common::*;
use drdebits::builder;
use drdebits::verify::run_verify;

const VERIFY_OK_LINE: &str =
    "verify: OK (structure, version copies and review window; not a source-currency check)";

fn run(dir: &std::path::Path, args: &[&str]) -> Output {
    run_in(dir, args, true)
}

fn snapshot(dir: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    fn walk(dir: &std::path::Path, out: &mut Vec<(std::path::PathBuf, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.push((path.clone(), std::fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out.sort();
    out
}

#[test]
fn build_then_verify_roundtrip() {
    let (dir, _root) = fixture();
    far_review(dir.path());
    assert_eq!(run(dir.path(), &["build"]).code, 0);
    let verify = run(dir.path(), &["verify"]);
    assert_eq!(verify.code, 0, "{}", verify.stderr);
    // The success line says what was checked.
    assert_eq!(verify.stdout, format!("{VERIFY_OK_LINE}\n"));
}

#[test]
fn digests_prints_the_two_record_digests() {
    let (dir, root) = fixture();
    assert_eq!(run(dir.path(), &["build"]).code, 0);
    let out = run(dir.path(), &["digests"]);
    assert_eq!(out.code, 0);
    let printed: Vec<(String, String)> = out
        .stdout
        .lines()
        .map(|l| {
            let (k, v) = l.split_once(": ").unwrap();
            (k.to_string(), v.to_string())
        })
        .collect();
    let expected: Vec<(String, String)> =
        builder::build_digests(&builder::load_sources(&root).unwrap())
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
    assert_eq!(printed, expected);
    assert!(printed.iter().all(|(_, d)| d.len() == 64));
}

#[test]
fn verify_failure_exit_code() {
    let (dir, _root) = fixture();
    far_review(dir.path());
    run(dir.path(), &["build"]);
    std::fs::write(dir.path().join("drdebits.md"), "tampered").unwrap();
    assert_eq!(run(dir.path(), &["verify"]).code, 1);
}

#[test]
fn impossible_evaluation_date_is_a_finding_for_build_and_verify() {
    let (dir, _root) = fixture();
    far_review(dir.path());
    assert_eq!(run(dir.path(), &["build"]).code, 0);
    write_result(dir.path(), "2026-99-99-example-model.json", &[]);
    let before = snapshot(dir.path());
    for command in ["build", "verify"] {
        let out = run(dir.path(), &[command]);
        assert_eq!(out.code, 1, "{command}");
        assert!(
            out.stderr
                .contains("evals/results/2026-99-99-example-model.json")
        );
        assert!(out.stderr.contains("file name date must be an ISO date"));
        assert_eq!(snapshot(dir.path()), before);
    }
}

#[test]
fn missing_root_reports_cleanly_for_both_commands() {
    // No root means the checks never started: verify exits 2, build 1.
    let dir = tempfile::tempdir().unwrap();
    for (command, expected) in [("verify", 2), ("build", 1)] {
        let out = run_in(dir.path(), &[command], false);
        assert_eq!(out.code, expected);
        assert!(
            out.stderr
                .starts_with(&format!("{command}: no DrDebits root found above ")),
            "{}",
            out.stderr
        );
    }
}

#[test]
fn build_stamps_every_hand_written_file_before_checksums() {
    let (dir, root) = fixture();
    let p = dir.path();
    std::fs::write(
        p.join("llms.txt"),
        "# G\n\nVersion: `0.0.1`\nSources last checked: 2026-01-01\n",
    )
    .unwrap();
    std::fs::write(
        p.join("README.md"),
        "> Version: `0.0.1`\n\nSources last checked: 2026-01-01\n\nInstall uv `0.12.0` before building.\n\n\
         | catalogue | Complete live TPB Guidance Statement catalogue, GS01 to GS01 |\n",
    )
    .unwrap();
    std::fs::write(
        p.join("MAINTENANCE.md"),
        "Part of [DrDebits](./drdebits.md) `0.0.1`.\n\n\
         The manifest covers the first 3 files. The installer accepts the 8\n\
         required files plus any subset of the 4 optional files. The current\n\
         manifest hashes all 3 files.\n",
    )
    .unwrap();
    assert_eq!(run(p, &["build"]).code, 0);
    let readme = std::fs::read_to_string(p.join("README.md")).unwrap();
    assert!(readme.contains("> Version: `0.9.9-test`") && readme.contains("`0.12.0`"));
    assert!(
        std::fs::read_to_string(p.join("MAINTENANCE.md"))
            .unwrap()
            .contains("0.9.9-test")
    );
    assert!(
        std::fs::read_to_string(p.join("llms.txt"))
            .unwrap()
            .contains("Version: `0.9.9-test`")
    );
    let day = drdebits::py::Date::fromisoformat("2026-02-01").unwrap();
    assert_eq!(run_verify(&root, day).unwrap(), Vec::<String>::new());
    // The source-check date is hand-written in llms.txt, not stamped.
    std::fs::write(
        p.join("llms.txt"),
        "# G\n\nVersion: `0.9.9-test`\nSources last checked: 2025-12-31\n",
    )
    .unwrap();
    assert_eq!(
        run_verify(&root, day).unwrap(),
        ["llms.txt: source-check date does not match sources_checked_at (2026-01-01)"]
    );
}

#[test]
fn verify_reports_a_broken_checker_as_2_not_1() {
    // A checker that falls over must not read as a checker that found a
    // fault. An undecodable guide fragment escaped the Python checks as an
    // exception; it still exits 2 here.
    let (dir, _root) = fixture();
    far_review(dir.path());
    run(dir.path(), &["build"]);
    std::fs::write(dir.path().join("src/guide/999-broken.md"), b"\xff\n").unwrap();
    let out = run(dir.path(), &["verify"]);
    assert_eq!(out.code, 2);
    assert!(
        out.stderr.contains("VERIFIER ERROR") && out.stderr.contains("unverified, not verified")
    );
}

#[test]
fn unreadable_sources_stay_a_finding() {
    let (dir, _root) = fixture();
    run(dir.path(), &["build"]);
    std::fs::write(dir.path().join("src/data/metadata.yaml"), "{ not: [valid").unwrap();
    assert_eq!(run(dir.path(), &["verify"]).code, 1);
}

#[test]
fn a_malformed_evaluation_run_leaves_the_worktree_unchanged() {
    let (dir, _root) = fixture();
    let p = dir.path();
    std::fs::write(
        p.join("llms.txt"),
        "# G\n\nVersion: `0.0.1`\nSources last checked: 2026-01-01\n",
    )
    .unwrap();
    let before: Vec<Vec<u8>> = ["llms.txt", "README.md", "MAINTENANCE.md"]
        .iter()
        .map(|r| std::fs::read(p.join(r)).unwrap())
        .collect();
    std::fs::create_dir_all(p.join("evals/results")).unwrap();
    std::fs::write(p.join("evals/results/not-a-result.txt"), "junk\n").unwrap();
    assert_eq!(run(p, &["build"]).code, 1);
    for (rel, data) in ["llms.txt", "README.md", "MAINTENANCE.md"]
        .iter()
        .zip(before)
    {
        assert_eq!(std::fs::read(p.join(rel)).unwrap(), data, "{rel}");
    }
}

#[test]
fn invalid_bytes_in_a_source_file_report_the_path() {
    let (dir, _root) = fixture();
    std::fs::write(
        dir.path().join("src/data/metadata.yaml"),
        b"meta:\n  guide_version: \xff\xfe bad\n",
    )
    .unwrap();
    let out = run(dir.path(), &["build"]);
    assert_eq!(out.code, 1);
    assert!(
        out.stderr.contains("metadata.yaml: is not valid UTF-8"),
        "{}",
        out.stderr
    );
}

#[test]
fn build_skips_a_stamped_file_the_tree_does_not_have() {
    let (dir, _root) = fixture();
    std::fs::remove_file(dir.path().join("llms.txt")).unwrap();
    let out = run(dir.path(), &["build"]);
    assert_eq!(out.code, 0);
    assert!(out.stdout.contains("stamped README.md") && !out.stdout.contains("stamped llms.txt"));
}

#[test]
fn digests_reports_an_unbuildable_tree() {
    let (dir, _root) = fixture();
    for entry in std::fs::read_dir(dir.path().join("src/guide")).unwrap() {
        std::fs::remove_file(entry.unwrap().path()).unwrap();
    }
    let out = run(dir.path(), &["digests"]);
    assert_eq!(out.code, 1);
    assert_eq!(out.stderr, "digests: src/guide/ has no fragments\n");
}

#[test]
fn root_may_come_before_the_command() {
    let (dir, _root) = fixture();
    let out = run_in(dir.path(), &["--root", ".", "digests"], false);
    assert_eq!(out.code, 0);
    assert!(out.stdout.starts_with("guide_sha256: "));
}

#[test]
fn verify_reads_the_real_clock() {
    // The fixture's review date has passed, so an unpinned verify fails on it.
    let (dir, _root) = fixture();
    run(dir.path(), &["build"]);
    let out = run(dir.path(), &["verify"]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("is not after the verification date"));
}

#[test]
fn printed_lines_use_the_platform_line_ending() {
    // Python's text-mode print wrote the platform line ending; so does the binary.
    let (dir, _root) = fixture();
    let raw = std::process::Command::new(env!("CARGO_BIN_EXE_drdebits_build"))
        .args(["digests", "--root", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let expected: &[u8] = if cfg!(windows) { b"\r\n" } else { b"\n" };
    assert!(raw.stdout.ends_with(expected));
    assert_eq!(
        raw.stdout.windows(2).filter(|w| w == b"\r\n").count(),
        if cfg!(windows) { 2 } else { 0 }
    );
}

#[test]
fn an_error_python_did_not_catch_exits_1_from_every_command() {
    // PyYAML's KeyError for an unknown boolean escaped each command's handler,
    // so Python stopped with a traceback and exit 1, even under compare --check.
    let (dir, _root) = fixture();
    let metadata = dir.path().join("src/data/metadata.yaml");
    let mut text = std::fs::read_to_string(&metadata).unwrap();
    text.push_str("unexpected: !!bool maybe\n");
    std::fs::write(&metadata, text).unwrap();
    for args in [
        &["build"][..],
        &["digests"],
        &["compare", "a.json", "b.json", "--check"],
    ] {
        let out = run(dir.path(), args);
        assert_eq!(
            (out.code, out.stderr),
            (
                1,
                format!("{}: unexpected error: KeyError: 'maybe'\n", args[0])
            ),
            "{args:?}"
        );
    }
}

#[test]
fn linkcheck_reports_a_missing_root_without_requesting_anything() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_in(dir.path(), &["linkcheck"], false);
    assert_eq!(out.code, 1);
    assert!(
        out.stderr
            .starts_with("linkcheck: no DrDebits root found above "),
        "{}",
        out.stderr
    );
    assert_eq!(out.stdout, "");
}
