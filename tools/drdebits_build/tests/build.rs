//! Output assembly against a synthetic source tree (ported from test_build.py).

mod common;

use common::{fixture, py};
use drdebits::builder::{self, GENERATED};
use drdebits::error::Fail;

#[test]
fn find_root_walks_up_and_refuses_outside_a_tree() {
    let (dir, root) = fixture();
    let nested = dir.path().join("src").join("guide");
    let found = builder::find_root(&nested).unwrap();
    assert_eq!(
        drdebits::py::resolve(&found.fs()),
        drdebits::py::resolve(&root.fs()),
        "find_root returns the resolved tree root"
    );
    let outside = tempfile::tempdir().unwrap();
    assert!(matches!(
        builder::find_root(outside.path()),
        Err(Fail::Build(_))
    ));
}

#[test]
fn guide_shape_and_determinism() {
    let (_dir, root) = fixture();
    let s = builder::load_sources(&root).unwrap();
    let (g1, g2) = (builder::build_guide(&s), builder::build_guide(&s));
    assert_eq!(g1, g2);
    assert!(g1.starts_with("---\nguide_version: 0.9.9-test\n"));
    assert!(
        g1.trim_end_matches('\n')
            .ends_with("DRDEBITS-END-v0.9.9-test")
    );
    assert!(g1.contains("## Rules") && g1.contains("| 0.9.9-test | 2026-01-01 | Draft | init |"));
}

#[test]
fn catalogue_md_header_version_and_table() {
    let (_dir, root) = fixture();
    let out = builder::build_catalogue_md(&builder::load_sources(&root).unwrap());
    assert_eq!(out.matches("0.9.9-test").count(), 1);
    assert!(out.starts_with("# Complete TPB Guidance Statement catalogue\n"));
    assert!(out.contains("| Statement (concise title and official link) | LLM trigger |"));
    assert!(out.contains("| [T](https://x.invalid/a) | g |"));
}

#[test]
fn catalogue_md_header_derived_from_metadata_and_rows() {
    // Count, GS range and check date come from the sources, not template literals.
    let (_dir, root) = fixture();
    let out = builder::build_catalogue_md(&builder::load_sources(&root).unwrap());
    assert!(out.contains("discoverable on 1 January 2026: 1 statements, GS01 to GS01,"));
}

#[test]
fn behaviour_md_header_version_and_table() {
    let (_dir, root) = fixture();
    let out = builder::build_behaviour_md(&builder::load_sources(&root).unwrap());
    assert_eq!(out.matches("0.9.9-test").count(), 1);
    assert!(out.starts_with("# DrDebits behaviour tests\n"));
    assert!(out.contains("| ID | Scenario | Expected status | Required behaviour and human step | Side-effect check |"));
    assert!(out.contains("| A-001 | s | HARD_STOP | r | c |"));
}

#[test]
fn apes_md_header_version_and_both_tables() {
    let (_dir, root) = fixture();
    let out = builder::build_apes_md(&builder::load_sources(&root).unwrap());
    assert_eq!(out.matches("0.9.9-test").count(), 1);
    assert!(out.starts_with("# Primary APES 110 reference map\n"));
    for needle in [
        "| Context | APES 110 starting points |",
        "| All | Part 1 |",
        "Key paragraph-level retrieval points are:",
        "| Control | APES 110 retrieval points |",
        "| Scope | R1.2 |",
    ] {
        assert!(out.contains(needle), "{needle}");
    }
}

#[test]
fn vendor_assurance_md_renders_one_table_per_declared_section() {
    let (_dir, root) = fixture();
    let out = builder::build_vendor_assurance_md(&builder::load_sources(&root).unwrap());
    assert_eq!(out.matches("0.9.9-test").count(), 1);
    assert!(out.starts_with("# AI tool and vendor assurance checklist\n"));
    assert_eq!(
        out.matches("| ID | Question | Evidence that answers it | Where the obligation sits | If nobody answers |").count(),
        2
    );
    assert!(out.contains("## First") && out.contains("## Second"));
    assert!(
        out.contains("| VA-01 | q1 | e1 | o1 | a1 |")
            && out.contains("| VA-02 | q2 | e2 | o2 | a2 |")
    );
    // Sections render in the declared order, and the file ends with one newline.
    assert!(out.find("## First") < out.find("## Second"));
    assert!(out.ends_with("| VA-02 | q2 | e2 | o2 | a2 |\n"));
}

#[test]
fn concordance_md_renders_dates_and_one_table_per_section() {
    let (_dir, root) = fixture();
    let out = builder::build_concordance_md(&builder::load_sources(&root).unwrap());
    assert_eq!(out.matches("0.9.9-test").count(), 1);
    assert!(out.starts_with("# TPB(GS) 55/2026 concordance\n"));
    assert!(out.contains("issued on 22 July 2026 and last read for this file on 1 January 2026"));
    assert_eq!(
        out.matches("| ID | Source | Consideration | DrDebits control | Note |")
            .count(),
        2
    );
    assert!(out.find("## Alpha") < out.find("## Beta"));
    assert!(out.contains("| CC-01 | Paragraph 1 | c1 | k1 | n1 |"));
    assert!(out.ends_with("| CC-02 | Practice 1 | c2 | k2 | n2 |\n"));
    assert!(
        GENERATED
            .iter()
            .any(|(rel, _)| *rel == "reference/tpb-gs55-concordance.md")
    );
}

