//! JSON read and written as the original builder's `json` module did.
//!
//! Records and responses are read with Python's C scanner semantics, so a
//! refusal names the same position, and a repeated key is refused as the
//! builder's `object_pairs_hook` refused it. Output matches
//! `json.dumps(..., indent=2, ensure_ascii=False)`.

use num_bigint::BigInt;

use crate::py;

/// A decoded JSON value, keeping object key order.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(BigInt),
    Float(f64),
    Str(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn str_obj(value: &str) -> Json {
        Json::Str(value.to_string())
    }

    /// `repr(value)` of the Python object.
    pub fn repr(&self) -> String {
        match self {
            Json::Null => "None".into(),
            Json::Bool(true) => "True".into(),
            Json::Bool(false) => "False".into(),
            Json::Int(i) => py::int_repr(i).unwrap_or_else(|e| e),
            Json::Float(f) => py::float_repr(*f),
            Json::Str(s) => py::repr_str(s),
            Json::Array(items) => format!(
                "[{}]",
                items.iter().map(Json::repr).collect::<Vec<_>>().join(", ")
            ),
            Json::Object(pairs) => format!(
                "{{{}}}",
                pairs
                    .iter()
                    .map(|(k, v)| format!("{}: {}", py::repr_str(k), v.repr()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    /// `str(value)` of the Python object.
    pub fn py_str(&self) -> String {
        match self {
            Json::Str(s) => s.clone(),
            other => other.repr(),
        }
    }

    /// Python truthiness.
    pub fn truthy(&self) -> bool {
        match self {
            Json::Null => false,
            Json::Bool(b) => *b,
            Json::Int(i) => i != &BigInt::from(0),
            Json::Float(f) => *f != 0.0,
            Json::Str(s) => !s.is_empty(),
            Json::Array(items) => !items.is_empty(),
            Json::Object(pairs) => !pairs.is_empty(),
        }
    }
}

/// Why a JSON text could not be read, rendered as Python's exception.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodeError(pub String);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn decode_error(msg: &str, doc: &[char], pos: usize) -> DecodeError {
    let lineno = doc[..pos].iter().filter(|c| **c == '\n').count() + 1;
    let colno = match doc[..pos].iter().rposition(|c| *c == '\n') {
        Some(i) => pos - i,
        None => pos + 1,
    };
    DecodeError(format!("{msg}: line {lineno} column {colno} (char {pos})"))
}

fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

enum Frame {
    Array(Vec<Json>),
    Object(Vec<(String, Json)>, String),
}

/// `json.loads(text, object_pairs_hook=_reject_duplicate_keys)`.
pub fn loads(text: &str) -> Result<Json, DecodeError> {
    let s: Vec<char> = text.chars().collect();
    if s.first() == Some(&'\u{feff}') {
        return Err(decode_error(
            "Unexpected UTF-8 BOM (decode using utf-8-sig)",
            &s,
            0,
        ));
    }
    let len = s.len();
    let skip = |mut i: usize| {
        while i < len && is_ws(s[i]) {
            i += 1;
        }
        i
    };
    let mut idx = skip(0);
    let mut stack: Vec<Frame> = Vec::new();
    let expecting_value = |pos: usize| decode_error("Expecting value", &s, pos);

    // Reads a key and its delimiter, leaving idx at the value.
    let read_key = |idx: &mut usize| -> Result<String, DecodeError> {
        if *idx >= len || s[*idx] != '"' {
            return Err(decode_error(
                "Expecting property name enclosed in double quotes",
                &s,
                *idx,
            ));
        }
        let (key, next) = scanstring(&s, *idx + 1)?;
        *idx = skip(next);
        if *idx >= len || s[*idx] != ':' {
            return Err(decode_error("Expecting ':' delimiter", &s, *idx));
        }
        *idx = skip(*idx + 1);
        Ok(key)
    };

    'value: loop {
        // Scan one value starting at idx, or open a container.
        if idx >= len {
            return Err(expecting_value(idx));
        }
        let mut value = match s[idx] {
            '"' => {
                let (string, next) = scanstring(&s, idx + 1)?;
                idx = next;
                Json::Str(string)
            }
            '{' => {
                idx = skip(idx + 1);
                if idx < len && s[idx] == '}' {
                    idx += 1;
                    Json::Object(Vec::new())
                } else {
                    let key = read_key(&mut idx)?;
                    stack.push(Frame::Object(Vec::new(), key));
                    continue 'value;
                }
            }
            '[' => {
                idx = skip(idx + 1);
                if idx < len && s[idx] == ']' {
                    idx += 1;
                    Json::Array(Vec::new())
                } else {
                    stack.push(Frame::Array(Vec::new()));
                    continue 'value;
                }
            }
            _ => {
                let literal = |word: &str| {
                    s[idx..]
                        .iter()
                        .take(word.chars().count())
                        .copied()
                        .eq(word.chars())
                };
                if literal("null") {
                    idx += 4;
                    Json::Null
                } else if literal("true") {
                    idx += 4;
                    Json::Bool(true)
                } else if literal("false") {
                    idx += 5;
                    Json::Bool(false)
                } else if literal("NaN") {
                    idx += 3;
                    Json::Float(f64::NAN)
                } else if literal("Infinity") {
                    idx += 8;
                    Json::Float(f64::INFINITY)
                } else if literal("-Infinity") {
                    idx += 9;
                    Json::Float(f64::NEG_INFINITY)
                } else {
                    let (number, next) =
                        match_number(&s, idx).ok_or_else(|| expecting_value(idx))??;
                    idx = next;
                    number
                }
            }
        };
        // Attach the finished value to its container, closing any that end here.
        loop {
            let Some(frame) = stack.last_mut() else {
                let end = skip(idx);
                if end != len {
                    return Err(decode_error("Extra data", &s, end));
                }
                return Ok(value);
            };
            match frame {
                Frame::Array(items) => {
                    items.push(value);
                    idx = skip(idx);
                    if idx < len && s[idx] == ']' {
                        idx += 1;
                        let Some(Frame::Array(items)) = stack.pop() else {
                            unreachable!()
                        };
                        value = Json::Array(items);
                        continue;
                    }
                    if idx >= len || s[idx] != ',' {
                        return Err(decode_error("Expecting ',' delimiter", &s, idx));
                    }
                    let comma = idx;
                    idx = skip(idx + 1);
                    if idx < len && s[idx] == ']' {
                        return Err(decode_error(
                            "Illegal trailing comma before end of array",
                            &s,
                            comma,
                        ));
                    }
                    continue 'value;
                }
                Frame::Object(pairs, key) => {
                    pairs.push((std::mem::take(key), value));
                    idx = skip(idx);
                    if idx < len && s[idx] == '}' {
                        idx += 1;
                        let Some(Frame::Object(pairs, _)) = stack.pop() else {
                            unreachable!()
                        };
                        // The builder's object_pairs_hook.
                        for (i, (k, _)) in pairs.iter().enumerate() {
                            if pairs[..i].iter().any(|(earlier, _)| earlier == k) {
                                return Err(DecodeError(format!(
                                    "duplicate key {}",
                                    py::repr_str(k)
                                )));
                            }
                        }
                        value = Json::Object(pairs);
                        continue;
                    }
                    if idx >= len || s[idx] != ',' {
                        return Err(decode_error("Expecting ',' delimiter", &s, idx));
                    }
                    let comma = idx;
                    idx = skip(idx + 1);
                    if idx < len && s[idx] == '}' {
                        return Err(decode_error(
                            "Illegal trailing comma before end of object",
                            &s,
                            comma,
                        ));
                    }
                    let next_key = read_key(&mut idx)?;
                    if let Some(Frame::Object(_, key)) = stack.last_mut() {
                        *key = next_key;
                    }
                    continue 'value;
                }
            }
        }
    }
}

/// The C scanner's number match; None where it raises StopIteration.
fn match_number(s: &[char], start: usize) -> Option<Result<(Json, usize), DecodeError>> {
    let len = s.len();
    let digit = |i: usize| i < len && s[i].is_ascii_digit();
    let mut idx = start;
    if s[idx] == '-' {
        idx += 1;
        if idx >= len {
            return None;
        }
    }
    if ('1'..='9').contains(&s[idx]) {
        idx += 1;
        while digit(idx) {
            idx += 1;
        }
    } else if s[idx] == '0' {
        idx += 1;
    } else {
        return None;
    }
    let mut is_float = false;
    if idx + 1 < len && s[idx] == '.' && s[idx + 1].is_ascii_digit() {
        is_float = true;
        idx += 2;
        while digit(idx) {
            idx += 1;
        }
    }
    if idx + 1 < len && (s[idx] == 'e' || s[idx] == 'E') {
        let e_start = idx;
        idx += 1;
        if idx + 1 < len && (s[idx] == '-' || s[idx] == '+') {
            idx += 1;
        }
        while digit(idx) {
            idx += 1;
        }
        if s[idx - 1].is_ascii_digit() {
            is_float = true;
        } else {
            idx = e_start;
        }
    }
    let text: String = s[start..idx].iter().collect();
    if is_float {
        let value: f64 = text.parse().expect("a JSON float literal");
        Some(Ok((Json::Float(value), idx)))
    } else {
        Some(
            py::parse_int(&text, 10)
                .map(|i| (Json::Int(i), idx))
                .map_err(DecodeError),
        )
    }
}

/// The C `scanstring`: `end` is the index after the opening quote.
fn scanstring(s: &[char], end: usize) -> Result<(String, usize), DecodeError> {
    let len = s.len();
    let begin = end - 1;
    let mut out = String::new();
    let mut end = end;
    loop {
        let mut next = end;
        let mut c = '\0';
        while next < len {
            c = s[next];
            if c == '"' || c == '\\' {
                break;
            }
            if (c as u32) <= 0x1f {
                return Err(decode_error("Invalid control character at", s, next));
            }
            next += 1;
        }
        if next >= len || c != '"' && c != '\\' {
            return Err(decode_error("Unterminated string starting at", s, begin));
        }
        out.extend(&s[end..next]);
        next += 1;
        if c == '"' {
            return Ok((out, next));
        }
        if next == len {
            return Err(decode_error("Unterminated string starting at", s, begin));
        }
        let c = s[next];
        if c != 'u' {
            end = next + 1;
            let replacement = match c {
                '"' => '"',
                '\\' => '\\',
                '/' => '/',
                'b' => '\x08',
                'f' => '\x0c',
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                _ => return Err(decode_error("Invalid \\escape", s, end - 2)),
            };
            out.push(replacement);
            continue;
        }
        next += 1;
        end = next + 4;
        if end >= len {
            return Err(decode_error("Invalid \\uXXXX escape", s, next - 1));
        }
        let hex = |from: usize, end: usize| -> Result<u32, DecodeError> {
            let mut value = 0;
            for &digit in &s[from..from + 4] {
                value = value << 4
                    | digit
                        .to_digit(16)
                        .ok_or_else(|| decode_error("Invalid \\uXXXX escape", s, end - 5))?;
            }
            Ok(value)
        };
        let mut code = hex(next, end)?;
        if (0xd800..=0xdbff).contains(&code) && end + 6 < len && s[end] == '\\' && s[end + 1] == 'u'
        {
            let low = hex(end + 2, end + 6)?;
            if (0xdc00..=0xdfff).contains(&low) {
                code = 0x10000 + (((code - 0xd800) << 10) | (low - 0xdc00));
                end += 6;
            }
        }
        match char::from_u32(code) {
            Some(ch) => out.push(ch),
            // shortcut: Python keeps a lone surrogate in the string, and the
            // builder could never write it out; it is refused at the escape.
            // Use a surrogate-preserving string type if a real input carries one.
            None => {
                return Err(decode_error(
                    "Unpaired surrogate in \\uXXXX escape",
                    s,
                    next - 2,
                ));
            }
        }
    }
}

fn dump_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            _ if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            _ => out.push(c),
        }
    }
    out.push('"');
}

