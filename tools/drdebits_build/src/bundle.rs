//! Install an approved manifest's complete bundle without overwriting a copy.
//!
//! The approved manifest digest comes from the deployer's trusted release
//! record. Matching bytes proves integrity, not release approval or host
//! enforcement.

use std::io::Write as _;
use std::sync::LazyLock;

use regex::Regex;

use crate::builder::sha256_hex;
use crate::error::Fail;
use crate::py::{self, OsStyle, PyPath};

pub const REQUIRED: [&str; 8] = [
    "LICENSE",
    "README.md",
    "CITATION.cff",
    "drdebits.md",
    "MAINTENANCE.md",
    "reference/tpb-catalogue.md",
    "reference/apes-110-map.md",
    "tests/behaviour-tests.md",
];
// NOTICE joined the bundle with v0.4.0, so earlier manifests omit it.
pub const OPTIONAL: [&str; 4] = [
    "reference/ai-vendor-assurance.md",
    "reference/tpb-gs55-concordance.md",
    "DISCLAIMER.md",
    "NOTICE",
];

static DIGEST_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[0-9a-f]{64}\z").expect("pattern"));
static ENTRY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([0-9a-f]{64}) [ *](.+)\z").expect("pattern"));

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Action {
    Install,
    Verify,
}

#[derive(clap::Args)]
pub struct Args {
    action: Action,
    /// The bundle to read.
    #[arg(long)]
    source: String,
    /// The approved SHA256SUMS digest from the trusted release record.
    #[arg(long)]
    manifest_sha256: String,
    /// For install: an unused directory in an existing parent.
    #[arg(long)]
    destination: Option<String>,
}

fn bundle(message: String) -> Fail {
    Fail::Bundle(message)
}

fn is_redirect(path: &std::path::Path) -> bool {
    // A Windows junction reads as a symbolic link here too.
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

fn read(root: &PyPath, relative: &str) -> Result<Vec<u8>, Fail> {
    let path = root.join(relative);
    let resolved = py::resolve(&path.fs());
    let resolved_root = py::resolve(&root.fs());
    if !resolved.starts_with(&resolved_root) {
        return Err(bundle(format!("bundle path escapes source: {relative}")));
    }
    // Reject redirected files and directories even when they point inside root.
    let mut part = path.clone();
    while part != *root {
        if is_redirect(&part.fs()) {
            return Err(bundle(format!("redirected bundle path: {relative}")));
        }
        let parent = part.parent();
        if parent == part {
            break;
        }
        part = parent;
    }
    std::fs::read(path.fs())
        .map_err(|e| Fail::Os(py::oserror_str(&e, &path.display(), OsStyle::Errno)))
}

/// Validate membership and digests before returning bytes to install.
pub fn verified_files(
    source: &PyPath,
    manifest_sha256: &str,
) -> Result<Vec<(String, Vec<u8>)>, Fail> {
    if !DIGEST_RE.is_match(manifest_sha256) {
        return Err(bundle(
            "approved manifest digest must be 64 lower-case hex characters".into(),
        ));
    }
    let manifest = read(source, "SHA256SUMS")?;
    if sha256_hex(&manifest) != manifest_sha256 {
        return Err(bundle(
            "SHA256SUMS does not match the approved manifest digest".into(),
        ));
    }
    let text = py::decode_utf8(&manifest).map_err(|e| Fail::Decode(e.to_string()))?;
    let mut entries: Vec<(String, String)> = Vec::new();
    for line in py::splitlines(&text) {
        let Some(caps) = ENTRY_RE.captures(line) else {
            return Err(bundle("malformed checksum entry".into()));
        };
        let (digest, relative) = (caps[1].to_string(), caps[2].to_string());
        let known = REQUIRED.contains(&relative.as_str()) || OPTIONAL.contains(&relative.as_str());
        if !known || entries.iter().any(|(r, _)| *r == relative) {
            return Err(bundle(format!(
                "unexpected or duplicate bundle member: {relative}"
            )));
        }
        entries.push((relative, digest));
    }
    if !REQUIRED.iter().all(|r| entries.iter().any(|(e, _)| e == r)) {
        return Err(bundle("manifest is missing required bundle members".into()));
    }
    let mut files = vec![("SHA256SUMS".to_string(), manifest)];
    for (relative, digest) in entries {
        let raw = read(source, &relative)?;
        if sha256_hex(&raw) != digest {
            return Err(bundle(format!("checksum mismatch: {relative}")));
        }
        files.push((relative, raw));
    }
    Ok(files)
}

pub fn install(source: &PyPath, destination: &PyPath, manifest_sha256: &str) -> Result<(), Fail> {
    let files = verified_files(source, manifest_sha256)?;
    // The deployer owns the parent directory. Never reuse an existing target,
    // including a broken symlink. A failed copy stays unapproved for inspection.
    std::fs::create_dir(destination.fs())
        .map_err(|e| Fail::Os(py::oserror_str(&e, &destination.display(), OsStyle::Native)))?;
    for (relative, raw) in &files {
        let target = destination.join(relative);
        let parent = target.parent();
        std::fs::create_dir_all(parent.fs())
            .map_err(|e| Fail::Os(py::oserror_str(&e, &parent.display(), OsStyle::Native)))?;
        let mut stream = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target.fs())
            .map_err(|e| Fail::Os(py::oserror_str(&e, &target.display(), OsStyle::Errno)))?;
        stream
            .write_all(raw)
            .map_err(|e| Fail::Os(py::oserror_str(&e, &target.display(), OsStyle::Errno)))?;
    }
    verified_files(destination, manifest_sha256)?;
    Ok(())
}

pub fn main(args: Args) -> u8 {
    let source = PyPath::new(&args.source);
    let result = match args.action {
        Action::Install => {
            let Some(destination) = &args.destination else {
                errln!(
                    "error: install requires --destination (an unused directory in an existing parent)"
                );
                return 2;
            };
            install(&source, &PyPath::new(destination), &args.manifest_sha256)
        }
        Action::Verify => verified_files(&source, &args.manifest_sha256).map(|_| ()),
    };
    match result {
        Ok(()) => {
            outln!("bundle: OK (approved manifest and file digests; host loading not checked)");
            0
        }
        // Python caught OSError and ValueError here; anything else escaped
        // as a traceback, which also exited 1.
        Err(e) => {
            errln!("bundle: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_redirect_walk_stops_at_the_top_when_root_is_not_an_ancestor() {
        // Members are relative names, so the walk normally ends at the root.
        // Named absolutely under a root spelt another way, it still ends.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("member"), b"bytes").unwrap();
        let root = PyPath::new(&dir.path().join("sub").join("..").to_string_lossy());
        let member = dir.path().join("member").to_string_lossy().into_owned();
        assert_eq!(read(&root, &member).unwrap(), b"bytes");
    }
}