#[test]
fn vendor_assurance_md_puts_each_row_under_its_own_section() {
    // Every declared section must still carry rows, so the load fails rather
    // than rendering "Second" as a bare heading with no table.
    let (dir, root) = fixture();
    let source = dir.path().join("src/data/ai-vendor-assurance.yaml");
    let text = std::fs::read_to_string(&source).unwrap().replacen(
        "section: \"Second\"",
        "section: \"First\"",
        1,
    );
    std::fs::write(&source, text).unwrap();
    assert!(matches!(builder::load_sources(&root), Err(Fail::Model(_))));
}

#[test]
fn sha256sums_covers_generated_and_static() {
    let (_dir, root) = fixture();
    let s = builder::load_sources(&root).unwrap();
    let out = builder::build_sha256sums(&root, &s, &builder::render_generated(&s)).unwrap();
    let lines: Vec<&str> = out.trim_end_matches('\n').split('\n').collect();
    assert!(lines[0].ends_with(" *LICENSE") && lines[2].ends_with(" *drdebits.md"));
    assert!(
        lines
            .iter()
            .all(|l| l.split(" *").next().unwrap().len() == 64)
    );
}

#[test]
fn write_outputs_lf_only() {
    let (dir, root) = fixture();
    let outdir = py(&dir.path().join("out"));
    let s = builder::load_sources(&root).unwrap();
    let written = builder::write_outputs(&root, &outdir, &s).unwrap();
    assert!(
        written.contains(&"drdebits.md".to_string()) && written.contains(&"SHA256SUMS".to_string())
    );
    let raw = std::fs::read(outdir.fs().join("drdebits.md")).unwrap();
    assert!(!raw.contains(&b'\r'));
}

#[test]
fn fragment_order_is_case_sensitive_string_sort() {
    // Fragments sort by name string on every platform.
    let (dir, root) = fixture();
    std::fs::write(dir.path().join("src/guide/500-B.md"), "## B section\n").unwrap();
    std::fs::write(dir.path().join("src/guide/500-a.md"), "## a section\n").unwrap();
    let guide = builder::build_guide(&builder::load_sources(&root).unwrap());
    assert!(guide.find("## B section") < guide.find("## a section"));
}

#[test]
fn stamp_version_touches_only_the_two_stamp_contexts() {
    let stamp = |text: &str, v: &str| builder::stamp_version(text, v).unwrap();
    assert_eq!(
        stamp("> Version: `0.2.0-draft`", "0.3.0-draft"),
        "> Version: `0.3.0-draft`"
    );
    assert_eq!(
        stamp(
            "Part of [DrDebits](./drdebits.md) `0.2.0-draft`.",
            "0.3.0-draft"
        ),
        "Part of [DrDebits](./drdebits.md) `0.3.0-draft`."
    );
    // Backticked version-like tokens outside the two contexts are prose.
    assert_eq!(
        stamp("Install uv `0.12.0` before building.", "2.0.0"),
        "Install uv `0.12.0` before building."
    );
    // Bare version-like tokens are prose, not a stamp.
    assert_eq!(
        stamp("v1.2.3 and 9.9.9-x.1", "2.0.0"),
        "v1.2.3 and 9.9.9-x.1"
    );
}

#[test]
fn stamp_version_applies_re_sub_template_escapes() {
    // The Python builder passed the version to re.sub as a template.
    assert_eq!(
        builder::stamp_version("Version: `1.0.0`", r"2.0\g<0>").unwrap(),
        "Version: `2.01.0.0`"
    );
    assert_eq!(
        builder::stamp_version("Version: `1.0.0`", r"a\\b").unwrap(),
        r"Version: `a\b`"
    );
    assert!(matches!(
        builder::stamp_version("x", r"\q"),
        Err(Fail::Internal(_))
    ));
    assert!(matches!(
        builder::stamp_version("x", r"\1"),
        Err(Fail::Internal(_))
    ));
}

#[test]
fn frontmatter_verbatim_values_in_order() {
    let meta = vec![
        ("title".to_string(), "DrDebits".to_string()),
        ("apes_110_pdf_sha256".to_string(), "B6937B93".to_string()),
    ];
    assert_eq!(
        builder::render_frontmatter(&meta),
        "---\ntitle: DrDebits\napes_110_pdf_sha256: B6937B93\n---\n"
    );
}

#[test]
fn table_single_space_padding_and_alignment_tokens() {
    let rows = vec![
        vec!["AUTH-001".to_string(), "says x".to_string()],
        vec!["INJ-001".to_string(), "says y".to_string()],
    ];
    assert_eq!(
        builder::render_table(&["ID", "Scenario"], &["---", "---"], &rows),
        "| ID | Scenario |\n|---|---|\n| AUTH-001 | says x |\n| INJ-001 | says y |\n"
    );
}
