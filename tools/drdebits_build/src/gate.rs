//! A deterministic gate a host runs on a high-impact draft before showing or
//! acting on it.
//!
//! The guide tells the model what a high-impact draft must contain; prompt
//! wording cannot make it comply. `gate` checks the parts a machine can check
//! and answers in four states:
//!
//! ```text
//! 0   PASS    every mechanical check passed; a human still reviews the draft
//! 1   BLOCK   the draft breaks the output contract or its arithmetic
//! 2   the gate could not run (unreadable response or guide sources)
//! 3   REVIEW  nothing blocked, but the prohibited-conclusion screen fired or
//!             the draft records a state-changing action; a human decides
//! ```
//!
//! Checked: the draft banner; one primary decision status naming one allowed
//! high-impact status; `action_authority: NONE`; a `state_changing_tool_action`
//! line; both source-version status lines with an allowed value; the sections
//! the guide makes mandatory, each with content; and every written calculation
//! of the form `a op b = c`, including each link of a chain, reperformed in
//! exact decimal arithmetic. A minus sign needs spaces around it, so section
//! numbers such as `8-1` and years such as `2026-27` are not subtractions. A
//! lone bracketed amount such as `(120)` blocks as ambiguous, because it may
//! be an accounting negative.
//!
//! Mandatory sections follow Risk classification for `PROCEED_DRAFT_ONLY` and
//! `NEEDS_FACTS`. An `ESCALATE` or `HARD_STOP` draft may be the abbreviated
//! record the Output contract allows, so it must keep the banner, status,
//! action fields and external-action record, and the other sections are
//! reported as notes. Sections the contract requires only "where material"
//! are reported as notes, because materiality is a judgement.
//!
//! Not checked: whether the facts, sources, analysis or conclusion are
//! right, whether a source is current, whether a refusal was required, or
//! whether the free-text scope, reason and next human step are adequate.

use std::sync::LazyLock;

use num_bigint::BigInt;
use num_traits::{Signed, Zero};

use crate::builder;
use crate::error::Fail;
use crate::harness;
use crate::model::Conclusion;
use crate::py::{self, PyPath};
use crate::pyjson::{self, Json};

pub const EXIT_PASS: u8 = 0;
pub const EXIT_BLOCK: u8 = 1;
pub const EXIT_COULD_NOT_RUN: u8 = 2;
pub const EXIT_REVIEW: u8 = 3;

const FULL_STATUSES: [&str; 2] = ["PROCEED_DRAFT_ONLY", "NEEDS_FACTS"];
const STOP_STATUSES: [&str; 2] = ["ESCALATE", "HARD_STOP"];
/// Risk classification: mandatory for every high-impact task.
const MANDATORY: [&str; 5] = [
    "Scope and period",
    "Source-version status",
    "Assumptions",
    "Human actions before use",
    "External-action record",
];
/// Output contract fields required "where material".
const WHERE_MATERIAL: [&str; 5] = [
    "Facts and evidence",
    "Applicable authority",
    "Analysis",
    "Ethical and regulatory flags",
    "Conclusion and uncertainty",
];
const SOURCE_VALUES: [&str; 2] = ["CONFIRMED", "SOURCE CURRENCY NOT CONFIRMED"];

static BANNER: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)^DRAFT\s*[—–:-]\s*PROFESSIONAL REVIEW REQUIRED$").expect("pattern")
});
static FIELD: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)^(action_authority|state_changing_tool_action|historical authority|current duties)\s*:\s*(.*)$")
        .expect("pattern")
});

