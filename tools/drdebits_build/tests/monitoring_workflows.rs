//! Issue migration and recovery, exercised through the workflows' own shell
//! steps (ported from test_monitoring_workflows.py). Only the GitHub
//! transport is stubbed; the real jq selection and shell control flow run,
//! and no GitHub write or network request can occur.

mod common;

use std::path::PathBuf;
use std::process::Command;

use common::repo_root;
use drdebits::yaml::{self, Value};

fn find_on_path(name: &str, dirs: Option<&[PathBuf]>) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let paths: Vec<PathBuf> = match dirs {
        Some(d) => d.to_vec(),
        None => std::env::split_paths(&std::env::var_os("PATH")?).collect(),
    };
    paths
        .into_iter()
        .map(|d| d.join(&exe))
        .find(|p| p.is_file())
}

/// Bash, avoiding the WindowsApps alias that can start WSL: Git's own bash on Windows.
fn bash() -> Option<PathBuf> {
    if cfg!(windows) {
        // git.exe may sit in Git's cmd, bin or ucrt64\bin; bash is in Git's bin.
        let git = find_on_path("git", None)?;
        let candidates: Vec<PathBuf> = git
            .ancestors()
            .skip(1)
            .take(3)
            .map(|dir| dir.join("bin"))
            .collect();
        find_on_path("bash", Some(&candidates))
    } else {
        find_on_path("bash", None)
    }
}

fn workflow(name: &str) -> Value {
    let text = std::fs::read_to_string(repo_root().join(".github/workflows").join(name)).unwrap();
    yaml::load(&text).unwrap()
}

fn list(value: &Value) -> Vec<Value> {
    match value {
        Value::List(items) => items.borrow().clone(),
        Value::Dict(pairs) => pairs.borrow().iter().map(|(_, v)| v.clone()).collect(),
        _ => panic!("a collection"),
    }
}

/// The steps of the job that holds a step named `step`.
fn job_steps_with(document: &Value, step: &str) -> Vec<Value> {
    for job in list(&document.get("jobs").unwrap()) {
        let steps = list(&job.get("steps").unwrap());
        if steps.iter().any(|s| {
            s.get("name")
                .and_then(|n| n.as_str().map(str::to_string))
                .as_deref()
                == Some(step)
        }) {
            return steps;
        }
    }
    panic!("no job has a step named {step}");
}

fn step_run(document: &Value, name: &str) -> String {
    let steps = job_steps_with(document, name);
    let step = steps
        .iter()
        .find(|s| {
            s.get("name")
                .and_then(|n| n.as_str().map(str::to_string))
                .as_deref()
                == Some(name)
        })
        .unwrap();
    step.get("run").unwrap().as_str().unwrap().to_string()
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\"'\"'"))
}

const STUB: &str = r#"
jq() { "$JQ" "$@"; }
gh() {
  case "$1 $2" in
    "api --paginate")
      [[ " $* " == *" --slurp "* ]] || return 91
      [[ " $* " != *" --jq "* ]] || return 92
      printf '%s\n' "$FIXTURE"
      ;;
    "issue view") printf '%s\n' 'previous monitoring report' ;;
    *) printf '%s\n' "$*" >> operations.txt ;;
  esac
}
GITHUB_REPOSITORY=example/repository
CHECK_EXIT_CODE=2
"#;

fn issues_json(issues: &[(u32, String, bool)]) -> String {
    let pages: Vec<String> = issues
        .chunks(100)
        .map(|page| {
            let items: Vec<String> = page
                .iter()
                .map(|(n, title, pr)| {
                    let pr = if *pr { ", \"pull_request\": {}" } else { "" };
                    format!(
                        "{{\"number\": {n}, \"title\": {}{pr}}}",
                        drdebits::pyjson::dumps(&drdebits::pyjson::Json::Str(title.clone()), false)
                    )
                })
                .collect();
            format!("[{}]", items.join(", "))
        })
        .collect();
    format!("[{}]", pages.join(", "))
}

