//! Load and validate the YAML sources of truth. All leaf values are strings.

use std::sync::LazyLock;

use num_bigint::BigInt;
use regex::Regex;

use crate::error::Fail;
use crate::py::{self, Date, PyPath};
use crate::yaml::{self, Value};

pub const ALLOWED_STATUSES: [&str; 5] = [
    "HARD_STOP",
    "ESCALATE",
    "NEEDS_FACTS",
    "PROCEED_DRAFT_ONLY",
    "Low impact — proportionate answer",
];

// A catalogue id is a TPB Guidance Statement number. Both the generated
// catalogue header and verify's range check publish the first and last rows as
// the authoritative "GSxx to GSyy" range, so the rows have to be real GS ids in
// ascending order: the builder and the verifier read the same two rows and
// would otherwise agree with each other about bad data.
static CATALOGUE_ID_RE: LazyLock<Regex> = LazyLock::new(|| id_pattern("GS"));
// A vendor assurance id: numbered and ascending, so a duplicated or reordered
// row is visible instead of silently renumbering what a firm recorded.
static VENDOR_ID_RE: LazyLock<Regex> = LazyLock::new(|| id_pattern("VA-"));
// A concordance id carries the same invariant: CC-12 keeps meaning that row.
static CONCORDANCE_ID_RE: LazyLock<Regex> = LazyLock::new(|| id_pattern("CC-"));

fn id_pattern(prefix: &str) -> Regex {
    Regex::new(&format!(r"^{}({}+)\z", regex::escape(prefix), *py::D)).expect("a valid pattern")
}

/// `int(digits)` for the digits an id pattern captured, which may be any decimal digits.
fn id_number(digits: &str) -> BigInt {
    let ascii: String = digits
        .chars()
        .map(|c| {
            char::from_digit(py::decimal_value(c).expect("a decimal digit"), 10).expect("a digit")
        })
        .collect();
    BigInt::parse_bytes(ascii.as_bytes(), 10).expect("decimal digits")
}

/// A string-valued source row, with any optional fields it declared kept raw.
#[derive(Clone, Debug)]
pub struct Row {
    fields: Vec<(String, String)>,
    optional: Vec<(String, Value)>,
}

impl Row {
    /// A row of string fields, as a loader would return it.
    pub fn new(fields: &[(&str, &str)]) -> Row {
        Row {
            fields: fields
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            optional: Vec::new(),
        }
    }

    pub fn get(&self, field: &str) -> &str {
        self.fields
            .iter()
            .find(|(k, _)| k == field)
            .map(|(_, v)| v.as_str())
            .expect("a declared field")
    }

    /// Replace one field's value.
    pub fn set(&mut self, field: &str, value: &str) {
        if let Some(slot) = self.fields.iter_mut().find(|(k, _)| k == field) {
            slot.1 = value.to_string();
        }
    }

    pub fn optional(&self, field: &str) -> Option<&Value> {
        self.optional
            .iter()
            .find(|(k, _)| k == field)
            .map(|(_, v)| v)
    }
}

fn model(message: String) -> Fail {
    Fail::Model(message)
}

fn repr(value: &Value) -> Result<String, Fail> {
    value
        .repr()
        .map_err(|e| Fail::Internal(format!("ValueError: {e}")))
}

fn tuple_repr(items: &[&str]) -> String {
    let parts: Vec<String> = items.iter().map(|s| py::repr_str(s)).collect();
    if parts.len() == 1 {
        format!("({},)", parts[0])
    } else {
        format!("({})", parts.join(", "))
    }
}