const NUM: &str = r"\$?\d{1,3}(?:,\d{3})+(?:\.\d+)?%?|\$?\d+(?:\.\d+)?%?";
static CLAIM: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
    // One line only: a bullet's leading hyphen is not a minus sign. Group 1
    // holds every link before the final result, each ending in `=`; the first
    // link needs an operator, so a label such as `Item 2 =` is not one.
    let operand = format!(r"\(*[ \t]*[-−]?(?:{NUM})[ \t]*\)*");
    let op = r"(?:[ \t]*[+×*/÷][ \t]*|[ \t]+[-−x][ \t]+)";
    fancy_regex::Regex::new(&format!(
        r"(?<![\w.,$/])({operand}(?:{op}{operand})+[ \t]*=[ \t]*(?:{operand}(?:{op}{operand})*[ \t]*=[ \t]*)*)(\([ \t]*[-−]?(?:{NUM})[ \t]*\)|[-−]?(?:{NUM}))(?![\d,.]*\d)(?![ \t]*[+×*/÷=])(?![ \t]+[-−x][ \t])"
    ))
    .expect("pattern")
});
static LEADING: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(&format!("^[-−]?(?:{NUM})")).expect("pattern"));
static NUMBER: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(NUM).expect("pattern"));
/// A lone unsigned bracketed amount, `(120)`: grouping, or an accounting
/// negative. `(-120)` carries its sign and is not ambiguous.
static BRACKETED: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(&format!(r"\([ \t]*(?:{NUM})[ \t]*\)")).expect("pattern"));
static DATE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\d{1,2}/\d{1,2}/\d{2,4}").expect("pattern"));
static NUMBERED: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^\d{1,2}[.)]?\s+").expect("pattern"));

/// An exact rational: `n / d` with `d > 0`.
#[derive(Clone, Debug)]
struct Q {
    n: BigInt,
    d: BigInt,
}

impl Q {
    fn new(n: BigInt, d: BigInt) -> Q {
        if d.is_negative() {
            Q { n: -n, d: -d }
        } else {
            Q { n, d }
        }
    }
    fn add(&self, o: &Q, sign: i32) -> Q {
        Q::new(
            &self.n * &o.d + BigInt::from(sign) * &o.n * &self.d,
            &self.d * &o.d,
        )
    }
    fn mul(&self, o: &Q) -> Q {
        Q::new(&self.n * &o.n, &self.d * &o.d)
    }
    fn div(&self, o: &Q) -> Option<Q> {
        (!o.n.is_zero()).then(|| Q::new(&self.n * &o.d, &self.d * &o.n))
    }
    fn neg(&self) -> Q {
        Q::new(-&self.n, self.d.clone())
    }
}

