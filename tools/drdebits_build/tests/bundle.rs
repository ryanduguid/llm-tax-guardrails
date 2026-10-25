//! Installed bundles keep their approved bytes and never overwrite a copy
//! (ported from test_bundle.py). Redirect and escape checks use real links
//! where the platform lets a test create them.

mod common;

use std::path::Path;

use common::{py, repo_root, run_in};
use drdebits::builder::sha256_hex;
use drdebits::bundle::{self, OPTIONAL, REQUIRED};
use drdebits::error::Fail;

fn make_bundle(dir: &Path, optional: bool) -> (std::path::PathBuf, String) {
    let source = dir.join("source");
    std::fs::create_dir_all(&source).unwrap();
    let mut names: Vec<&str> = REQUIRED.to_vec();
    if optional {
        names.extend(OPTIONAL);
    }
    names.sort();
    let mut manifest = String::new();
    for name in names {
        let path = source.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let raw = format!("Synthetic bundle: {name}\n");
        std::fs::write(&path, &raw).unwrap();
        manifest.push_str(&format!("{} *{name}\n", sha256_hex(raw.as_bytes())));
    }
    std::fs::write(source.join("SHA256SUMS"), &manifest).unwrap();
    (source, sha256_hex(manifest.as_bytes()))
}

fn bundle_error<T: std::fmt::Debug>(result: Result<T, Fail>) -> String {
    match result {
        Err(Fail::Bundle(m)) => m,
        other => panic!("expected a BundleError, got {other:?}"),
    }
}

#[test]
fn complete_install_preserves_historical_and_current_bundles() {
    for optional in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let (source, digest) = make_bundle(dir.path(), optional);
        let target = dir.path().join("installed");
        bundle::install(&py(&source), &py(&target), &digest).unwrap();
        assert_eq!(
            bundle::verified_files(&py(&target), &digest).unwrap(),
            bundle::verified_files(&py(&source), &digest).unwrap()
        );
        assert!(matches!(
            bundle::install(&py(&source), &py(&target), &digest),
            Err(Fail::Os(_))
        ));
        std::fs::write(target.join("drdebits.md"), "tampered").unwrap();
        let out = run_in(
            dir.path(),
            &[
                "bundle",
                "verify",
                "--source",
                &target.to_string_lossy(),
                "--manifest-sha256",
                &digest,
            ],
            false,
        );
        assert_eq!(out.code, 1);
    }
}

#[test]
fn untrusted_or_incomplete_manifest_never_creates_destination() {
    for damage in ["digest", "missing", "duplicate", "escape", "unknown"] {
        let dir = tempfile::tempdir().unwrap();
        let (source, _) = make_bundle(dir.path(), true);
        let manifest = source.join("SHA256SUMS");
        let mut lines: Vec<String> = std::fs::read_to_string(&manifest)
            .unwrap()
            .lines()
            .map(|l| format!("{l}\n"))
            .collect();
        let digest = if damage == "digest" {
            "0".repeat(64)
        } else {
            match damage {
                "missing" => lines.retain(|l| !l.ends_with(" *drdebits.md\n")),
                "duplicate" => lines.push(lines[0].clone()),
                _ => lines.push(format!(
                    "{} *{}\n",
                    "0".repeat(64),
                    if damage == "escape" {
                        "../outside"
                    } else {
                        ".env"
                    }
                )),
            }
            std::fs::write(&manifest, lines.concat()).unwrap();
            sha256_hex(&std::fs::read(&manifest).unwrap())
        };
        let target = dir.path().join("installed");
        bundle_error(bundle::install(&py(&source), &py(&target), &digest));
        assert!(!target.exists(), "{damage}");
    }
}

#[test]
fn changed_or_missing_file_never_creates_destination() {
    let dir = tempfile::tempdir().unwrap();
    let (source, digest) = make_bundle(dir.path(), true);
    std::fs::write(source.join("drdebits.md"), b"changed").unwrap();
    let target = dir.path().join("installed");
    assert!(
        bundle_error(bundle::install(&py(&source), &py(&target), &digest))
            .contains("checksum mismatch")
    );
    assert!(!target.exists());
    std::fs::remove_file(source.join("drdebits.md")).unwrap();
    assert!(matches!(
        bundle::install(&py(&source), &py(&target), &digest),
        Err(Fail::Os(_))
    ));
    assert!(!target.exists());
}

#[cfg(unix)]
fn link_dir(original: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(original, link)
}

#[cfg(windows)]
fn link_dir(original: &Path, link: &Path) -> std::io::Result<()> {
    // A junction needs no privilege, unlike a Windows symbolic link.
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(original)
        .output()?;
    if status.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other("mklink /J failed"))
    }
}

