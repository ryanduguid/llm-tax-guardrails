//! Assemble the runtime files from src/. Deterministic, LF, UTF-8.

use std::path::Path;
use std::sync::LazyLock;

use sha2::{Digest, Sha256};

use crate::error::Fail;
use crate::evals;
use crate::model::{self, ApesMap, Behaviour, Meta, Row, Sectioned, WholeResponse, meta_get};
use crate::py::{self, Date, OsStyle, PyPath};
use crate::templates::*;

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// `template.format(**fields)` for the fixed templates: one pass, so a value
/// that itself contains braces is never substituted again.
pub fn format_named(template: &str, fields: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let close = rest[open..].find('}').expect("a closed placeholder") + open;
        let name = &rest[open + 1..close];
        let value = fields
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| *v)
            .expect("a known placeholder");
        out.push_str(value);
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

// Markdown rendering primitives. Explicit line construction only.

pub fn render_frontmatter(meta: &[(String, String)]) -> String {
    let mut lines = vec!["---".to_string()];
    lines.extend(meta.iter().map(|(k, v)| format!("{k}: {v}")));
    lines.push("---".into());
    lines.join("\n") + "\n"
}

pub fn render_table<S: AsRef<str>>(headers: &[S], aligns: &[&str], rows: &[Vec<String>]) -> String {
    let headers: Vec<&str> = headers.iter().map(AsRef::as_ref).collect();
    let mut out = vec![
        format!("| {} |", headers.join(" | ")),
        format!("|{}|", aligns.join("|")),
    ];
    out.extend(rows.iter().map(|r| format!("| {} |", r.join(" | "))));
    out.join("\n") + "\n"
}

// Matches only the guide-version token in its two sanctioned stamp contexts:
// the "Version: `X`" header line and the "Part of [DrDebits](./drdebits.md)
// `X`" satellite reference. The context and backticks are zero-width
// lookaround, so a replacement touches only the inner token, and every other
// version-like token (bare prose such as "uv 0.12.0", a documented tool pin
// such as `0.12.0`) is left alone.
const VERSION_TOKEN: &str = r"[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.]+)?";
pub static STAMP_RE: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
    fancy_regex::Regex::new(&format!(
        r"(?<=Version: `){VERSION_TOKEN}(?=`)|(?<=\[DrDebits\]\(\./drdebits\.md\) `){VERSION_TOKEN}(?=`)"
    ))
    .expect("a valid pattern")
});

/// The tokens `STAMP_RE` finds in `text`, with their byte offsets.
pub fn stamp_matches(text: &str) -> Vec<(usize, usize)> {
    STAMP_RE
        .find_iter(text)
        .map(|m| m.expect("a bounded match"))
        .map(|m| (m.start(), m.end()))
        .collect()
}

/// Hand-written files the build stamps the guide version into, and that
/// verify then holds to metadata. llms.txt is the index a retrieval agent
/// reads instead of README.md, so it carries the same version and date.
pub const STAMPED_FILES: [&str; 3] = ["README.md", "MAINTENANCE.md", "llms.txt"];

/// Of those, the files that also carry a hand-written copy of the source-check date.
pub const SOURCE_CHECK_DATE_FILES: [&str; 2] = ["README.md", "llms.txt"];

const CHANGELOG_HEADERS: [&str; 4] = ["Version", "Date", "Status", "Change"];

/// Find the DrDebits root at or above `start`, as an absolute resolved path.
pub fn find_root(start: &Path) -> Result<PyPath, Fail> {
    let resolved = py::resolve(start);
    for candidate in resolved.ancestors() {
        if candidate.join("drdebits.md").is_file() && candidate.join("src").is_dir() {
            return Ok(PyPath::new(&candidate.to_string_lossy()));
        }
    }
    Err(Fail::Build(format!(
        "no DrDebits root found above {}",
        start.display()
    )))
}

/// Everything the builders read from src/.
#[derive(Clone, Debug)]
pub struct Sources {
    pub meta: Meta,
    pub fragments: Vec<(String, String)>,
    pub catalogue: Vec<Row>,
    pub behaviour: Vec<Behaviour>,
    pub whole_response: WholeResponse,
    pub apes: ApesMap,
    pub vendor: Sectioned,
    pub concordance: Sectioned,
    pub changelog: Vec<Row>,
}

impl Sources {
    pub fn meta(&self, key: &str) -> &str {
        meta_get(&self.meta, key).expect("required metadata")
    }
}

/// `Path.glob("*.md")` names, which match case-insensitively on Windows.
fn markdown_names(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| {
            let tail = name.len().checked_sub(3).and_then(|i| name.get(i..));
            tail.is_some_and(|t| {
                if cfg!(windows) {
                    t.eq_ignore_ascii_case(".md")
                } else {
                    t == ".md"
                }
            })
        })
        .collect();
    // Sort by name, not path: path ordering casefolds on Windows.
    names.sort();
    names
}