/// A number as written: `$1,100.50`, `10%`, `−3`. Returns its value and the
/// count of decimal places it shows (a percentage counts in percent).
fn number(text: &str) -> Option<(Q, u32, bool)> {
    let negative = text.starts_with(['-', '−']);
    let body = text.trim_start_matches(['-', '−']).trim_start_matches('$');
    let percent = body.ends_with('%');
    let digits = body.trim_end_matches('%').replace(',', "");
    let (whole, frac) = digits.split_once('.').unwrap_or((&digits, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let places = u32::try_from(frac.len()).ok()?;
    let mut n: BigInt = format!("{whole}{frac}").parse().ok()?;
    let mut d = BigInt::from(10).pow(places);
    if percent {
        d *= 100;
    }
    if negative {
        n = -n;
    }
    Some((Q::new(n, d), places, percent))
}

enum Token {
    Num(Q),
    Op(char),
}

fn tokens(text: &str) -> Option<Vec<Token>> {
    let number_at = regex::Regex::new(&format!("^(?:{NUM})")).expect("pattern");
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if c.is_whitespace() {
            rest = &rest[c.len_utf8()..];
        } else if let Some(m) = number_at.find(rest) {
            out.push(Token::Num(number(m.as_str())?.0));
            rest = &rest[m.end()..];
        } else {
            let op = match c {
                '+' | '(' | ')' | '/' => c,
                '-' | '−' => '-',
                '×' | '*' | 'x' => '*',
                '÷' => '/',
                _ => return None,
            };
            out.push(Token::Op(op));
            rest = &rest[c.len_utf8()..];
        }
    }
    Some(out)
}

/// Standard precedence: unary minus, then `*` and `/`, then `+` and `-`.
struct Parser {
    tokens: Vec<Token>,
    at: usize,
    divided_by_zero: bool,
}

impl Parser {
    fn op(&self) -> Option<char> {
        match self.tokens.get(self.at) {
            Some(Token::Op(c)) => Some(*c),
            _ => None,
        }
    }
    fn expr(&mut self) -> Option<Q> {
        let mut value = self.term()?;
        while let Some(op @ ('+' | '-')) = self.op() {
            self.at += 1;
            let right = self.term()?;
            value = value.add(&right, if op == '+' { 1 } else { -1 });
        }
        Some(value)
    }
    fn term(&mut self) -> Option<Q> {
        let mut value = self.factor()?;
        while let Some(op @ ('*' | '/')) = self.op() {
            self.at += 1;
            let right = self.factor()?;
            value = if op == '*' {
                value.mul(&right)
            } else {
                let quotient = value.div(&right);
                self.divided_by_zero |= quotient.is_none();
                quotient?
            };
        }
        Some(value)
    }
    fn factor(&mut self) -> Option<Q> {
        match self.tokens.get(self.at) {
            Some(Token::Op('-')) => {
                self.at += 1;
                Some(self.factor()?.neg())
            }
            Some(Token::Op('(')) => {
                self.at += 1;
                let value = self.expr()?;
                (self.op() == Some(')')).then(|| self.at += 1)?;
                Some(value)
            }
            Some(Token::Num(q)) => {
                let q = q.clone();
                self.at += 1;
                Some(q)
            }
            _ => None,
        }
    }
}

/// The value of an expression: `None` when it does not parse, `Some(None)`
/// when it divides by zero.
fn evaluate(expression: &str) -> Option<Option<Q>> {
    let mut parser = Parser {
        tokens: tokens(expression)?,
        at: 0,
        divided_by_zero: false,
    };
    let value = parser.expr();
    if parser.divided_by_zero {
        return Some(None);
    }
    (parser.at == parser.tokens.len()).then_some(Some(value?))
}

/// Whether `a` is within one unit of the last of `places` decimal places of
/// `b`, counting in percent when `percent` is set; compared as integers.
fn close(a: &Q, b: &Q, places: u32, percent: bool) -> bool {
    let scale = BigInt::from(10).pow(places) * if percent { 100 } else { 1 };
    let difference = (&a.n * &b.d - &b.n * &a.d).abs();
    difference * scale < &a.d * &b.d
}

fn decimal(q: &Q, places: u32) -> String {
    // Half away from zero, for display only: floor(|n| * 10^places / d + 1/2).
    let scaled: BigInt = q.n.abs() * BigInt::from(10).pow(places);
    let magnitude: BigInt = (scaled * 2 + &q.d) / (&q.d * 2);
    let digits = magnitude.to_string();
    let places = places as usize;
    let padded = format!("{digits:0>width$}", width = places + 1);
    let (whole, frac) = padded.split_at(padded.len() - places);
    let sign = if q.n.is_negative() && !magnitude.is_zero() {
        "-"
    } else {
        ""
    };
    if places == 0 {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{frac}")
    }
}

/// One written calculation and whether it reperforms.
pub struct Calculation {
    pub text: String,
    pub computed: String,
    pub matches: bool,
}

/// Every `a op b = c` in the text, reperformed. The claimed result matches
/// when it is within one unit of its last decimal place, so a result rounded
/// up, down or to nearest passes and a wrong digit does not. In a chain
/// `a op b = c op d = e`, each link must equal either the whole next
/// expression or its leading number, as a running total writes it.
pub fn calculations(text: &str) -> Vec<Calculation> {
    let mut out = Vec::new();
    let lines: Vec<String> = py::splitlines(text).into_iter().map(normalise).collect();
    for m in lines
        .iter()
        .flat_map(|line| CLAIM.captures_iter(line.as_str()))
        .filter_map(Result::ok)
    {
        let (whole, left, right) = (&m[0], &m[1], &m[2]);
        if DATE.is_match(left) {
            continue;
        }
        let text = py::strip(whole).to_string();
        if BRACKETED.is_match(whole) {
            out.push(Calculation {
                text,
                computed: "ambiguous: a bracketed amount may be negative; write a minus sign"
                    .into(),
                matches: false,
            });
            continue;
        }
        // A signed bracketed result, `(-50)`.
        let right = right.trim_start_matches('(').trim_end_matches(')').trim();
        let Some((claimed, places, percent)) = number(right) else {
            continue;
        };
        // An opening bracket of the surrounding prose, as in `(2 + 3 = 5)`.
        let unopened = left
            .matches('(')
            .count()
            .saturating_sub(left.matches(')').count());
        let mut links: Vec<&str> = left
            .split('=')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        for _ in 0..unopened {
            links[0] = links[0].strip_prefix('(').unwrap_or(links[0]).trim_start();
        }
        let Some(values) = links
            .iter()
            .map(|l| evaluate(l))
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        let Some(values) = values.into_iter().collect::<Option<Vec<Q>>>() else {
            out.push(Calculation {
                text,
                computed: "undefined (division by zero)".into(),
                matches: false,
            });
            continue;
        };
        let last = values.len() - 1;
        let failed = (0..=last).find(|&i| {
            if i == last {
                return !close(&values[i], &claimed, places, percent);
            }
            let leading = LEADING
                .find(links[i + 1])
                .and_then(|m| number(m.as_str()))
                .is_some_and(|(q, p, pc)| close(&values[i], &q, p, pc));
            // The whole next expression, to the finer of its own written
            // precision and the final result's.
            let written = NUMBER
                .find_iter(links[i + 1])
                .filter_map(|m| number(m.as_str()))
                .map(|(_, p, _)| p)
                .max()
                .unwrap_or(0);
            !leading && !close(&values[i], &values[i + 1], places.max(written), percent)
        });
        let shown = failed.unwrap_or(last);
        let percent = percent && shown == last;
        let value = if percent {
            values[shown].mul(&Q::new(BigInt::from(100), BigInt::from(1)))
        } else {
            values[shown].clone()
        };
        out.push(Calculation {
            text,
            computed: format!(
                "{}{}",
                decimal(&value, places.max(2)),
                if percent { "%" } else { "" }
            ),
            matches: failed.is_none(),
        });
    }
    out
}

/// A line without Markdown emphasis, heading marks or list markers. A marker
/// counts only when whitespace follows it, so `-5` keeps its sign.
fn normalise(line: &str) -> String {
    let text = line.replace("**", "").replace('`', "");
    let mut rest = text.trim();
    loop {
        let hashes = rest.trim_start_matches('#');
        let after = if hashes.len() < rest.len() {
            hashes
        } else {
            rest.strip_prefix(['-', '*', '+', '>']).unwrap_or(rest)
        };
        if after.len() == rest.len()
            || !(after.is_empty() || after.starts_with(char::is_whitespace))
        {
            return rest.to_string();
        }
        rest = after.trim_start();
    }
}

/// The contract section a line heads, ignoring a trailing colon and a
/// numbering prefix such as `1.`.
fn heading_of(line: &str) -> Option<&'static str> {
    let line = line.trim_end_matches(':').trim();
    let line = NUMBERED.find(line).map_or(line, |m| &line[m.end()..]);
    MANDATORY
        .iter()
        .chain(WHERE_MATERIAL.iter())
        .find(|h| line.eq_ignore_ascii_case(h))
        .copied()
}

