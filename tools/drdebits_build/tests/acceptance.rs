//! The real repository verifies clean, hand edits fail, and a version bump
//! propagates (ported from test_acceptance.py).
//!
//! Verify's date is pinned to the repository's own source-check date, which
//! check (g) requires to fall before review_due, so the pin can never bless a
//! release the gate rejects.

mod common;

use common::{Tree, run_in};
use drdebits::model::{load_metadata, meta_get};
use drdebits::py::Date;
use drdebits::verify::run_verify;

fn pinned_today(tree: &Tree) -> Date {
    let meta = load_metadata(&tree.root().join("src/data/metadata.yaml")).unwrap();
    Date::fromisoformat(&meta_get(&meta, "sources_checked_at").unwrap()[..10]).unwrap()
}

#[test]
fn verification_date_is_before_review_due() {
    let tree = Tree::new();
    let meta = load_metadata(&tree.root().join("src/data/metadata.yaml")).unwrap();
    assert!(
        pinned_today(&tree) < Date::fromisoformat(meta_get(&meta, "review_due").unwrap()).unwrap()
    );
}

#[test]
fn real_repo_verifies() {
    let tree = Tree::new();
    assert_eq!(
        run_verify(&tree.root(), pinned_today(&tree)).unwrap(),
        Vec::<String>::new()
    );
}

#[test]
fn hand_edit_fails() {
    let tree = Tree::new();
    tree.replace("reference/tpb-catalogue.md", "TPB", "TBP");
    assert!(
        !run_verify(&tree.root(), pinned_today(&tree))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn version_bump_propagates() {
    // The bump procedure (MAINTENANCE.md step 8): metadata, the guide's header
    // line, the newest changelog entry and CITATION.cff move together.
    let tree = Tree::new();
    let meta = load_metadata(&tree.root().join("src/data/metadata.yaml")).unwrap();
    let current = meta_get(&meta, "guide_version").unwrap().to_string();
    let parts: Vec<&str> = current.splitn(3, '.').collect();
    let patch_digits: String = parts[2].chars().take_while(char::is_ascii_digit).collect();
    let suffix = &parts[2][patch_digits.len()..];
    let bumped = format!(
        "{}.{}.{}{}",
        parts[0],
        parts[1],
        patch_digits.parse::<u32>().unwrap() + 1,
        suffix
    );
    for rel in [
        "src/data/metadata.yaml",
        "src/guide/000-header.md",
        "src/data/changelog.yaml",
        "CITATION.cff",
    ] {
        let text = tree.read(rel).replace(&current, &bumped);
        tree.write(rel, &text);
    }
    let build = run_in(tree.path(), &["build"], true);
    assert_eq!(build.code, 0, "{}", build.stderr);
    assert_eq!(
        run_verify(&tree.root(), pinned_today(&tree)).unwrap(),
        Vec::<String>::new()
    );
    assert!(tree.read("README.md").contains(&bumped));
    assert!(
        tree.read("drdebits.md")
            .trim_end_matches('\n')
            .ends_with(&bumped)
    );
}