pub fn list_repr(items: &[String]) -> String {
    format!(
        "[{}]",
        items
            .iter()
            .map(|s| py::repr_str(s))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn read(path: &PyPath) -> Result<Value, Fail> {
    let bytes = std::fs::read(path.fs()).map_err(|e| {
        model(format!(
            "{path}: cannot read source file ({})",
            py::oserror_str(&e, &path.display(), py::OsStyle::Errno)
        ))
    })?;
    read_bytes(path, &bytes)
}

/// Read a source file's bytes as the loaders do: UTF-8 text, then one YAML
/// mapping, refusing anything else with a finding that names `path`.
pub fn read_bytes(path: &PyPath, bytes: &[u8]) -> Result<Value, Fail> {
    let text =
        py::decode_utf8(bytes).map_err(|e| model(format!("{path}: is not valid UTF-8 ({e})")))?;
    let data = match yaml::load(&py::universal_newlines(&text)) {
        Ok(data) => data,
        Err(yaml::Error::Yaml(message)) => return Err(model(format!("{path}: {message}"))),
        Err(yaml::Error::TooDeep) => {
            return Err(model(format!("{path}: is nested too deeply to read")));
        }
        Err(yaml::Error::Internal(message)) => return Err(Fail::Internal(message)),
    };
    if !matches!(data, Value::Dict(_)) {
        return Err(model(format!("{path}: top level must be a mapping")));
    }
    Ok(data)
}

fn require_str(path: &PyPath, place: &str, value: Option<&Value>) -> Result<String, Fail> {
    match value {
        Some(Value::Str(s)) if !s.is_empty() => Ok(s.clone()),
        Some(other) => Err(model(format!(
            "{path}: {place} must be a non-empty string, got {}",
            repr(other)?
        ))),
        None => Err(model(format!(
            "{path}: {place} must be a non-empty string, got None"
        ))),
    }
}

fn dict_keys(value: &Value) -> Option<Vec<Value>> {
    match value {
        Value::Dict(pairs) => Some(pairs.borrow().iter().map(|(k, _)| k.clone()).collect()),
        _ => None,
    }
}

/// Every key is one of `allowed`, and every one of `required` is present.
fn keys_between(keys: &[Value], required: &[&str], allowed: &[&str]) -> bool {
    let names: Vec<Option<&str>> = keys.iter().map(Value::as_str).collect();
    names
        .iter()
        .all(|k| k.is_some_and(|k| allowed.contains(&k)))
        && required.iter().all(|f| names.contains(&Some(f)))
}

fn rows(
    path: &PyPath,
    data: &Value,
    key: &str,
    fields: &[&str],
    table_safe: bool,
    optional: &[&str],
) -> Result<Vec<Row>, Fail> {
    let entries = match data.get(key) {
        Some(Value::List(items)) if !items.borrow().is_empty() => items.borrow().clone(),
        _ => return Err(model(format!("{path}: '{key}' must be a non-empty list"))),
    };
    let allowed: Vec<&str> = fields.iter().chain(optional).copied().collect();
    let mut out = Vec::new();
    for (i, row) in entries.iter().enumerate() {
        let ok = dict_keys(row).is_some_and(|keys| keys_between(&keys, fields, &allowed));
        if !ok {
            let mut message = format!(
                "{path}: entry {i} must have exactly fields {}",
                tuple_repr(fields)
            );
            if !optional.is_empty() {
                message.push_str(&format!(", optionally with {}", tuple_repr(optional)));
            }
            return Err(model(message));
        }
        let mut clean = Row {
            fields: Vec::new(),
            optional: Vec::new(),
        };
        for f in fields {
            let value = require_str(path, &format!("entry {i} field '{f}'"), row.get(f).as_ref())?;
            // No source value may contain a newline (either kind): a table row
            // would split across lines and a metadata value would inject extra
            // frontmatter lines into the guide, and verify would bless both.
            // Pipes are rejected in table-bound values only, because
            // metadata's checksum_files uses '|' as its separator.
            if value.contains('\n') || value.contains('\r') {
                return Err(model(format!(
                    "{path}: entry {i} field '{f}' contains a newline"
                )));
            }
            if table_safe && value.contains('|') {
                return Err(model(format!(
                    "{path}: entry {i} field '{f}' contains '|', which would break the rendered Markdown table"
                )));
            }
            clean.fields.push((f.to_string(), value));
        }
        for f in optional {
            if let Some(v) = row.get(f) {
                clean.optional.push((f.to_string(), v));
            }
        }
        out.push(clean);
    }
    Ok(out)
}

/// A non-empty list of distinct one-line strings, pipe-free.
fn string_list(path: &PyPath, place: &str, value: &Value) -> Result<Vec<String>, Fail> {
    let items = match value {
        Value::List(items) if !items.borrow().is_empty() => items.borrow().clone(),
        _ => {
            return Err(model(format!(
                "{path}: {place} must be a non-empty list of strings"
            )));
        }
    };
    let mut out: Vec<String> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let item = require_str(path, &format!("{place} item {i}"), Some(item))?;
        if item.contains(['\r', '\n', '|']) {
            return Err(model(format!(
                "{path}: {place} item {i} must be one line with no '|'"
            )));
        }
        out.push(item);
    }
    let mut seen = std::collections::HashSet::new();
    if !out.iter().all(|s| seen.insert(s.clone())) {
        return Err(model(format!("{path}: {place} repeats a value")));
    }
    Ok(out)
}

fn unique_ids(path: &PyPath, rows: &[Row]) -> Result<(), Fail> {
    let mut seen = std::collections::HashSet::new();
    if !rows.iter().all(|r| seen.insert(r.get("id").to_string())) {
        return Err(model(format!("{path}: duplicate ids")));
    }
    Ok(())
}

/// Metadata, in the order the source lists it.
pub type Meta = Vec<(String, String)>;

pub fn meta_get<'a>(meta: &'a Meta, key: &str) -> Option<&'a str> {
    meta.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

