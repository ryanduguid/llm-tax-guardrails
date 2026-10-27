//! The Python behaviours the original builder's output depended on.
//!
//! The builder was a Python program, and its findings quote Python's own
//! renderings: `repr` of values, `str` of paths and `OSError`s, the UTF-8
//! decoder's messages and `str` predicates such as `isspace`. Reproducing
//! them here keeps every status line byte-for-byte what a maintainer saw.

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use num_bigint::BigInt;

use crate::pyunicode as tables;

fn in_table(cp: u32, table: &[(u32, u32)]) -> bool {
    table
        .binary_search_by(|&(lo, hi)| {
            if hi < cp {
                std::cmp::Ordering::Less
            } else if lo > cp {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// `str.isspace` for one character.
pub fn is_space(c: char) -> bool {
    in_table(c as u32, tables::SPACE)
}

/// `str.isprintable` for one character.
pub fn is_printable(c: char) -> bool {
    !in_table(c as u32, tables::NOT_PRINTABLE)
}

/// `str.isdecimal` for one character: what `\d` matches in a `str` pattern.
pub fn is_decimal(c: char) -> bool {
    in_table(c as u32, tables::DECIMAL)
}

/// The value `int()` gives a decimal digit, ASCII or not.
pub fn decimal_value(c: char) -> Option<u32> {
    if !is_decimal(c) {
        return None;
    }
    let cp = c as u32;
    let index = tables::DECIMAL_ZERO.partition_point(|&zero| zero <= cp);
    let zero = tables::DECIMAL_ZERO[index.checked_sub(1)?];
    (cp - zero < 10).then_some(cp - zero)
}

/// The ranges of a character class, without its brackets, for a `regex` pattern.
pub fn class_body(table: &[(u32, u32)]) -> String {
    let mut out = String::new();
    for &(lo, hi) in table {
        if lo == hi {
            let _ = write!(out, "\\x{{{lo:X}}}");
        } else {
            let _ = write!(out, "\\x{{{lo:X}}}-\\x{{{hi:X}}}");
        }
    }
    out
}

/// The class a `str` pattern's `\d`, `\s` or `\w` matches, as a `regex` class.
fn class(table: &[(u32, u32)]) -> String {
    format!("[{}]", class_body(table))
}

/// Python's `\d` for a `str` pattern.
pub static D: LazyLock<String> = LazyLock::new(|| class(tables::DECIMAL));
/// Python's `\s` for a `str` pattern.
pub static S: LazyLock<String> = LazyLock::new(|| class(tables::SPACE));
/// Python's `\w` for a `str` pattern.
pub static W: LazyLock<String> = LazyLock::new(|| class(tables::WORD));

/// Python's `\b`, for a `fancy_regex` pattern.
pub fn word_boundary() -> String {
    let w = &*W;
    format!("(?:(?<={w})(?!{w})|(?<!{w})(?={w}))")
}

/// What one ASCII letter matches in a Python pattern compiled with `re.IGNORECASE`.
pub fn ignorecase_letter(letter: char) -> String {
    let mut out = format!(
        "[{}{}",
        letter.to_ascii_lowercase(),
        letter.to_ascii_uppercase()
    );
    for (base, extra) in tables::IGNORECASE_EXTRA {
        if *base == letter.to_ascii_lowercase() {
            for cp in *extra {
                let _ = write!(out, "\\x{{{cp:X}}}");
            }
        }
    }
    out.push(']');
    out
}

/// A literal, matched as `re.IGNORECASE` would match it.
pub fn ignorecase_literal(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_alphabetic() {
                ignorecase_letter(c)
            } else {
                regex::escape(&c.to_string())
            }
        })
        .collect()
}

/// `str.strip()`.
pub fn strip(s: &str) -> &str {
    s.trim_matches(is_space)
}

/// `str.split()` with no separator.
pub fn split_whitespace(s: &str) -> Vec<&str> {
    s.split(is_space).filter(|part| !part.is_empty()).collect()
}

/// `str.splitlines()`.
pub fn splitlines(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let boundary = matches!(
            c,
            '\n' | '\r'
                | '\x0b'
                | '\x0c'
                | '\x1c'
                | '\x1d'
                | '\x1e'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        );
        if boundary {
            out.push(&s[start..i]);
            let mut next = i + c.len_utf8();
            if c == '\r'
                && let Some(&(j, '\n')) = chars.peek()
            {
                chars.next();
                next = j + 1;
            }
            start = next;
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

/// `str.casefold()`.
pub fn casefold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let cp = c as u32;
        match tables::CASEFOLD.binary_search_by_key(&cp, |&(k, _)| k) {
            Ok(i) => out.push_str(tables::CASEFOLD[i].1),
            Err(_) => out.push(c),
        }
    }
    out
}

/// The first `n` characters, as `s[:n]` takes them.
pub fn prefix_chars(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// `len(s)`: a count of code points.
pub fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// `repr(s)` for a `str`.
pub fn repr_str(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            _ if c == quote || c == '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ if (c as u32) < 0x20 || c as u32 == 0x7f => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            _ if (c as u32) < 0x7f => out.push(c),
            _ if is_printable(c) => out.push(c),
            _ if (c as u32) <= 0xff => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            _ if (c as u32) <= 0xffff => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            _ => {
                let _ = write!(out, "\\U{:08x}", c as u32);
            }
        }
    }
    out.push(quote);
    out
}

/// `repr(b)` for `bytes`.
pub fn repr_bytes(b: &[u8]) -> String {
    let quote = if b.contains(&b'\'') && !b.contains(&b'"') {
        b'"'
    } else {
        b'\''
    };
    let mut out = String::from("b");
    out.push(quote as char);
    for &byte in b {
        match byte {
            _ if byte == quote || byte == b'\\' => {
                out.push('\\');
                out.push(byte as char);
            }
            b'\t' => out.push_str("\\t"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            0x20..=0x7e => out.push(byte as char),
            _ => {
                let _ = write!(out, "\\x{byte:02x}");
            }
        }
    }
    out.push(quote as char);
    out
}

/// `format!("{value:e}")`, but where two spellings are equally short, the one
/// CPython's `repr` prints: the nearer to `value`, an exact tie going to the even
/// digit. Rust's shortest formatting can break that tie upwards: `1 + 2**-17` is
/// `1.0000076293945312` in Python, not `…313`. `{:.Ne}` rounds exactly with ties
/// to even, so its spelling is used whenever it still reads back as `value`.
fn shortest_exp(value: f64) -> String {
    let shortest = format!("{value:e}");
    let precision = shortest
        .bytes()
        .take_while(|&b| b != b'e')
        .filter(u8::is_ascii_digit)
        .count()
        - 1;
    let nearest = format!("{value:.precision$e}");
    if nearest.parse::<f64>() == Ok(value) {
        nearest
    } else {
        shortest
    }
}

/// `repr(f)` for a `float`.
pub fn float_repr(f: f64) -> String {
    if f.is_nan() {
        return "nan".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "inf".into() } else { "-inf".into() };
    }
    let sci = shortest_exp(f.abs());
    let (mantissa, exponent) = sci.split_once('e').expect("scientific notation");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let decpt: i64 = exponent.parse::<i64>().expect("exponent") + 1;
    let sign = if f.is_sign_negative() { "-" } else { "" };
    let n = digits.len() as i64;
    let body = if f == 0.0 {
        "0.0".to_string()
    } else if decpt <= -4 || decpt > 16 {
        let mut m = digits[..1].to_string();
        if n > 1 {
            m.push('.');
            m.push_str(&digits[1..]);
        }
        let e = decpt - 1;
        format!("{m}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
    } else if decpt <= 0 {
        format!("0.{}{digits}", "0".repeat((-decpt) as usize))
    } else if decpt >= n {
        format!("{digits}{}.0", "0".repeat((decpt - n) as usize))
    } else {
        format!(
            "{}.{}",
            &digits[..decpt as usize],
            &digits[decpt as usize..]
        )
    };
    format!("{sign}{body}")
}

/// Python refuses to render an `int` of more than this many decimal digits.
pub const INT_MAX_STR_DIGITS: usize = 4300;

/// The `ValueError` Python raises for an over-long decimal `int`.
pub fn int_digits_error(digits: usize) -> String {
    format!(
        "Exceeds the limit ({INT_MAX_STR_DIGITS} digits) for integer string conversion: value has \
         {digits} digits; use sys.set_int_max_str_digits() to increase the limit"
    )
}

/// `repr(i)` for an `int`, or the `ValueError` an over-long one raises.
pub fn int_repr(i: &BigInt) -> Result<String, String> {
    let text = i.to_string();
    let digits = text.trim_start_matches('-').len();
    if digits > INT_MAX_STR_DIGITS {
        // Python measures the limit before rendering; the count it reports is
        // its own estimate, so only the refusal is reproduced exactly.
        return Err(
            "Exceeds the limit (4300 digits) for integer string conversion; use \
                    sys.set_int_max_str_digits() to increase the limit"
                .into(),
        );
    }
    Ok(text)
}

/// Unicode digits to ASCII and Unicode spaces to ' ', as `int()` and `float()` first do.
fn number_text(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if is_space(c) {
            out.push(' ');
        } else if let Some(d) = decimal_value(c) {
            out.push(char::from_digit(d, 10).expect("digit"));
        } else if c.is_ascii() {
            out.push(c);
        } else {
            return None;
        }
    }
    Some(out)
}

/// `int(s, base)`; the error is the `ValueError` message.
pub fn parse_int(s: &str, base: u32) -> Result<BigInt, String> {
    // CPython formats the input with `%.200R`: the repr, cut to 200 characters.
    let invalid = || {
        format!(
            "invalid literal for int() with base {base}: {}",
            prefix_chars(&repr_str(s), 200)
        )
    };
    let text = number_text(s).ok_or_else(invalid)?;
    let text =
        text.trim_matches(|c: char| c.is_ascii_whitespace() || ('\x1c'..='\x1f').contains(&c));
    let (negative, mut body) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let prefix = match base {
        16 => Some(["0x", "0X"]),
        8 => Some(["0o", "0O"]),
        2 => Some(["0b", "0B"]),
        _ => None,
    };
    let mut after_prefix = false;
    if let Some(prefixes) = prefix {
        for p in prefixes {
            if let Some(rest) = body.strip_prefix(p) {
                body = rest;
                after_prefix = true;
                break;
            }
        }
    }
    // An underscore may follow a digit, or a base prefix, and must precede a digit.
    let mut digits = String::new();
    let mut previous_digit = false;
    let mut previous_underscore = false;
    for (i, c) in body.chars().enumerate() {
        if c == '_' {
            if previous_underscore || !(previous_digit || i == 0 && after_prefix) {
                return Err(invalid());
            }
            previous_underscore = true;
            previous_digit = false;
            continue;
        }
        if c.to_digit(base).is_none() {
            return Err(invalid());
        }
        digits.push(c);
        previous_digit = true;
        previous_underscore = false;
    }
    if digits.is_empty() || previous_underscore {
        return Err(invalid());
    }
    if base == 10 && digits.len() > INT_MAX_STR_DIGITS {
        return Err(int_digits_error(digits.len()));
    }
    let value = BigInt::parse_bytes(digits.as_bytes(), base).ok_or_else(invalid)?;
    Ok(if negative { -value } else { value })
}

/// `float(s)`; the error is the `ValueError` message.
pub fn parse_float(s: &str) -> Result<f64, String> {
    let invalid = || format!("could not convert string to float: {}", repr_str(s));
    let text = number_text(s).ok_or_else(invalid)?;
    let text =
        text.trim_matches(|c: char| c.is_ascii_whitespace() || ('\x1c'..='\x1f').contains(&c));
    let (sign, body) = match text.as_bytes().first() {
        Some(b'-') => ("-", &text[1..]),
        Some(b'+') => ("", &text[1..]),
        _ => ("", text),
    };
    let lower = body.to_ascii_lowercase();
    if matches!(lower.as_str(), "inf" | "infinity" | "nan") {
        return format!("{sign}{lower}")
            .parse::<f64>()
            .map_err(|_| invalid());
    }
    // Digits with single underscores between them, an optional fraction and
    // an optional exponent: the grammar float() accepts.
    let bytes = body.as_bytes();
    let mut i = 0;
    let mut clean = String::from(sign);
    let digit_run = |i: &mut usize, clean: &mut String| -> bool {
        let start = *i;
        while *i < bytes.len() {
            let b = bytes[*i];
            if b.is_ascii_digit() {
                clean.push(b as char);
                *i += 1;
            } else if b == b'_'
                && *i > start
                && bytes[*i - 1].is_ascii_digit()
                && *i + 1 < bytes.len()
                && bytes[*i + 1].is_ascii_digit()
            {
                *i += 1;
            } else {
                break;
            }
        }
        *i > start
    };
    let whole = digit_run(&mut i, &mut clean);
    let mut fraction = false;
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        clean.push('.');
        fraction = digit_run(&mut i, &mut clean);
        if !fraction {
            clean.push('0');
        }
    }
    if !whole && !fraction {
        return Err(invalid());
    }
    if !whole {
        clean.insert(sign.len(), '0');
    }
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        clean.push('e');
        if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
            clean.push(bytes[i] as char);
            i += 1;
        }
        if !digit_run(&mut i, &mut clean) {
            return Err(invalid());
        }
    }
    if i != bytes.len() {
        return Err(invalid());
    }
    clean.parse::<f64>().map_err(|_| invalid())
}