/// `None` when the section is missing, otherwise whether any line before the
/// next contract section has content.
fn section(lines: &[String], heading: &str) -> Option<bool> {
    let start = lines.iter().position(|l| heading_of(l) == Some(heading))?;
    Some(
        lines[start + 1..]
            .iter()
            .take_while(|l| heading_of(l).is_none())
            // A Markdown rule such as `---` or `***` is not content.
            .any(|l| {
                !l.chars()
                    .all(|c| matches!(c, '-' | '*' | '_' | '=') || c.is_whitespace())
            }),
    )
}

/// The gate's findings for one response.
#[derive(Default)]
pub struct Report {
    pub status: Option<String>,
    pub blocks: Vec<String>,
    pub reviews: Vec<String>,
    pub notes: Vec<String>,
    pub calculations: Vec<Calculation>,
}

impl Report {
    pub fn verdict(&self) -> (&'static str, u8) {
        if !self.blocks.is_empty() {
            ("BLOCK", EXIT_BLOCK)
        } else if !self.reviews.is_empty() {
            ("REVIEW", EXIT_REVIEW)
        } else {
            ("PASS", EXIT_PASS)
        }
    }
}

fn check_fields(lines: &[String], stop: bool, report: &mut Report) {
    let mut fields: Vec<(String, String)> = Vec::new();
    for line in lines {
        if let Some(c) = FIELD.captures(line) {
            fields.push((
                c[1].to_lowercase(),
                py::strip(&c[2]).trim_end_matches('.').to_string(),
            ));
        }
    }
    let values = |key: &str| {
        fields
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .collect::<Vec<_>>()
    };
    let authority = values("action_authority");
    if authority.is_empty() {
        report
            .blocks
            .push("missing field: action_authority: NONE".into());
    }
    for value in authority.iter().filter(|v| **v != "NONE") {
        report.blocks.push(format!(
            "action_authority must be NONE, found {}",
            py::repr_str(value)
        ));
    }
    match values("state_changing_tool_action").as_slice() {
        [] => report
            .blocks
            .push("missing field: state_changing_tool_action".into()),
        values if values.iter().any(|v| v.is_empty()) => {
            report.blocks.push(
                "state_changing_tool_action is empty; write NONE or the record reference".into(),
            );
        }
        values => {
            for value in values.iter().filter(|v| **v != "NONE") {
                report.reviews.push(format!(
                    "state_changing_tool_action records {}; a person must confirm it against the external-action record",
                    py::repr_str(value)
                ));
            }
        }
    }
    for (key, label) in [
        ("historical authority", "Historical authority"),
        ("current duties", "Current duties"),
    ] {
        match values(key).as_slice() {
            [] if stop => report.notes.push(format!(
                "no {label} line; allowed only in an abbreviated escalation record"
            )),
            [] => report
                .blocks
                .push(format!("missing source-version line: {label}")),
            values => {
                // An allowed value, optionally followed by a note: `CONFIRMED (date)`.
                let allowed = |v: &str| {
                    SOURCE_VALUES.iter().any(|s| {
                        v.strip_prefix(s).is_some_and(|rest| {
                            !rest.starts_with(|c: char| c.is_alphanumeric() || c == '_')
                        })
                    })
                };
                for value in values.iter().filter(|v| !allowed(v)) {
                    report.blocks.push(format!(
                        "{label} must be CONFIRMED or SOURCE CURRENCY NOT CONFIRMED, found {}",
                        py::repr_str(value)
                    ));
                }
            }
        }
    }
}

