//! The supplier information for a firm's AI register names the current
//! release (ported from test_ai_register_entry.py).

mod common;

use common::repo_root;
use drdebits::model::{load_metadata, meta_get};
use drdebits::py::PyPath;

#[test]
fn register_entry_names_the_release_tag_in_metadata() {
    let root = repo_root();
    let meta = load_metadata(&PyPath::new(
        &root.join("src/data/metadata.yaml").to_string_lossy(),
    ))
    .unwrap();
    let tag = meta_get(&meta, "release_tag").unwrap();
    let entry = std::fs::read_to_string(root.join("docs/ai-register-entry.md")).unwrap();
    assert!(entry.contains(&format!(
        "| Name and version | DrDebits (`llm-tax-guardrails`) release {tag}, "
    )));
    // The limits and updates rows name the release too; a bump must revisit them.
    let pattern =
        regex::Regex::new(r"\bv\d+\.\d+\.\d+(?:-[0-9A-Za-z]+(?:\.[0-9A-Za-z]+)*)?").unwrap();
    let mut found: Vec<&str> = pattern.find_iter(&entry).map(|m| m.as_str()).collect();
    found.sort();
    found.dedup();
    assert_eq!(found, [tag]);
}

#[test]
fn readme_lists_the_register_entry() {
    assert!(
        std::fs::read_to_string(repo_root().join("README.md"))
            .unwrap()
            .contains("(./docs/ai-register-entry.md)")
    );
}