/// A UTF-8 decoding failure, rendered as Python's `UnicodeDecodeError`.
#[derive(Debug, Clone)]
pub struct DecodeError {
    pub start: usize,
    pub end: usize,
    pub byte: u8,
    pub reason: &'static str,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.end == self.start + 1 {
            write!(
                f,
                "'utf-8' codec can't decode byte 0x{:02x} in position {}: {}",
                self.byte, self.start, self.reason
            )
        } else {
            write!(
                f,
                "'utf-8' codec can't decode bytes in position {}-{}: {}",
                self.start,
                self.end - 1,
                self.reason
            )
        }
    }
}

/// Decode UTF-8 strictly, failing as `bytes.decode("utf-8")` does.
pub fn decode_utf8(bytes: &[u8]) -> Result<String, DecodeError> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text.to_string()),
        Err(e) => {
            let start = e.valid_up_to();
            let byte = bytes[start];
            let (end, reason) = match e.error_len() {
                None => (bytes.len(), "unexpected end of data"),
                Some(n) if (0xc2..=0xf4).contains(&byte) => {
                    (start + n, "invalid continuation byte")
                }
                Some(n) => (start + n, "invalid start byte"),
            };
            Err(DecodeError {
                start,
                end,
                byte,
                reason,
            })
        }
    }
}