/// A text read whose failure is an `OSError` or a `UnicodeDecodeError`.
pub fn read_text(path: &PyPath) -> Result<String, Fail> {
    py::read_text(&path.fs()).map_err(|e| match e {
        py::ReadError::Os(e) => Fail::Os(py::oserror_str(&e, &path.display(), OsStyle::Errno)),
        py::ReadError::Decode(e) => Fail::Decode(e.to_string()),
    })
}

pub fn read_bytes(path: &PyPath) -> Result<Vec<u8>, Fail> {
    std::fs::read(path.fs())
        .map_err(|e| Fail::Os(py::oserror_str(&e, &path.display(), OsStyle::Errno)))
}

pub fn load_sources(root: &PyPath) -> Result<Sources, Fail> {
    let data = root.join("src").join("data");
    let meta = model::load_metadata(&data.join("metadata.yaml"))?;
    model::validate_required_metadata(&meta)?;
    let catalogue = model::load_catalogue(&data.join("tpb-catalogue.yaml"))?;
    model::validate_counts(&meta, &catalogue)?;
    let guide = root.join("src").join("guide");
    let mut fragments = Vec::new();
    for name in markdown_names(&guide.fs()) {
        let text = read_text(&guide.join(&name))?;
        fragments.push((name, text));
    }
    if fragments.is_empty() {
        return Err(Fail::Build("src/guide/ has no fragments".into()));
    }
    let behaviour_path = data.join("behaviour-tests.yaml");
    Ok(Sources {
        meta,
        fragments,
        catalogue,
        behaviour: model::load_behaviour_tests(&behaviour_path)?,
        whole_response: model::load_whole_response(&behaviour_path)?,
        apes: model::load_apes_map(&data.join("apes-110-map.yaml"))?,
        vendor: model::load_vendor_assurance(&data.join("ai-vendor-assurance.yaml"))?,
        concordance: model::load_concordance(&data.join("gs55-concordance.yaml"))?,
        changelog: model::load_changelog(&data.join("changelog.yaml"))?,
    })
}

fn build_changelog_section(s: &Sources) -> String {
    let rows: Vec<Vec<String>> = s
        .changelog
        .iter()
        .map(|r| {
            ["version", "date", "status", "change"]
                .iter()
                .map(|f| r.get(f).to_string())
                .collect()
        })
        .collect();
    "## Change log\n\n".to_string() + &render_table(&CHANGELOG_HEADERS, &["---"; 4], &rows)
}