/// Check one response against the output contract and its own arithmetic.
pub fn check(response: &str, conclusions: &[Conclusion]) -> Report {
    let mut report = Report::default();
    let lines: Vec<String> = py::splitlines(response)
        .into_iter()
        .map(normalise)
        .collect();
    let first = lines
        .iter()
        .find(|l| !l.is_empty())
        .map_or("", String::as_str);
    if !BANNER.is_match(first) {
        report
            .blocks
            .push("the first line must be the banner DRAFT — PROFESSIONAL REVIEW REQUIRED".into());
    }
    let statuses = harness::decision_statuses(response);
    match statuses.as_slice() {
        [one] if FULL_STATUSES.contains(&one.as_str()) || STOP_STATUSES.contains(&one.as_str()) => {
            report.status = Some(one.clone());
        }
        [one] if !one.is_empty() => report.blocks.push(format!(
            "{} is not a high-impact decision status; this gate applies to high-impact drafts",
            py::repr_str(one)
        )),
        [] | [_] => report
            .blocks
            .push("no primary decision status selects one allowed status".into()),
        _ => report.blocks.push(format!(
            "{} decision fields; write exactly one 'Primary decision status:' line",
            statuses.len()
        )),
    }
    let stop = report
        .status
        .as_deref()
        .is_some_and(|s| STOP_STATUSES.contains(&s));
    check_fields(&lines, stop, &mut report);
    for heading in MANDATORY {
        let problem = match section(&lines, heading) {
            Some(true) => continue,
            Some(false) => format!("empty {heading} section"),
            None => format!("no {heading} section"),
        };
        if stop && heading != "External-action record" {
            report.notes.push(format!(
                "{problem}; allowed only in an abbreviated escalation record"
            ));
        } else {
            report
                .blocks
                .push(format!("missing mandatory section: {heading} ({problem})"));
        }
    }
    for heading in WHERE_MATERIAL.iter().filter(|_| !stop) {
        let problem = match section(&lines, heading) {
            Some(true) => continue,
            Some(false) => "empty",
            None => "no",
        };
        report.notes.push(format!(
            "{problem} {heading} section; required where material"
        ));
    }
    report.calculations = calculations(response);
    for c in report.calculations.iter().filter(|c| !c.matches) {
        report.blocks.push(format!(
            "calculation does not reperform: {} (computes to {})",
            c.text, c.computed
        ));
    }
    for label in harness::prohibited_hits(response, conclusions) {
        report.reviews.push(format!("prohibited-conclusion screen: {label}; a human must confirm whether it is concluded or refused"));
    }
    report
}