/// Text mode's universal newlines: `\r\n` and `\r` both read as `\n`.
pub fn universal_newlines(text: &str) -> String {
    if text.contains('\r') {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text.to_string()
    }
}

/// Why a text read failed.
#[derive(Debug)]
pub enum ReadError {
    Os(io::Error),
    Decode(DecodeError),
}

/// `Path.read_text(encoding="utf-8")`.
pub fn read_text(path: &Path) -> Result<String, ReadError> {
    let bytes = std::fs::read(path).map_err(ReadError::Os)?;
    let text = decode_utf8(&bytes).map_err(ReadError::Decode)?;
    Ok(universal_newlines(&text))
}

/// How the platform reported an `OSError`, which decides how Python renders it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OsStyle {
    /// `open()` and friends: the C runtime's errno, on every platform.
    Errno,
    /// Directory and metadata calls, which on Windows report a Win32 error.
    Native,
}

fn strip_os_suffix(message: &str) -> &str {
    match message.rfind(" (os error ") {
        Some(i) => &message[..i],
        None => message,
    }
}

#[cfg(windows)]
fn crt_errno(winerror: i32) -> (i32, &'static str) {
    match winerror {
        2 | 3 | 15 | 206 => (2, "No such file or directory"),
        4 => (24, "Too many open files"),
        5 | 32 | 33 | 1920 => (13, "Permission denied"),
        6 => (9, "Bad file descriptor"),
        8 => (12, "Not enough space"),
        80 | 183 => (17, "File exists"),
        _ => (22, "Invalid argument"),
    }
}