#[test]
fn missing_member_under_a_linked_parent_is_an_os_error() {
    // A link above the source (or a Windows 8.3 short name in TEMP) must not
    // turn a missing member into an escape.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("real")).unwrap();
    let (real_source, digest) = make_bundle(&dir.path().join("real"), true);
    link_dir(&dir.path().join("real"), &dir.path().join("linked"))
        .expect("create a directory link");
    let source = dir
        .path()
        .join("linked")
        .join(real_source.file_name().unwrap());
    std::fs::remove_file(source.join("drdebits.md")).unwrap();
    let target = dir.path().join("installed");
    let result = bundle::install(&py(&source), &py(&target), &digest);
    assert!(matches!(result, Err(Fail::Os(_))), "{result:?}");
    assert!(!target.exists());
}

#[test]
fn redirected_bundle_member_is_rejected_before_read() {
    let dir = tempfile::tempdir().unwrap();
    let (source, digest) = make_bundle(dir.path(), true);
    // Move reference/ aside inside the source and redirect to it.
    std::fs::rename(source.join("reference"), source.join("real-reference")).unwrap();
    link_dir(&source.join("real-reference"), &source.join("reference"))
        .expect("create a directory link");
    assert!(bundle_error(bundle::verified_files(&py(&source), &digest)).contains("redirected"));
}

#[cfg(unix)]
#[test]
fn member_resolving_outside_the_source_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let (source, digest) = make_bundle(dir.path(), true);
    let outside = dir.path().join("outside.md");
    std::fs::rename(source.join("drdebits.md"), &outside).unwrap();
    std::os::unix::fs::symlink(&outside, source.join("drdebits.md")).unwrap();
    assert!(
        bundle_error(bundle::verified_files(&py(&source), &digest))
            .contains("escapes source: drdebits.md")
    );
}

#[cfg(windows)]
#[test]
fn member_resolving_outside_the_source_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let (source, digest) = make_bundle(dir.path(), true);
    let outside = dir.path().join("outside-reference");
    std::fs::rename(source.join("reference"), &outside).unwrap();
    link_dir(&outside, &source.join("reference")).expect("create a directory link");
    let message = bundle_error(bundle::verified_files(&py(&source), &digest));
    assert!(message.contains("escapes source: reference/"), "{message}");
}

#[test]
fn checked_in_bundle_installs_and_cli_verifies() {
    let source = repo_root();
    let digest = sha256_hex(&std::fs::read(source.join("SHA256SUMS")).unwrap());
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("current");
    let install = run_in(
        dir.path(),
        &[
            "bundle",
            "install",
            "--source",
            &source.to_string_lossy(),
            "--destination",
            &target.to_string_lossy(),
            "--manifest-sha256",
            &digest,
        ],
        false,
    );
    assert_eq!(install.code, 0, "{}", install.stderr);
    assert_eq!(
        install.stdout,
        "bundle: OK (approved manifest and file digests; host loading not checked)\n"
    );
    let verify = run_in(
        dir.path(),
        &[
            "bundle",
            "verify",
            "--source",
            &target.to_string_lossy(),
            "--manifest-sha256",
            &digest,
        ],
        false,
    );
    assert_eq!(verify.code, 0);
}

#[test]
fn approved_digest_must_be_lower_case_hex() {
    let dir = tempfile::tempdir().unwrap();
    let (source, digest) = make_bundle(dir.path(), true);
    assert!(
        bundle_error(bundle::verified_files(&py(&source), &digest.to_uppercase()))
            .contains("64 lower-case hex")
    );
}

#[test]
fn malformed_checksum_line_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let (source, _) = make_bundle(dir.path(), true);
    let manifest = source.join("SHA256SUMS");
    let mut bytes = std::fs::read(&manifest).unwrap();
    bytes.extend_from_slice(b"not a checksum line\n");
    std::fs::write(&manifest, &bytes).unwrap();
    assert!(
        bundle_error(bundle::verified_files(&py(&source), &sha256_hex(&bytes)))
            .contains("malformed checksum entry")
    );
}

#[test]
fn install_without_destination_is_a_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    let (source, digest) = make_bundle(dir.path(), true);
    let out = run_in(
        dir.path(),
        &[
            "bundle",
            "install",
            "--source",
            &source.to_string_lossy(),
            "--manifest-sha256",
            &digest,
        ],
        false,
    );
    assert_eq!(out.code, 2);
    assert!(out.stderr.contains("install requires --destination"));
}