fn dump_into(value: &Json, level: usize, sort_keys: bool, out: &mut String) {
    let pad = |n: usize| "  ".repeat(n);
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(true) => out.push_str("true"),
        Json::Bool(false) => out.push_str("false"),
        Json::Int(i) => out.push_str(&i.to_string()),
        Json::Float(f) if f.is_nan() => out.push_str("NaN"),
        Json::Float(f) if f.is_infinite() => {
            out.push_str(if *f > 0.0 { "Infinity" } else { "-Infinity" })
        }
        Json::Float(f) => out.push_str(&py::float_repr(*f)),
        Json::Str(s) => dump_str(s, out),
        Json::Array(items) if items.is_empty() => out.push_str("[]"),
        Json::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('\n');
                out.push_str(&pad(level + 1));
                dump_into(item, level + 1, sort_keys, out);
            }
            out.push('\n');
            out.push_str(&pad(level));
            out.push(']');
        }
        Json::Object(pairs) if pairs.is_empty() => out.push_str("{}"),
        Json::Object(pairs) => {
            let mut order: Vec<&(String, Json)> = pairs.iter().collect();
            if sort_keys {
                order.sort_by(|a, b| a.0.cmp(&b.0));
            }
            out.push('{');
            for (i, (k, v)) in order.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('\n');
                out.push_str(&pad(level + 1));
                dump_str(k, out);
                out.push_str(": ");
                dump_into(v, level + 1, sort_keys, out);
            }
            out.push('\n');
            out.push_str(&pad(level));
            out.push('}');
        }
    }
}