/// `str(OSError)` for an operation on `filename` (already rendered as `str(path)`).
pub fn oserror_str(error: &io::Error, filename: &str, style: OsStyle) -> String {
    let Some(code) = error.raw_os_error() else {
        return error.to_string();
    };
    let message = error.to_string();
    #[cfg(windows)]
    {
        if style == OsStyle::Errno {
            let (errno, text) = crt_errno(code);
            return format!("[Errno {errno}] {text}: {}", repr_str(filename));
        }
        let text = strip_os_suffix(&message).trim_end_matches(|c: char| c == '.' || c <= ' ');
        format!("[WinError {code}] {text}: {}", repr_str(filename))
    }
    #[cfg(not(windows))]
    {
        let _ = style;
        format!(
            "[Errno {code}] {}: {}",
            strip_os_suffix(&message),
            repr_str(filename)
        )
    }
}

/// A path as `pathlib` holds and prints it.
///
/// `pathlib` drops `.` components and repeated separators and prints with
/// the platform separator; a finding that names a file shows that form, so
/// paths are carried in it alongside the real filesystem path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PyPath {
    anchor: String,
    parts: Vec<String>,
}

#[cfg(windows)]
const SEP: &str = "\\";
#[cfg(not(windows))]
const SEP: &str = "/";

fn is_sep(c: char) -> bool {
    c == '/' || cfg!(windows) && c == '\\'
}

impl PyPath {
    pub fn new(text: &str) -> Self {
        let (anchor, rest) = Self::split_anchor(text);
        let mut path = PyPath {
            anchor,
            parts: Vec::new(),
        };
        path.push_parts(rest);
        path
    }

    #[cfg(windows)]
    fn split_anchor(text: &str) -> (String, &str) {
        let bytes = text.as_bytes();
        let sep = |i: usize| bytes.get(i).is_some_and(|b| *b == b'/' || *b == b'\\');
        if sep(0) && sep(1) {
            // \\server\share\ : a UNC anchor.
            let rest = &text[2..];
            let mut pieces = rest.splitn(3, ['/', '\\']);
            let server = pieces.next().unwrap_or("");
            let share = pieces.next().unwrap_or("");
            let tail = pieces.next().unwrap_or("");
            if !server.is_empty() && !share.is_empty() {
                return (format!("\\\\{server}\\{share}\\"), tail);
            }
            return ("\\".into(), rest);
        }
        if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
            let drive = &text[..2];
            if sep(2) {
                return (format!("{drive}\\"), &text[3..]);
            }
            return (drive.to_string(), &text[2..]);
        }
        if sep(0) {
            return ("\\".into(), &text[1..]);
        }
        (String::new(), text)
    }

    #[cfg(not(windows))]
    fn split_anchor(text: &str) -> (String, &str) {
        if text.starts_with("//") && !text.starts_with("///") {
            ("//".into(), &text[2..])
        } else if text.starts_with('/') {
            ("/".into(), text.trim_start_matches('/'))
        } else {
            (String::new(), text)
        }
    }

    fn push_parts(&mut self, rest: &str) {
        for part in rest.split(is_sep) {
            if !part.is_empty() && part != "." {
                self.parts.push(part.to_string());
            }
        }
    }

    /// `self / part`, where `part` is a relative path that may hold separators.
    pub fn join(&self, part: &str) -> Self {
        let other = PyPath::new(part);
        if !other.anchor.is_empty() {
            return other;
        }
        let mut out = self.clone();
        out.parts.extend(other.parts);
        out
    }

    /// `str(path)`.
    pub fn display(&self) -> String {
        if self.anchor.is_empty() && self.parts.is_empty() {
            return ".".into();
        }
        format!("{}{}", self.anchor, self.parts.join(SEP))
    }

    /// The filesystem path to operate on.
    pub fn fs(&self) -> PathBuf {
        PathBuf::from(self.display())
    }

    /// `path.name`.
    pub fn name(&self) -> &str {
        self.parts.last().map_or("", String::as_str)
    }

    /// `path.parent`.
    pub fn parent(&self) -> Self {
        let mut out = self.clone();
        out.parts.pop();
        out
    }

    #[cfg(windows)]
    pub fn is_absolute(&self) -> bool {
        self.anchor.len() > 2 || self.anchor.starts_with("\\\\")
    }

    #[cfg(not(windows))]
    pub fn is_absolute(&self) -> bool {
        !self.anchor.is_empty()
    }
}