pub fn load_metadata(path: &PyPath) -> Result<Meta, Fail> {
    let rows = rows(path, &read(path)?, "fields", &["key", "value"], false, &[])?;
    let mut seen = std::collections::HashSet::new();
    if !rows.iter().all(|r| seen.insert(r.get("key").to_string())) {
        return Err(model(format!("{path}: duplicate metadata keys")));
    }
    let meta: Meta = rows
        .iter()
        .map(|r| (r.get("key").to_string(), r.get("value").to_string()))
        .collect();
    // The catalogue header derives prose from this value, so a malformed date
    // must surface here as a finding, not later inside the builders.
    if let Some(checked) = meta_get(&meta, "sources_checked_at")
        && Date::fromisoformat(py::prefix_chars(checked, 10)).is_none()
    {
        return Err(model(format!(
            "{path}: sources_checked_at must start with an ISO date (YYYY-MM-DD), got {}",
            py::repr_str(checked)
        )));
    }
    Ok(meta)
}

fn ascending_ids(
    path: &PyPath,
    rows: &[Row],
    pattern: &Regex,
    id_word: &str,
    reason: &str,
) -> Result<(), Fail> {
    let mut previous: Option<(BigInt, String)> = None;
    for r in rows {
        let id = r.get("id");
        let Some(caps) = pattern.captures(id) else {
            return Err(model(format!(
                "{path}: id {} must be a {id_word}",
                py::repr_str(id)
            )));
        };
        let number = id_number(&caps[1]);
        if let Some((prev_number, prev_id)) = &previous
            && number <= *prev_number
        {
            return Err(model(format!(
                "{path}: id {id} does not come after {prev_id}; {reason}"
            )));
        }
        previous = Some((number, id.to_string()));
    }
    Ok(())
}

pub fn load_catalogue(path: &PyPath) -> Result<Vec<Row>, Fail> {
    let rows = rows(
        path,
        &read(path)?,
        "entries",
        &["id", "title", "url", "trigger"],
        true,
        &[],
    )?;
    unique_ids(path, &rows)?;
    let mut previous: Option<(BigInt, String)> = None;
    for r in &rows {
        let id = r.get("id");
        let Some(caps) = CATALOGUE_ID_RE.captures(id) else {
            return Err(model(format!(
                "{path}: id {} must be a Guidance Statement id (GS followed by digits)",
                py::repr_str(id)
            )));
        };
        let number = id_number(&caps[1]);
        if let Some((prev_number, prev_id)) = &previous
            && number <= *prev_number
        {
            return Err(model(format!(
                "{path}: id {id} does not come after {prev_id}; entries must be in ascending id order, \
                     because the catalogue header and verify both publish the first and last rows as the \
                     statement range"
            )));
        }
        previous = Some((number, id.to_string()));
        if !r.get("url").starts_with("https://") {
            return Err(model(format!("{path}: url for {id} must be https")));
        }
    }
    Ok(rows)
}

pub const BEHAVIOUR_FIELDS: [&str; 5] = [
    "id",
    "scenario",
    "expected_status",
    "required_behaviour",
    "side_effect_check",
];
/// The optional exact inputs a case gives the model, in export order.
pub const BEHAVIOUR_INPUT_FIELDS: [&str; 2] = ["prompt", "system_instruction"];
/// The optional per-case field emphasising the prohibited conclusions that
/// case turns on. Every declared conclusion makes a response a `violation`
/// wherever it appears, so this field narrows nothing.
pub const PROHIBITED_ANYWHERE: &str = "prohibited_anywhere";
const WHOLE_RESPONSE_FIELDS: [&str; 2] = ["verdict", "rule"];
/// The only verdict the whole-response rule can carry: the scorer records a
/// whole-response breach as `violation`.
const WHOLE_RESPONSE_VERDICT: &str = "violation";
const PROHIBITED_CONCLUSION_FIELDS: [&str; 3] = ["label", "definition", "markers"];