fn report_json(report: &Report) -> Json {
    let list = |items: &[String]| Json::Array(items.iter().map(|s| Json::str_obj(s)).collect());
    let (verdict, _) = report.verdict();
    Json::Object(vec![
        ("verdict".into(), Json::str_obj(verdict)),
        (
            "status".into(),
            report.status.as_deref().map_or(Json::Null, Json::str_obj),
        ),
        ("blocks".into(), list(&report.blocks)),
        ("reviews".into(), list(&report.reviews)),
        ("notes".into(), list(&report.notes)),
        (
            "calculations".into(),
            Json::Array(
                report
                    .calculations
                    .iter()
                    .map(|c| {
                        Json::Object(vec![
                            ("text".into(), Json::str_obj(&c.text)),
                            ("computed".into(), Json::str_obj(&c.computed)),
                            ("matches".into(), Json::Bool(c.matches)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

#[derive(clap::Args)]
pub struct Args {
    /// The model response to check, or - for standard input.
    response: String,
    /// Print the findings as JSON.
    #[arg(long)]
    json: bool,
}

fn read_response(path: &str) -> Result<String, Fail> {
    if path == "-" {
        let mut text = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
            .map_err(|e| Fail::Os(e.to_string()))?;
        Ok(text)
    } else {
        builder::read_text(&PyPath::new(path))
    }
}

pub fn main(root: &Option<String>, args: Args) -> u8 {
    let loaded = crate::root_for(root)
        .and_then(|root| builder::load_sources(&root))
        .and_then(|sources| Ok((sources, read_response(&args.response)?)));
    let (sources, response) = match loaded {
        Ok(found) => found,
        Err(e) => {
            errln!("gate: {e}");
            return EXIT_COULD_NOT_RUN;
        }
    };
    let report = check(&response, &sources.whole_response.prohibited_conclusions);
    let (verdict, code) = report.verdict();
    if args.json {
        outln!("{}", pyjson::dumps(&report_json(&report), false));
        return code;
    }
    outln!(
        "gate: {verdict}{}",
        report
            .status
            .as_deref()
            .map_or(String::new(), |s| format!(" ({s})"))
    );
    for (label, items) in [
        ("block", &report.blocks),
        ("review", &report.reviews),
        ("note", &report.notes),
    ] {
        for item in items {
            outln!("  {label}: {item}");
        }
    }
    for c in report.calculations.iter().filter(|c| c.matches) {
        outln!("  reperformed: {}", c.text);
    }
    code
}