impl std::fmt::Display for PyPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.display())
    }
}

/// `Path.resolve()`, without Windows' verbatim `\\?\` prefix.
pub fn resolve(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    // Non-strict, as CPython: resolve the deepest existing ancestor and keep
    // the missing rest as written, so a link or Windows short name above a
    // missing file still resolves.
    let mut existing = absolute.as_path();
    let mut missing = Vec::new();
    let resolved = loop {
        if let Ok(real) = std::fs::canonicalize(existing) {
            break missing.iter().rev().fold(real, |acc, part| acc.join(part));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name);
                existing = parent;
            }
            _ => break absolute.clone(),
        }
    };
    without_verbatim_prefix(&resolved.to_string_lossy()).unwrap_or(resolved)
}

/// `text` without Windows' verbatim `\\?\` prefix, or None when it has none.
fn without_verbatim_prefix(text: &str) -> Option<PathBuf> {
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        Some(PathBuf::from(format!(r"\\{rest}")))
    } else {
        text.strip_prefix(r"\\?\").map(PathBuf::from)
    }
}

/// A `datetime.date`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

pub const MAX_DATE: Date = Date {
    year: 9999,
    month: 12,
    day: 31,
};

fn is_leap(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(year) => 29,
        _ => 28,
    }
}

fn days_before_year(year: i32) -> i64 {
    let y = i64::from(year) - 1;
    y * 365 + y / 4 - y / 100 + y / 400
}

fn days_before_month(year: i32, month: u32) -> i64 {
    (1..month).map(|m| i64::from(days_in_month(year, m))).sum()
}

impl Date {
    /// `datetime.date(year, month, day)`; the error is the `ValueError` message.
    pub fn new(year: i64, month: i64, day: i64) -> Result<Date, String> {
        if !(1..=9999).contains(&year) {
            return Err(format!("year must be in 1..9999, not {year}"));
        }
        if !(1..=12).contains(&month) {
            return Err("month must be in 1..12".into());
        }
        let dim = i64::from(days_in_month(year as i32, month as u32));
        if !(1..=dim).contains(&day) {
            return Err(format!(
                "day {day} must be in range 1..{dim} for month {month} in year {year}"
            ));
        }
        Ok(Date {
            year: year as i32,
            month: month as u32,
            day: day as u32,
        })
    }

    /// The proleptic Gregorian ordinal, 1 for 0001-01-01.
    pub fn ordinal(&self) -> i64 {
        days_before_year(self.year) + days_before_month(self.year, self.month) + i64::from(self.day)
    }

    pub fn from_ordinal(ordinal: i64) -> Option<Date> {
        if !(1..=MAX_DATE.ordinal()).contains(&ordinal) {
            return None;
        }
        // Estimate the year, then correct it upwards: the estimate is never
        // too late (every_ordinal_round_trips checks the whole range).
        let mut year = ((ordinal - 1) * 400 / 146_097 + 1) as i32;
        debug_assert!(days_before_year(year) < ordinal);
        while days_before_year(year + 1) < ordinal {
            year += 1;
        }
        let mut rest = ordinal - days_before_year(year);
        let mut month = 1;
        while rest > i64::from(days_in_month(year, month)) {
            rest -= i64::from(days_in_month(year, month));
            month += 1;
        }
        Some(Date {
            year,
            month,
            day: rest as u32,
        })
    }

    /// `date + timedelta(days=n)`, or None where Python raises OverflowError.
    pub fn add_days(&self, days: i64) -> Option<Date> {
        Date::from_ordinal(self.ordinal() + days)
    }