/// A behaviour test, with its optional fields checked.
#[derive(Clone, Debug)]
pub struct Behaviour {
    pub row: Row,
    pub prompt: Option<String>,
    pub system_instruction: Option<String>,
    pub prohibited_anywhere: Option<Vec<String>>,
}

impl Behaviour {
    pub fn get(&self, field: &str) -> &str {
        self.row.get(field)
    }
}

pub fn load_behaviour_tests(path: &PyPath) -> Result<Vec<Behaviour>, Fail> {
    let rows = rows(
        path,
        &read(path)?,
        "entries",
        &BEHAVIOUR_FIELDS,
        true,
        &[PROHIBITED_ANYWHERE, "prompt", "system_instruction"],
    )?;
    unique_ids(path, &rows)?;
    let mut out = Vec::new();
    for r in rows {
        let id = r.get("id").to_string();
        let status = r.get("expected_status");
        if !ALLOWED_STATUSES.contains(&status) {
            return Err(model(format!(
                "{path}: {id} has unknown status {}",
                py::repr_str(status)
            )));
        }
        let mut inputs = [None, None];
        for (field, slot) in BEHAVIOUR_INPUT_FIELDS.iter().zip(&mut inputs) {
            if let Some(value) = r.optional(field) {
                let text = require_str(path, &format!("{id} {field}"), Some(value))?;
                if py::strip(&text).is_empty() {
                    return Err(model(format!("{path}: {id} {field} must not be blank")));
                }
                *slot = Some(text);
            }
        }
        let [prompt, system_instruction] = inputs;
        if system_instruction.is_some() && prompt.is_none() {
            return Err(model(format!(
                "{path}: {id} system_instruction requires an exact prompt"
            )));
        }
        let mut prohibited = None;
        if let Some(value) = r.optional(PROHIBITED_ANYWHERE) {
            prohibited = Some(string_list(
                path,
                &format!("{id} {PROHIBITED_ANYWHERE}"),
                value,
            )?);
        }
        out.push(Behaviour {
            row: r,
            prompt,
            system_instruction,
            prohibited_anywhere: prohibited,
        });
    }
    Ok(out)
}

