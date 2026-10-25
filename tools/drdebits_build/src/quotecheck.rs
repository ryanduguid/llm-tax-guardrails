//! Offline quote evidence checks; text membership never establishes legal support.

use unicode_normalization::UnicodeNormalization;

use crate::builder::sha256_hex;
use crate::py;
use crate::pyjson::{self, Json};

#[derive(clap::Args)]
pub struct Args {
    /// The source text the reviewer or model saw, as a UTF-8 file.
    #[arg(long)]
    source: String,
    /// The quote to look for.
    #[arg(
        long,
        conflicts_with = "quote_file",
        required_unless_present = "quote_file"
    )]
    quote: Option<String>,
    /// The quote to look for, as a UTF-8 file.
    #[arg(long)]
    quote_file: Option<String>,
    /// How many characters of the source were visible; all of it by default.
    #[arg(long)]
    characters_used: Option<i64>,
    /// The supplied source is an excerpt, not the complete document.
    #[arg(long)]
    source_incomplete: bool,
}

fn normalise(text: &str) -> String {
    let nfkc: String = text.nfkc().collect();
    py::split_whitespace(&nfkc).join(" ")
}

/// Index of `needle` in `haystack` in characters, as `str.find` returns it.
fn char_find(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .find(needle)
        .map(|byte| haystack[..byte].chars().count())
}

/// Assess only text the reviewer or model saw, with explicit coverage.
///
/// Offsets refer to the original supplied text and exist only for exact
/// matches. A normalised match is a review lead, never verification of a
/// verbatim quote. `source_complete` concerns the supplied document, not its
/// legal currency.
pub fn assess_quote(
    source_text: &str,
    quote: &str,
    characters_used: Option<i64>,
    source_complete: bool,
) -> Result<Json, String> {
    if py::strip(quote).is_empty() {
        return Err("Source and a nonblank quote must be text".into());
    }
    let length = py::char_len(source_text) as i64;
    let used = characters_used.unwrap_or(length);
    if !(0..=length).contains(&used) {
        return Err("Characters used must be an integer within the supplied text".into());
    }
    let visible = py::prefix_chars(source_text, used as usize);
    let complete = source_complete && used == length;
    let start = char_find(visible, quote);
    let status = if start.is_some() {
        "EXACT_MATCH"
    } else if normalise(visible).contains(&normalise(quote)) {
        "NORMALISED_MATCH"
    } else if complete {
        "NOT_FOUND"
    } else {
        "UNVERIFIED"
    };
    let offset = |v: Option<usize>| v.map_or(Json::Null, |n| Json::Int((n as i64).into()));
    Ok(Json::Object(vec![
        ("status".into(), Json::str_obj(status)),
        (
            "source_sha256".into(),
            Json::str_obj(&sha256_hex(source_text.as_bytes())),
        ),
        (
            "visible_sha256".into(),
            Json::str_obj(&sha256_hex(visible.as_bytes())),
        ),
        ("source_characters".into(), Json::Int(length.into())),
        ("characters_used".into(), Json::Int(used.into())),
        ("source_complete".into(), Json::Bool(source_complete)),
        ("coverage_complete".into(), Json::Bool(complete)),
        ("start".into(), offset(start)),
        ("end".into(), offset(start.map(|s| s + py::char_len(quote)))),
        (
            "boundary".into(),
            Json::str_obj("Text membership only; applicability and claim support require review."),
        ),
    ]))
}

fn read_utf8(path: &str) -> Result<String, String> {
    let bytes = std::fs::read(path)
        .map_err(|e| py::oserror_str(&e, &py::PyPath::new(path).display(), py::OsStyle::Errno))?;
    py::decode_utf8(&bytes).map_err(|e| e.to_string())
}

pub fn main(args: Args) -> u8 {
    let result = (|| -> Result<Json, String> {
        let source = read_utf8(&args.source)?;
        let quote = match (&args.quote, &args.quote_file) {
            (Some(q), _) => q.clone(),
            (None, Some(file)) => read_utf8(file)?,
            (None, None) => unreachable!("clap requires one"),
        };
        assess_quote(
            &source,
            &quote,
            args.characters_used,
            !args.source_incomplete,
        )
    })();
    match result {
        Ok(json) => {
            outln!("{}", pyjson::dumps(&json, false));
            0
        }
        Err(e) => {
            errln!("quote: {e}");
            1
        }
    }
}