    /// `date.isoformat()`.
    pub fn isoformat(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// `date.fromisoformat(s)`, following CPython's C implementation.
    pub fn fromisoformat(s: &str) -> Option<Date> {
        let b = s.as_bytes();
        if !matches!(b.len(), 7 | 8 | 10) {
            return None;
        }
        // The C parser reads a NUL-terminated buffer.
        let at = |i: usize| b.get(i).copied().unwrap_or(0);
        let digits = |p: &mut usize, n: usize| -> Option<i64> {
            let mut value = 0i64;
            for _ in 0..n {
                let c = at(*p);
                if !c.is_ascii_digit() {
                    return None;
                }
                value = value * 10 + i64::from(c - b'0');
                *p += 1;
            }
            Some(value)
        };
        let mut p = 0;
        let year = digits(&mut p, 4)?;
        let separator = at(p) == b'-';
        if separator {
            p += 1;
        }
        if at(p) == b'W' {
            p += 1;
            let week = digits(&mut p, 2)?;
            let weekday = if p < b.len() {
                if separator {
                    if at(p) != b'-' {
                        return None;
                    }
                    p += 1;
                }
                digits(&mut p, 1)?
            } else {
                1
            };
            return Date::from_iso_week(year, week, weekday);
        }
        let month = digits(&mut p, 2)?;
        if separator {
            if at(p) != b'-' {
                return None;
            }
            p += 1;
        }
        let day = digits(&mut p, 2)?;
        Date::new(year, month, day).ok()
    }

    fn from_iso_week(year: i64, week: i64, weekday: i64) -> Option<Date> {
        if !(1..=9999).contains(&year) {
            return None;
        }
        let year = year as i32;
        let first = Date {
            year,
            month: 1,
            day: 1,
        }
        .ordinal();
        let first_weekday = (first + 6) % 7;
        if week <= 0 || week >= 53 {
            let long_year = first_weekday == 3 || first_weekday == 2 && is_leap(year);
            if !(week == 53 && long_year) {
                return None;
            }
        }
        if !(1..=7).contains(&weekday) {
            return None;
        }
        let mut monday = first - first_weekday;
        if first_weekday > 3 {
            monday += 7;
        }
        Date::from_ordinal(monday + (week - 1) * 7 + weekday - 1)
    }

    /// `date.today()`, in the machine's local time zone.
    pub fn today() -> Date {
        use chrono::Datelike;
        let now = chrono::Local::now().date_naive();
        Date {
            year: now.year(),
            month: now.month(),
            day: now.day(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repr_matches_python() {
        assert_eq!(repr_str("a'b"), "\"a'b\"");
        assert_eq!(repr_str("a'\"b"), "'a\\'\"b'");
        assert_eq!(
            repr_str("\t\u{0}\u{7f}é\u{a0}\u{2028}\u{1F600}"),
            "'\\t\\x00\\x7fé\\xa0\\u2028\u{1F600}'"
        );
        assert_eq!(repr_bytes(b"a'\x00\xff"), "b\"a'\\x00\\xff\"");
    }

    #[test]
    fn float_repr_matches_python() {
        for (value, expected) in [
            (1e16, "1e+16"),
            (1e15, "1000000000000000.0"),
            (1e-5, "1e-05"),
            (0.0001, "0.0001"),
            (-0.0, "-0.0"),
            (1.5e300, "1.5e+300"),
            (123456789.123, "123456789.123"),
            (5e-324, "5e-324"),
            (1.0, "1.0"),
        ] {
            assert_eq!(float_repr(value), expected);
        }
    }

    #[test]
    fn fromisoformat_follows_cpython() {
        let ok = |s: &str| Date::fromisoformat(s).map(|d| d.isoformat());
        assert_eq!(ok("20261017ab").as_deref(), Some("2026-10-17"));
        assert_eq!(ok("2026W011xx").as_deref(), Some("2025-12-29"));
        assert_eq!(ok("2026-W01").as_deref(), Some("2025-12-29"));
        assert_eq!(ok("20261017\u{e9}").as_deref(), Some("2026-10-17"));
        assert_eq!(ok("2026-W53").as_deref(), Some("2026-12-28"));
        assert_eq!(ok("2020-W53-7").as_deref(), Some("2021-01-03"));
        for bad in [
            "2026-1017",
            "202610-17",
            "2026-10-1x",
            "0000-01-01",
            "9999-W52-6",
            "+202-10-17",
            " 2026-1-1",
            "2026-W01-1x",
            "2026W01-1",
            "2026-02-30",
        ] {
            assert_eq!(ok(bad), None, "{bad}");
        }
    }

    #[test]
    fn decode_errors_match_python() {
        let e = decode_utf8(b"ab\xff").unwrap_err();
        assert_eq!(
            e.to_string(),
            "'utf-8' codec can't decode byte 0xff in position 2: invalid start byte"
        );
        let e = decode_utf8(b"\xe2\x82(").unwrap_err();
        assert_eq!(
            e.to_string(),
            "'utf-8' codec can't decode bytes in position 0-1: invalid continuation byte"
        );
        let e = decode_utf8(b"a\xe2\x82").unwrap_err();
        assert_eq!(
            e.to_string(),
            "'utf-8' codec can't decode bytes in position 1-2: unexpected end of data"
        );
    }

    #[test]
    fn string_predicates_match_python() {
        assert_eq!(splitlines("a\r\nb\rc\x1cd\n"), vec!["a", "b", "c", "d"]);
        assert_eq!(strip("\x1c x \u{3000}"), "x");
        assert_eq!(casefold("Straße ΣΑΣ"), "strasse σασ");
        assert_eq!(decimal_value('\u{0663}'), Some(3));
        assert_eq!(parse_int(" 1\u{0661}", 10).unwrap(), BigInt::from(11));
        assert_eq!(parse_int("0b101", 2).unwrap(), BigInt::from(5));
        assert!(parse_int("1__0", 10).is_err());
        assert_eq!(parse_float("1.e+5").unwrap(), 100000.0);
        assert_eq!(parse_float("1_0.5").unwrap(), 10.5);
        assert!(parse_float("1._5").is_err());
    }

    #[test]
    fn pyath_prints_like_pathlib() {
        assert_eq!(
            PyPath::new(".").join("src/data").display(),
            format!("src{SEP}data")
        );
        assert_eq!(PyPath::new("./a//b/.").display(), format!("a{SEP}b"));
        assert_eq!(PyPath::new("").display(), ".");
    }

    #[test]
    fn float_repr_matches_cpython_corpus() {
        // CPython 3.15's repr for doubles chosen to exercise ties between two
        // equally short spellings: bits in hex, then the repr.
        assert_eq!(float_repr(1.0 + 2f64.powi(-17)), "1.0000076293945312");
        let wrong: Vec<String> = include_str!("../tests/data/float_repr.txt")
            .lines()
            .filter_map(|line| {
                let (bits, expected) = line.split_once(' ').expect("bits and repr");
                let actual = float_repr(f64::from_bits(u64::from_str_radix(bits, 16).unwrap()));
                (actual != expected).then(|| format!("{bits}: python {expected}, rust {actual}"))
            })
            .collect();
        let first = &wrong[..wrong.len().min(10)];
        assert!(wrong.is_empty(), "{} wrong: {first:?}", wrong.len());
    }

    #[test]
    fn float_repr_keeps_the_shortest_spelling_when_the_nearest_misreads() {
        // At a power of two the gap below is half the gap above, so the
        // correctly rounded 16-digit spelling can fall outside it; CPython
        // then prints the shortest spelling that reads back.
        assert_eq!(float_repr(2f64.powi(-1017)), "7.120236347223045e-307");
        assert_eq!(float_repr(2f64.powi(-957)), "8.209073602596753e-289");
    }

    #[test]
    fn repr_escapes_match_python() {
        assert_eq!(repr_str("\u{e0001}"), "'\\U000e0001'");
        assert_eq!(repr_str("\u{10ffff}"), "'\\U0010ffff'");
        assert_eq!(
            repr_bytes(b"'\\\t\n\r\x00\x7f"),
            "b\"'\\\\\\t\\n\\r\\x00\\x7f\""
        );
        assert_eq!(repr_bytes(b"\"'"), "b'\"\\''");
        assert_eq!(repr_bytes(b"\"x\""), "b'\"x\"'");
    }

    #[test]
    fn int_repr_refuses_more_than_4300_digits() {
        let limit = BigInt::from(10).pow(4299);
        assert_eq!(int_repr(&limit).unwrap().len(), 4300);
        assert_eq!(
            int_repr(&(limit * 10)).unwrap_err(),
            "Exceeds the limit (4300 digits) for integer string conversion; use \
             sys.set_int_max_str_digits() to increase the limit"
        );
    }

    #[test]
    fn int_and_float_parse_as_python() {
        assert_eq!(parse_int("+5", 10).unwrap(), BigInt::from(5));
        assert_eq!(parse_int("-5", 10).unwrap(), BigInt::from(-5));
        assert_eq!(parse_int("0x_1f", 16).unwrap(), BigInt::from(31));
        assert_eq!(
            parse_int("\u{bd}", 10).unwrap_err(),
            "invalid literal for int() with base 10: '\u{bd}'"
        );
        // %.200R cuts the repr, so its closing quote goes from 199 characters.
        let prefix = "invalid literal for int() with base 10: ";
        let at = |n: usize| parse_int(&"x".repeat(n), 10).unwrap_err();
        assert_eq!(at(198), format!("{prefix}'{}'", "x".repeat(198)));
        assert_eq!(at(199), format!("{prefix}'{}", "x".repeat(199)));
        assert_eq!(at(201), format!("{prefix}'{}", "x".repeat(199)));
        assert_eq!(parse_float("-inf").unwrap(), f64::NEG_INFINITY);
        assert_eq!(parse_float("+iNf").unwrap(), f64::INFINITY);
        assert_eq!(parse_float("-Infinity").unwrap(), f64::NEG_INFINITY);
        assert!(parse_float("+nan").unwrap().is_nan());
        assert_eq!(parse_float("-1.5").unwrap(), -1.5);
        assert_eq!(parse_float("+.5").unwrap(), 0.5);
        assert_eq!(parse_float("\u{661}.5").unwrap(), 1.5);
        for bad in ["1e", "1e+", "1e5x", "-", "--1", "x", "\u{bd}"] {
            assert_eq!(
                parse_float(bad).unwrap_err(),
                format!("could not convert string to float: {}", repr_str(bad)),
                "{bad}"
            );
        }
    }

    #[test]
    fn iso_week_dates_follow_cpython() {
        let ok = |s: &str| Date::fromisoformat(s).map(|d| d.isoformat());
        assert_eq!(ok("2026W01").as_deref(), Some("2025-12-29"));
        assert_eq!(ok("2015-W53-7").as_deref(), Some("2016-01-03"));
        for bad in [
            "2026-W01x1",
            "2026-01x01",
            "0000-W01-1",
            "2026-W01-8",
            "2026-W01-0",
            "2021-W53-1",
            "2026-W00-1",
        ] {
            assert_eq!(ok(bad), None, "{bad}");
        }
    }

    #[test]
    fn every_ordinal_round_trips() {
        // date.fromordinal and toordinal over the whole range, 0001-01-01 to 9999-12-31.
        let mut previous = Date::from_ordinal(1).unwrap();
        assert_eq!(previous.isoformat(), "0001-01-01");
        for ordinal in 2..=MAX_DATE.ordinal() {
            let date = Date::from_ordinal(ordinal).unwrap();
            assert_eq!(date.ordinal(), ordinal);
            assert!(date > previous, "{ordinal}");
            previous = date;
        }
        assert_eq!(previous, MAX_DATE);
        assert_eq!(Date::from_ordinal(0), None);
        assert_eq!(Date::from_ordinal(MAX_DATE.ordinal() + 1), None);
    }

    #[test]
    fn oserror_without_an_os_code_is_its_message() {
        let error = io::Error::other("no code here");
        assert_eq!(oserror_str(&error, "x", OsStyle::Errno), "no code here");
        assert_eq!(strip_os_suffix("Access is denied"), "Access is denied");
        assert_eq!(strip_os_suffix("Gone (os error 2)"), "Gone");
    }

    #[test]
    fn joining_an_absolute_path_replaces_the_base() {
        let base = PyPath::new("a");
        let absolute = PyPath::new(&std::env::temp_dir().to_string_lossy());
        assert!(absolute.is_absolute());
        assert!(!base.is_absolute());
        assert_eq!(base.join(&absolute.display()), absolute);
    }

    #[cfg(not(windows))]
    #[test]
    fn two_leading_slashes_stay_an_anchor_on_posix() {
        // PurePosixPath keeps exactly two leading slashes; three collapse to one.
        assert_eq!(PyPath::new("//x/y").display(), "//x/y");
        assert_eq!(PyPath::new("///x").display(), "/x");
    }

    #[cfg(windows)]
    #[test]
    fn windows_errors_print_as_the_c_runtime_errno() {
        // CPython 3.15.0's winerror-to-errno mapping and os.strerror.
        for (winerror, expected) in [
            (2, "[Errno 2] No such file or directory: 'x'"),
            (4, "[Errno 24] Too many open files: 'x'"),
            (5, "[Errno 13] Permission denied: 'x'"),
            (32, "[Errno 13] Permission denied: 'x'"),
            (6, "[Errno 9] Bad file descriptor: 'x'"),
            (8, "[Errno 12] Not enough space: 'x'"),
            (80, "[Errno 17] File exists: 'x'"),
            (183, "[Errno 17] File exists: 'x'"),
            (87, "[Errno 22] Invalid argument: 'x'"),
        ] {
            let error = io::Error::from_raw_os_error(winerror);
            assert_eq!(oserror_str(&error, "x", OsStyle::Errno), expected);
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_anchors_follow_pure_windows_path() {
        // PureWindowsPath(text).anchor and str(), Python 3.15.0.
        for (text, anchor, shown) in [
            ("C:rel", "C:", "C:rel"),
            ("C:", "C:", "C:"),
            ("c:/x", r"c:\", r"c:\x"),
            ("/rest", r"\", r"\rest"),
            (r"\rest", r"\", r"\rest"),
            ("//server/share/x", r"\\server\share\", r"\\server\share\x"),
        ] {
            let path = PyPath::new(text);
            assert_eq!(
                (path.anchor.as_str(), path.display()),
                (anchor, shown.to_string()),
                "{text}"
            );
        }
    }

    #[test]
    fn the_verbatim_prefix_is_removed() {
        assert_eq!(
            without_verbatim_prefix(r"\\?\C:\a"),
            Some(PathBuf::from(r"C:\a"))
        );
        assert_eq!(
            without_verbatim_prefix(r"\\?\UNC\server\share\a"),
            Some(PathBuf::from(r"\\server\share\a"))
        );
        assert_eq!(without_verbatim_prefix("/a/b"), None);
    }
}