pub fn build_guide(s: &Sources) -> String {
    let front: Vec<(String, String)> = s
        .meta
        .iter()
        .filter(|(k, _)| k != "checksum_files")
        .cloned()
        .collect();
    let mut out = render_frontmatter(&front);
    for (_, text) in &s.fragments {
        out.push_str(text);
    }
    out.push_str(&build_changelog_section(s));
    out.push('\n');
    out.push_str(s.meta("guide_end_marker"));
    out.push('\n');
    out
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// An ISO date as prose, for example "16 August 2026", with fixed English month names.
fn render_checked_date(value: &str) -> String {
    let d = Date::fromisoformat(py::prefix_chars(value, 10))
        .expect("validated when the sources loaded");
    format!("{} {} {}", d.day, MONTHS[d.month as usize - 1], d.year)
}

pub fn build_catalogue_md(s: &Sources) -> String {
    let rows: Vec<Vec<String>> = s
        .catalogue
        .iter()
        .map(|r| {
            vec![
                format!("[{}]({})", r.get("title"), r.get("url")),
                r.get("trigger").to_string(),
            ]
        })
        .collect();
    let checked = render_checked_date(s.meta("sources_checked_at"));
    let header = format_named(
        CATALOGUE_HEADER_TEMPLATE,
        &[
            ("version", s.meta("guide_version")),
            ("checked", &checked),
            ("count", s.meta("tpb_guidance_statement_count")),
            ("first", s.catalogue[0].get("id")),
            ("last", s.catalogue[s.catalogue.len() - 1].get("id")),
        ],
    );
    header
        + &render_table(
            &["Statement (concise title and official link)", "LLM trigger"],
            &["---", "---"],
            &rows,
        )
}

pub fn build_behaviour_md(s: &Sources) -> String {
    let rows: Vec<Vec<String>> = s
        .behaviour
        .iter()
        .map(|r| {
            model::BEHAVIOUR_FIELDS
                .iter()
                .map(|f| r.get(f).to_string())
                .collect()
        })
        .collect();
    let total = s.behaviour.len().to_string();
    let header = format_named(
        BEHAVIOUR_HEADER_TEMPLATE,
        &[("version", s.meta("guide_version")), ("total", &total)],
    );
    header
        + &render_table(
            &[
                "ID",
                "Scenario",
                "Expected status",
                "Required behaviour and human step",
                "Side-effect check",
            ],
            &["---"; 5],
            &rows,
        )
}

pub fn build_apes_md(s: &Sources) -> String {
    let header = format_named(
        APES_HEADER_TEMPLATE,
        &[("version", s.meta("guide_version"))],
    );
    let pairs = |rows: &[Row]| -> Vec<Vec<String>> {
        rows.iter()
            .map(|r| vec![r.get("label").to_string(), r.get("value").to_string()])
            .collect()
    };
    let a = render_table(
        &["Context", "APES 110 starting points"],
        &["---", "---"],
        &pairs(&s.apes.contexts),
    );
    let b = render_table(
        &["Control", "APES 110 retrieval points"],
        &["---", "---"],
        &pairs(&s.apes.retrieval_points),
    );
    header + &a + APES_BETWEEN_TABLES + &b
}

const VENDOR_HEADERS: [&str; 5] = [
    "ID",
    "Question",
    "Evidence that answers it",
    "Where the obligation sits",
    "If nobody answers",
];

fn sectioned_tables(
    mut out: String,
    data: &Sectioned,
    headers: &[&str],
    fields: &[&str],
) -> String {
    for title in &data.sections {
        let rows: Vec<Vec<String>> = data
            .entries
            .iter()
            .filter(|r| r.get("section") == title)
            .map(|r| fields.iter().map(|f| r.get(f).to_string()).collect())
            .collect();
        out.push_str(&format!("## {title}\n\n"));
        out.push_str(&render_table(headers, &vec!["---"; headers.len()], &rows));
        out.push('\n');
    }
    // Every section contributed a trailing blank line; the file ends with one
    // newline, as the other generated files do.
    out.trim_end_matches('\n').to_string() + "\n"
}

pub fn build_vendor_assurance_md(s: &Sources) -> String {
    let header = format_named(
        VENDOR_HEADER_TEMPLATE,
        &[("version", s.meta("guide_version"))],
    );
    sectioned_tables(
        header,
        &s.vendor,
        &VENDOR_HEADERS,
        &["id", "question", "evidence", "obligation", "if_absent"],
    )
}

const CONCORDANCE_HEADERS: [&str; 5] =
    ["ID", "Source", "Consideration", "DrDebits control", "Note"];

pub fn build_concordance_md(s: &Sources) -> String {
    let issued = render_checked_date(&s.concordance.issued);
    let checked = render_checked_date(&s.concordance.checked);
    let header = format_named(
        CONCORDANCE_HEADER_TEMPLATE,
        &[
            ("version", s.meta("guide_version")),
            ("issued", &issued),
            ("checked", &checked),
        ],
    );
    sectioned_tables(
        header,
        &s.concordance,
        &CONCORDANCE_HEADERS,
        &["id", "source", "consideration", "control", "note"],
    )
}

/// A renderer for one generated file.
pub type Render = fn(&Sources) -> String;

/// The generated files, in the order the Python builder held them.
pub const GENERATED: [(&str, Render); 7] = [
    ("drdebits.md", build_guide),
    ("reference/tpb-catalogue.md", build_catalogue_md),
    ("tests/behaviour-tests.md", build_behaviour_md),
    ("reference/apes-110-map.md", build_apes_md),
    (
        "reference/ai-vendor-assurance.md",
        build_vendor_assurance_md,
    ),
    ("reference/tpb-gs55-concordance.md", build_concordance_md),
    (evals::CASES_FILE, evals::build_cases),
];

pub fn render_generated(s: &Sources) -> Vec<(String, String)> {
    GENERATED
        .iter()
        .map(|(rel, render)| (rel.to_string(), render(s)))
        .collect()
}

pub fn build_sha256sums(
    root: &PyPath,
    s: &Sources,
    generated: &[(String, String)],
) -> Result<String, Fail> {
    let mut lines = Vec::new();
    for rel in s.meta("checksum_files").split('|') {
        let digest = match generated.iter().find(|(k, _)| k == rel) {
            Some((_, content)) => sha256_hex(content.as_bytes()),
            None => sha256_hex(&read_bytes(&root.join(rel))?),
        };
        lines.push(format!("{digest} *{rel}"));
    }
    Ok(lines.join("\n") + "\n")
}

/// The identity digests a result or observation record carries, taken from
/// the build output, not the committed files, so a runner can record them
/// before a rebuild is committed.
pub fn build_digests(s: &Sources) -> Vec<(&'static str, String)> {
    vec![
        ("guide_sha256", sha256_hex(build_guide(s).as_bytes())),
        ("cases_sha256", sha256_hex(evals::build_cases(s).as_bytes())),
    ]
}

/// `STAMP_RE.sub(version, text)`, including `re.sub`'s template escapes.
pub fn stamp_version(text: &str, version: &str) -> Result<String, Fail> {
    let template = parse_template(version)?;
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (start, end) in stamp_matches(text) {
        out.push_str(&text[last..start]);
        for piece in &template {
            match piece {
                Piece::Literal(s) => out.push_str(s),
                Piece::Whole => out.push_str(&text[start..end]),
            }
        }
        last = end;
    }
    out.push_str(&text[last..]);
    Ok(out)
}

enum Piece {
    Literal(String),
    Whole,
}

/// The replacement-string grammar of `re.sub`, for a pattern with no groups.
fn parse_template(source: &str) -> Result<Vec<Piece>, Fail> {
    let bad = |m: String| Fail::Internal(format!("re.PatternError: {m}"));
    let chars: Vec<char> = source.chars().collect();
    let mut pieces = Vec::new();
    let mut literal = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c != '\\' {
            literal.push(c);
            i += 1;
            continue;
        }
        let Some(&next) = chars.get(i + 1) else {
            return Err(bad("bad escape (end of pattern)".into()));
        };
        i += 2;
        match next {
            'g' => {
                if chars.get(i) != Some(&'<') {
                    return Err(bad("missing <".into()));
                }
                let close = chars[i..]
                    .iter()
                    .position(|c| *c == '>')
                    .ok_or_else(|| bad("missing >, unterminated name".into()))?;
                let name: String = chars[i + 1..i + close].iter().collect();
                i += close + 1;
                if name == "0"
                    || name.chars().all(|c| c.is_ascii_digit())
                        && !name.is_empty()
                        && name.trim_start_matches('0').is_empty()
                {
                    pieces.push(Piece::Literal(std::mem::take(&mut literal)));
                    pieces.push(Piece::Whole);
                } else {
                    return Err(bad(format!("invalid group reference {name}")));
                }
            }
            '0' => {
                let mut digits = String::from("0");
                for _ in 0..2 {
                    match chars.get(i) {
                        Some(d) if ('0'..='7').contains(d) => {
                            digits.push(*d);
                            i += 1;
                        }
                        _ => break,
                    }
                }
                let code = u32::from_str_radix(&digits, 8).expect("octal") & 0xff;
                literal.push(char::from_u32(code).expect("a byte value"));
            }
            '1'..='9' => {
                let mut digits = next.to_string();
                if let Some(d) = chars.get(i).filter(|d| d.is_ascii_digit()) {
                    digits.push(*d);
                    i += 1;
                    if ('0'..='7').contains(&next)
                        && ('0'..='7').contains(d)
                        && let Some(e) = chars.get(i).filter(|e| ('0'..='7').contains(*e))
                    {
                        digits.push(*e);
                        i += 1;
                        let code = u32::from_str_radix(&digits, 8).expect("octal");
                        if code > 0o377 {
                            return Err(bad(format!(
                                "octal escape value \\{digits} outside of range 0-0o377"
                            )));
                        }
                        literal.push(char::from_u32(code).expect("a byte value"));
                        continue;
                    }
                }
                return Err(bad(format!(
                    "invalid group reference {}",
                    digits.parse::<u32>().expect("digits")
                )));
            }
            'a' => literal.push('\x07'),
            'b' => literal.push('\x08'),
            'f' => literal.push('\x0c'),
            'n' => literal.push('\n'),
            'r' => literal.push('\r'),
            't' => literal.push('\t'),
            'v' => literal.push('\x0b'),
            '\\' => literal.push('\\'),
            _ if next.is_ascii_alphabetic() => return Err(bad(format!("bad escape \\{next}"))),
            _ => {
                literal.push('\\');
                literal.push(next);
            }
        }
    }
    pieces.push(Piece::Literal(literal));
    Ok(pieces)
}

/// Write every generated file, SHA256SUMS and the results table under `outdir`.
pub fn write_outputs(root: &PyPath, outdir: &PyPath, s: &Sources) -> Result<Vec<String>, Fail> {
    let mut written = render_generated(s);
    let sums = build_sha256sums(root, s, &written)?;
    written.push(("SHA256SUMS".into(), sums));
    // The results table depends on the recorded runs as well as src/.
    written.push((
        evals::RESULTS_FILE.into(),
        evals::build_results_md(root, s)?,
    ));
    for (rel, content) in &written {
        let target = outdir.join(rel);
        let parent = target.parent();
        std::fs::create_dir_all(parent.fs())
            .map_err(|e| Fail::Os(py::oserror_str(&e, &parent.display(), OsStyle::Native)))?;
        std::fs::write(target.fs(), content.as_bytes())
            .map_err(|e| Fail::Os(py::oserror_str(&e, &target.display(), OsStyle::Errno)))?;
    }
    Ok(written.into_iter().map(|(rel, _)| rel).collect())
}