#[test]
fn monitoring_reuses_legacy_alerts_and_recovers_all_reports() {
    let (Some(bash), Some(jq)) = (bash(), find_on_path("jq", None)) else {
        eprintln!("skipped: workflow integration needs Bash and jq");
        return;
    };
    for (name, prefix, legacy) in [
        (
            "link-check",
            "Link check",
            "SOURCE CURRENCY NOT CONFIRMED for listed links",
        ),
        (
            "source-currency",
            "Source currency",
            "a pinned compilation has been superseded",
        ),
    ] {
        let document = workflow(&format!("{name}.yml"));
        for existing in ["legacy", "current", "both", "none"] {
            for recovery in [false, true] {
                for unrelated in [1u32, 101] {
                    let mut issues: Vec<(u32, String, bool)> = (0..unrelated)
                        .map(|n| (1000 + n, format!("{prefix}: unrelated issue"), false))
                        .collect();
                    issues.push((98, format!("{prefix}: review required"), true));
                    if existing == "legacy" || existing == "both" {
                        issues.push((11, format!("{prefix}: {legacy}"), false));
                    }
                    if existing == "current" || existing == "both" {
                        issues.push((12, format!("{prefix}: review required"), false));
                    }
                    let mut expected: Vec<u32> = issues
                        .iter()
                        .map(|i| i.0)
                        .filter(|n| *n == 11 || *n == 12)
                        .collect();
                    expected.sort();
                    let step = if recovery {
                        "Record recovery"
                    } else {
                        "Open or update the report issue"
                    };
                    let script = step_run(&document, step);
                    let dir = tempfile::tempdir().unwrap();
                    let jq_path = jq.to_string_lossy().replace('\\', "/");
                    let setup = format!(
                        "FIXTURE={}\nJQ={}\n",
                        shell_quote(&issues_json(&issues)),
                        shell_quote(&jq_path)
                    );
                    std::fs::write(
                        dir.path().join(format!("{name}.txt")),
                        "synthetic finding\n",
                    )
                    .unwrap();
                    std::fs::write(dir.path().join("operations.txt"), "").unwrap();
                    std::fs::write(dir.path().join("step.sh"), format!("{setup}{STUB}{script}"))
                        .unwrap();
                    let out = Command::new(&bash)
                        .arg("step.sh")
                        .current_dir(dir.path())
                        .output()
                        .unwrap();
                    assert!(
                        out.status.success(),
                        "{name} {existing} {recovery} {unrelated}: {}",
                        String::from_utf8_lossy(&out.stderr)
                    );
                    let text = std::fs::read_to_string(dir.path().join("operations.txt")).unwrap();
                    let operations: Vec<&str> = text.lines().collect();
                    let case = format!(
                        "{name} {existing} recovery={recovery} unrelated={unrelated}: {operations:?}"
                    );
                    if recovery {
                        assert_eq!(operations.len(), expected.len(), "{case}");
                        for (i, number) in expected.iter().enumerate() {
                            assert!(
                                operations[i].contains(&format!("issue close {number} ")),
                                "{case}"
                            );
                        }
                    } else if !expected.is_empty() {
                        assert!(
                            operations[0].contains(&format!("issue edit {} ", expected[0])),
                            "{case}"
                        );
                        assert!(
                            operations[0].contains(&format!("--title {prefix}: review required")),
                            "{case}"
                        );
                        assert_eq!(operations.len(), expected.len(), "{case}");
                        for (i, number) in expected[1..].iter().enumerate() {
                            assert!(
                                operations[i + 1].contains(&format!("issue close {number} ")),
                                "{case}"
                            );
                        }
                    } else {
                        assert_eq!(operations.len(), 1, "{case}");
                        assert!(operations[0].starts_with("issue create "), "{case}");
                    }
                    assert!(
                        operations.iter().all(|o| !o.contains("issue close 98 ")),
                        "{case}"
                    );
                }
            }
        }
    }
}

#[test]
fn link_check_fails_unless_every_reachable_link_was_verified() {
    let Some(bash) = bash() else {
        eprintln!("skipped: workflow integration needs Bash");
        return;
    };
    let document = workflow("link-check.yml");
    let steps = job_steps_with(&document, "Check and retain the report");
    let last = steps.last().unwrap();
    assert_eq!(last.get("if").unwrap().as_str(), Some("always()"));
    let script = format!("{}\n", last.get("run").unwrap().as_str().unwrap());
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("gate.sh"), script).unwrap();
    for (code, passes) in [
        ("0", true),
        ("3", true),
        ("1", false),
        ("2", false),
        ("", false),
    ] {
        let out = Command::new(&bash)
            .arg(dir.path().join("gate.sh"))
            .env_clear()
            .env("CHECK_EXIT_CODE", code)
            .output()
            .unwrap();
        assert_eq!(out.status.success(), passes, "exit code {code:?}");
    }
}

#[test]
fn the_report_jobs_run_a_checker_they_did_not_build() {
    // A job that can write issues must not run dependency build scripts.
    for name in ["link-check.yml", "source-currency.yml"] {
        let document = workflow(name);
        for job in list(&document.get("jobs").unwrap()) {
            let can_write = job
                .get("permissions")
                .and_then(|p| p.get("issues"))
                .and_then(|v| v.as_str().map(str::to_string))
                .as_deref()
                == Some("write");
            let runs: Vec<String> = list(&job.get("steps").unwrap())
                .iter()
                .filter_map(|s| s.get("run").and_then(|r| r.as_str().map(str::to_string)))
                .collect();
            let builds = runs
                .iter()
                .any(|r| r.contains("cargo ") || r.contains("rustup "));
            assert!(
                !(can_write && builds),
                "{name}: a job with issues: write builds code"
            );
        }
        let top = document.get("permissions").unwrap();
        assert!(
            top.get("issues").is_none(),
            "{name}: workflow-level permissions grant issues"
        );
    }
}