#[derive(Clone, Debug)]
pub struct Conclusion {
    pub label: String,
    pub definition: String,
    pub markers: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct WholeResponse {
    pub verdict: String,
    pub rule: String,
    pub prohibited_conclusions: Vec<Conclusion>,
}

/// Load the whole-response rule and the prohibited-conclusion labels.
///
/// The rule is machine-readable on purpose: a runner reads it from
/// `evals/cases.json` rather than inferring it from prose, and each label is
/// defined exactly once here, which is what makes a per-case
/// `prohibited_anywhere` value checkable.
pub fn load_whole_response(path: &PyPath) -> Result<WholeResponse, Fail> {
    let data = read(path)?;
    let rule = data.get("whole_response");
    let rule_ok = rule.as_ref().and_then(dict_keys).is_some_and(|keys| {
        keys.len() == WHOLE_RESPONSE_FIELDS.len()
            && keys_between(&keys, &WHOLE_RESPONSE_FIELDS, &WHOLE_RESPONSE_FIELDS)
    });
    if !rule_ok {
        return Err(model(format!(
            "{path}: 'whole_response' must be a mapping with exactly fields {}",
            tuple_repr(&WHOLE_RESPONSE_FIELDS)
        )));
    }
    let rule = rule.expect("checked above");
    let mut values = Vec::new();
    for field in WHOLE_RESPONSE_FIELDS {
        let shown = py::repr_str(field);
        let value = require_str(
            path,
            &format!("whole_response field {shown}"),
            rule.get(field).as_ref(),
        )?;
        if value.contains(['\r', '\n', '|']) {
            return Err(model(format!(
                "{path}: whole_response field {shown} must be one line with no '|'"
            )));
        }
        values.push(value);
    }
    if values[0] != WHOLE_RESPONSE_VERDICT {
        return Err(model(format!(
            "{path}: whole_response verdict must be {}, got {}; the exported rule cannot disagree with the scorer",
            py::repr_str(WHOLE_RESPONSE_VERDICT),
            py::repr_str(&values[0])
        )));
    }
    let entries = match data.get("prohibited_conclusions") {
        Some(Value::List(items)) if !items.borrow().is_empty() => items.borrow().clone(),
        _ => {
            return Err(model(format!(
                "{path}: 'prohibited_conclusions' must be a non-empty list"
            )));
        }
    };
    let mut conclusions: Vec<Conclusion> = Vec::new();
    for (i, row) in entries.iter().enumerate() {
        let ok = dict_keys(row).is_some_and(|keys| {
            keys.len() == PROHIBITED_CONCLUSION_FIELDS.len()
                && keys_between(
                    &keys,
                    &PROHIBITED_CONCLUSION_FIELDS,
                    &PROHIBITED_CONCLUSION_FIELDS,
                )
        });
        if !ok {
            return Err(model(format!(
                "{path}: prohibited conclusion {i} must have exactly fields {}",
                tuple_repr(&PROHIBITED_CONCLUSION_FIELDS)
            )));
        }
        let mut clean = Vec::new();
        for field in ["label", "definition"] {
            let shown = py::repr_str(field);
            let value = require_str(
                path,
                &format!("prohibited conclusion {i} field {shown}"),
                row.get(field).as_ref(),
            )?;
            if value.contains(['\r', '\n', '|']) {
                return Err(model(format!(
                    "{path}: prohibited conclusion {i} field {shown} must be one line with no '|'"
                )));
            }
            clean.push(value);
        }
        let markers = string_list(
            path,
            &format!("prohibited conclusion {} markers", py::repr_str(&clean[0])),
            &row.get("markers").expect("checked above"),
        )?;
        conclusions.push(Conclusion {
            label: clean[0].clone(),
            definition: clean[1].clone(),
            markers,
        });
    }
    let mut seen = std::collections::HashSet::new();
    if !conclusions.iter().all(|c| seen.insert(c.label.clone())) {
        return Err(model(format!(
            "{path}: duplicate prohibited-conclusion label; each label is defined once"
        )));
    }
    Ok(WholeResponse {
        verdict: values[0].clone(),
        rule: values[1].clone(),
        prohibited_conclusions: conclusions,
    })
}

pub fn load_changelog(path: &PyPath) -> Result<Vec<Row>, Fail> {
    rows(
        path,
        &read(path)?,
        "entries",
        &["version", "date", "status", "change"],
        true,
        &[],
    )
}

#[derive(Clone, Debug)]
pub struct ApesMap {
    pub contexts: Vec<Row>,
    pub retrieval_points: Vec<Row>,
}

pub fn load_apes_map(path: &PyPath) -> Result<ApesMap, Fail> {
    let data = read(path)?;
    let mut tables = Vec::new();
    for key in ["contexts", "retrieval_points"] {
        let single = Value::Dict(std::rc::Rc::new(std::cell::RefCell::new(vec![(
            Value::Str(key.into()),
            data.get(key).unwrap_or(Value::None),
        )])));
        tables.push(rows(path, &single, key, &["label", "value"], true, &[])?);
    }
    let retrieval_points = tables.pop().expect("two tables");
    let contexts = tables.pop().expect("two tables");
    Ok(ApesMap {
        contexts,
        retrieval_points,
    })
}

pub const VENDOR_FIELDS: [&str; 6] = [
    "id",
    "section",
    "question",
    "evidence",
    "obligation",
    "if_absent",
];

/// Declared sections, then numbered rows grouped by section, in order.
#[derive(Clone, Debug)]
pub struct Sectioned {
    pub sections: Vec<String>,
    pub entries: Vec<Row>,
    pub issued: String,
    pub checked: String,
}

fn sectioned_rows(
    path: &PyPath,
    data: &Value,
    fields: &[&str],
    id_re: &Regex,
    id_word: &str,
) -> Result<Sectioned, Fail> {
    let titles = match data.get("sections") {
        Some(Value::List(items)) if !items.borrow().is_empty() => items.borrow().clone(),
        _ => {
            return Err(model(format!(
                "{path}: 'sections' must be a non-empty list"
            )));
        }
    };
    let mut declared: Vec<String> = Vec::new();
    for (i, title) in titles.iter().enumerate() {
        let title = require_str(path, &format!("section {i}"), Some(title))?;
        // A heading is rendered from this value, so a newline would inject
        // document structure the sources never declared.
        if title.contains('\n') || title.contains('\r') {
            return Err(model(format!("{path}: section {i} contains a newline")));
        }
        declared.push(title);
    }
    let mut seen = std::collections::HashSet::new();
    if !declared.iter().all(|t| seen.insert(t.clone())) {
        return Err(model(format!("{path}: duplicate section titles")));
    }
    let rows = rows(path, data, "entries", fields, true, &[])?;
    unique_ids(path, &rows)?;
    ascending_ids(
        path,
        &rows,
        id_re,
        id_word,
        "entries must be in ascending id order so a firm's recorded reference keeps meaning the same row",
    )?;
    let mut order: Vec<String> = Vec::new();
    for r in &rows {
        let section = r.get("section");
        if !declared.iter().any(|d| d == section) {
            return Err(model(format!(
                "{path}: {} names section {}, which is not declared",
                r.get("id"),
                py::repr_str(section)
            )));
        }
        if order.last().map(String::as_str) != Some(section) {
            if order.iter().any(|o| o == section) {
                return Err(model(format!(
                    "{path}: {} resumes section {} after another section; a section's entries must be \
                     contiguous, because each section renders as one table",
                    r.get("id"),
                    py::repr_str(section)
                )));
            }
            order.push(section.to_string());
        }
    }
    if order != declared {
        let missing: Vec<String> = declared
            .iter()
            .filter(|t| !order.contains(t))
            .cloned()
            .collect();
        return Err(model(format!(
            "{path}: sections with entries {} do not match the declared order {}; sections without entries: {}",
            list_repr(&order),
            list_repr(&declared),
            list_repr(&missing)
        )));
    }
    Ok(Sectioned {
        sections: declared,
        entries: rows,
        issued: String::new(),
        checked: String::new(),
    })
}

/// Load the AI vendor assurance checklist: declared sections, then rows.
///
/// The builder renders one heading and one table per declared section and
/// picks each table's rows by section, so the rows must carry that grouping:
/// every row names a declared section, a section's rows sit together, and
/// every declared section has rows.
pub fn load_vendor_assurance(path: &PyPath) -> Result<Sectioned, Fail> {
    sectioned_rows(
        path,
        &read(path)?,
        &VENDOR_FIELDS,
        &VENDOR_ID_RE,
        "checklist id (VA- followed by digits)",
    )
}

pub const CONCORDANCE_FIELDS: [&str; 6] = [
    "id",
    "section",
    "source",
    "consideration",
    "control",
    "note",
];

/// Load the TPB(GS) 55/2026 concordance: its pinned dates, then sections and rows.
///
/// The issue date names the Guidance Statement revision the rows were written
/// against and the check date says when it was last read, so a later TPB
/// revision is a visible source-currency event rather than a silent drift.
pub fn load_concordance(path: &PyPath) -> Result<Sectioned, Fail> {
    let data = read(path)?;
    let mut out = sectioned_rows(
        path,
        &data,
        &CONCORDANCE_FIELDS,
        &CONCORDANCE_ID_RE,
        "concordance id (CC- followed by digits)",
    )?;
    let mut dates = Vec::new();
    for key in ["issued", "checked"] {
        let value = require_str(path, key, data.get(key).as_ref())?;
        if Date::fromisoformat(&value).is_none() {
            return Err(model(format!(
                "{path}: {key} must be an ISO date, got {}",
                py::repr_str(&value)
            )));
        }
        dates.push(value);
    }
    out.checked = dates.pop().expect("two dates");
    out.issued = dates.pop().expect("two dates");
    Ok(out)
}

/// Every key the builders and verifier dereference unconditionally.
pub const REQUIRED_METADATA_KEYS: [&str; 8] = [
    "guide_version",
    "release_tag",
    "guide_end_marker",
    "sources_checked_at",
    "review_due",
    "tpb_guidance_statement_count",
    "tpb_library_index_count",
    "checksum_files",
];

pub fn validate_required_metadata(meta: &Meta) -> Result<(), Fail> {
    let missing: Vec<String> = REQUIRED_METADATA_KEYS
        .iter()
        .filter(|k| meta_get(meta, k).is_none())
        .map(|k| k.to_string())
        .collect();
    if !missing.is_empty() {
        return Err(model(format!(
            "metadata: missing required keys {}",
            list_repr(&missing)
        )));
    }
    Ok(())
}

pub fn validate_counts(meta: &Meta, catalogue: &[Row]) -> Result<(), Fail> {
    let n = catalogue.len().to_string();
    for field in ["tpb_guidance_statement_count", "tpb_library_index_count"] {
        let value = meta_get(meta, field);
        if value != Some(n.as_str()) {
            let shown = value.map_or_else(|| "None".to_string(), py::repr_str);
            return Err(model(format!(
                "metadata {field}={shown} but catalogue has {n} rows"
            )));
        }
    }
    Ok(())
}