/// `json.dumps(value, indent=2, ensure_ascii=False, sort_keys=...)`.
pub fn dumps(value: &Json, sort_keys: bool) -> String {
    let mut out = String::new();
    dump_into(value, 0, sort_keys, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(text: &str) -> String {
        loads(text).unwrap_err().0
    }

    #[test]
    fn errors_match_the_c_scanner() {
        assert_eq!(
            err("{\"a\":1,}"),
            "Illegal trailing comma before end of object: line 1 column 7 (char 6)"
        );
        assert_eq!(
            err("[1,]"),
            "Illegal trailing comma before end of array: line 1 column 3 (char 2)"
        );
        assert_eq!(
            err("\"\u{1}\""),
            "Invalid control character at: line 1 column 2 (char 1)"
        );
        assert_eq!(err("\"\\q\""), "Invalid \\escape: line 1 column 2 (char 1)");
        assert_eq!(
            err("\"\\u12\""),
            "Invalid \\uXXXX escape: line 1 column 3 (char 2)"
        );
        assert_eq!(
            err("\"abc"),
            "Unterminated string starting at: line 1 column 1 (char 0)"
        );
        assert_eq!(
            err("{\"a\" 1}"),
            "Expecting ':' delimiter: line 1 column 6 (char 5)"
        );
        assert_eq!(err("1 2"), "Extra data: line 1 column 3 (char 2)");
        assert_eq!(err(""), "Expecting value: line 1 column 1 (char 0)");
        assert_eq!(
            err("\u{feff}{}"),
            "Unexpected UTF-8 BOM (decode using utf-8-sig): line 1 column 1 (char 0)"
        );
        assert_eq!(
            err("[1 2]"),
            "Expecting ',' delimiter: line 1 column 4 (char 3)"
        );
        assert_eq!(
            err("{1:2}"),
            "Expecting property name enclosed in double quotes: line 1 column 2 (char 1)"
        );
        assert_eq!(err("-"), "Expecting value: line 1 column 1 (char 0)");
        assert_eq!(err("01"), "Extra data: line 1 column 2 (char 1)");
        assert_eq!(
            err("\"\\"),
            "Unterminated string starting at: line 1 column 1 (char 0)"
        );
        assert_eq!(err("{\"a\":"), "Expecting value: line 1 column 6 (char 5)");
        assert_eq!(
            err("{\"a\":1"),
            "Expecting ',' delimiter: line 1 column 7 (char 6)"
        );
        assert_eq!(err("["), "Expecting value: line 1 column 2 (char 1)");
        assert_eq!(err("{\"a\":1,\"a\":2}"), "duplicate key 'a'");
    }

    #[test]
    fn dumps_matches_python() {
        let value = loads("{\"b\": [1, 2.5, \"\u{e9}\\n\\u0001\"], \"a\": {}, \"c\": []}").unwrap();
        assert_eq!(
            dumps(&value, false),
            "{\n  \"b\": [\n    1,\n    2.5,\n    \"\u{e9}\\n\\u0001\"\n  ],\n  \"a\": {},\n  \"c\": []\n}"
        );
        assert!(dumps(&value, true).starts_with("{\n  \"a\": {}"));
    }
}
