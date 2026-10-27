//! A YAML loader that reads exactly as the original builder's PyYAML did.
//!
//! This is a port of PyYAML 6.0.3's pure-Python reader, scanner, parser,
//! composer, resolver and safe constructor, with the builder's one change:
//! a repeated mapping key is refused. The sources are the authority for
//! generated controls, so which files load, the values they load as and the
//! wording of every refusal match the Python builder.
//!
//! PyYAML is Copyright (c) 2017-2021 Ingy döt Net and Copyright (c) 2006-2016
//! Kirill Simonov, and is distributed under the MIT licence reproduced in
//! `NOTICE` beside this crate.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::LazyLock;

use num_bigint::BigInt;
use num_traits::Zero;
use regex::Regex;

use crate::py::{self, Date};

/// Why a document could not be loaded.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// A `yaml.YAMLError`, rendered as `str(exc)`.
    Yaml(String),
    /// Python's `RecursionError` while composing a deeply nested document.
    TooDeep,
    /// Any other Python exception: the original builder did not catch these.
    Internal(String),
}

type Result<T> = std::result::Result<T, Error>;

const NAME: &str = "<unicode string>";

#[derive(Clone, Copy, Debug)]
struct Mark {
    line: usize,
    column: usize,
    pointer: usize,
}

const BREAKS_Z: &str = "\0\r\n\u{85}\u{2028}\u{2029}";
const BLANK_Z: &str = "\0 \t\r\n\u{85}\u{2028}\u{2029}";
const SPACE_Z: &str = "\0 \r\n\u{85}\u{2028}\u{2029}";
const BREAKS: &str = "\r\n\u{85}\u{2028}\u{2029}";

fn r(c: char) -> String {
    py::repr_str(&c.to_string())
}

/// The kinds of token, named by their PyYAML ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Directive,
    DocumentStart,
    DocumentEnd,
    StreamStart,
    StreamEnd,
    BlockSequenceStart,
    BlockMappingStart,
    BlockEnd,
    FlowSequenceStart,
    FlowMappingStart,
    FlowSequenceEnd,
    FlowMappingEnd,
    Key,
    Value,
    BlockEntry,
    FlowEntry,
    Alias,
    Anchor,
    Tag,
    Scalar,
}

impl Kind {
    fn id(self) -> &'static str {
        match self {
            Kind::Directive => "<directive>",
            Kind::DocumentStart => "<document start>",
            Kind::DocumentEnd => "<document end>",
            Kind::StreamStart => "<stream start>",
            Kind::StreamEnd => "<stream end>",
            Kind::BlockSequenceStart => "<block sequence start>",
            Kind::BlockMappingStart => "<block mapping start>",
            Kind::BlockEnd => "<block end>",
            Kind::FlowSequenceStart => "[",
            Kind::FlowMappingStart => "{",
            Kind::FlowSequenceEnd => "]",
            Kind::FlowMappingEnd => "}",
            Kind::Key => "?",
            Kind::Value => ":",
            Kind::BlockEntry => "-",
            Kind::FlowEntry => ",",
            Kind::Alias => "<alias>",
            Kind::Anchor => "<anchor>",
            Kind::Tag => "<tag>",
            Kind::Scalar => "<scalar>",
        }
    }
}

#[derive(Clone, Debug)]
enum DirectiveValue {
    None,
    Yaml(BigInt),
    Tag(String, String),
}

#[derive(Clone, Debug)]
struct Token {
    kind: Kind,
    start: Mark,
    end: Mark,
    name: String,
    directive: DirectiveValue,
    value: String,
    handle: Option<String>,
    plain: bool,
    style: Option<char>,
}

impl Token {
    fn new(kind: Kind, start: Mark, end: Mark) -> Self {
        Token {
            kind,
            start,
            end,
            name: String::new(),
            directive: DirectiveValue::None,
            value: String::new(),
            handle: None,
            plain: false,
            style: None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct SimpleKey {
    token_number: usize,
    required: bool,
    index: usize,
    line: usize,
    mark: Mark,
    column: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    StreamStart,
    ImplicitDocumentStart,
    DocumentStart,
    DocumentEnd,
    DocumentContent,
    BlockNode,
    BlockSequenceFirstEntry,
    BlockSequenceEntry,
    IndentlessSequenceEntry,
    BlockMappingFirstKey,
    BlockMappingKey,
    BlockMappingValue,
    FlowSequenceFirstEntry,
    FlowSequenceEntry,
    FlowSequenceEntryMappingKey,
    FlowSequenceEntryMappingValue,
    FlowSequenceEntryMappingEnd,
    FlowMappingFirstKey,
    FlowMappingKey,
    FlowMappingValue,
    FlowMappingEmptyValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EventKind {
    StreamStart,
    StreamEnd,
    DocumentStart,
    DocumentEnd,
    Alias,
    Scalar,
    SequenceStart,
    SequenceEnd,
    MappingStart,
    MappingEnd,
}

#[derive(Clone, Debug)]
struct Event {
    kind: EventKind,
    start: Mark,
    anchor: Option<String>,
    tag: Option<String>,
    implicit: (bool, bool),
    value: String,
}

impl Event {
    fn new(kind: EventKind, start: Mark) -> Self {
        Event {
            kind,
            start,
            anchor: None,
            tag: None,
            implicit: (false, false),
            value: String::new(),
        }
    }
}

#[derive(Clone, Debug)]
enum NodeValue {
    Scalar(String),
    Sequence(Vec<usize>),
    Mapping(Vec<(usize, usize)>),
}

#[derive(Clone, Debug)]
struct Node {
    tag: String,
    value: NodeValue,
    start: Mark,
}

impl Node {
    fn id(&self) -> &'static str {
        match self.value {
            NodeValue::Scalar(_) => "scalar",
            NodeValue::Sequence(_) => "sequence",
            NodeValue::Mapping(_) => "mapping",
        }
    }
}

/// A `datetime.timezone`: the UTC singleton or a fixed offset in seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tz {
    Utc,
    Offset(i64),
}

/// A `datetime.datetime` as PyYAML constructs one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateTime {
    pub date: Date,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub microsecond: u32,
    pub tz: Option<Tz>,
}

impl DateTime {
    fn utc_micros(&self) -> i128 {
        let offset = match self.tz {
            Some(Tz::Offset(seconds)) => seconds,
            _ => 0,
        };
        let seconds = i128::from(self.date.ordinal()) * 86_400
            + i128::from(self.hour) * 3600
            + i128::from(self.minute) * 60
            + i128::from(self.second)
            - i128::from(offset);
        seconds * 1_000_000 + i128::from(self.microsecond)
    }
}

/// A loaded value: the Python object PyYAML would have built.
#[derive(Clone, Debug)]
pub enum Value {
    None,
    Bool(bool),
    Int(BigInt),
    /// A float, with the node it came from: Python compares NaNs by identity.
    Float(f64, usize),
    Str(String),
    Bytes(Vec<u8>),
    Date(Date),
    DateTime(DateTime),
    List(Rc<RefCell<Vec<Value>>>),
    Tuple(Vec<Value>),
    Dict(Rc<RefCell<Vec<(Value, Value)>>>),
    Set(Rc<RefCell<Vec<Value>>>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// `dict.get(key)` for a string key.
    pub fn get(&self, key: &str) -> Option<Value> {
        match self {
            Value::Dict(pairs) => pairs
                .borrow()
                .iter()
                .find(|(k, _)| k.as_str() == Some(key))
                .map(|(_, v)| v.clone()),
            _ => None,
        }
    }

    fn hashable(&self) -> bool {
        !matches!(self, Value::List(_) | Value::Dict(_) | Value::Set(_))
    }

    /// `repr(value)`, or the `ValueError` an over-long `int` raises.
    pub fn repr(&self) -> std::result::Result<String, String> {
        let mut seen = Vec::new();
        self.repr_into(&mut seen)
    }

    fn repr_into(&self, seen: &mut Vec<usize>) -> std::result::Result<String, String> {
        Ok(match self {
            Value::None => "None".into(),
            Value::Bool(true) => "True".into(),
            Value::Bool(false) => "False".into(),
            Value::Int(i) => py::int_repr(i)?,
            Value::Float(f, _) => py::float_repr(*f),
            Value::Str(s) => py::repr_str(s),
            Value::Bytes(b) => py::repr_bytes(b),
            Value::Date(d) => format!("datetime.date({}, {}, {})", d.year, d.month, d.day),
            Value::DateTime(dt) => {
                let d = dt.date;
                let mut out = format!(
                    "datetime.datetime({}, {}, {}, {}, {}",
                    d.year, d.month, d.day, dt.hour, dt.minute
                );
                if dt.microsecond != 0 {
                    out.push_str(&format!(", {}, {}", dt.second, dt.microsecond));
                } else if dt.second != 0 {
                    out.push_str(&format!(", {}", dt.second));
                }
                match dt.tz {
                    None => {}
                    Some(Tz::Utc) => out.push_str(", tzinfo=datetime.timezone.utc"),
                    Some(Tz::Offset(seconds)) => {
                        let days = seconds.div_euclid(86_400);
                        let rest = seconds.rem_euclid(86_400);
                        let mut fields = Vec::new();
                        if days != 0 {
                            fields.push(format!("days={days}"));
                        }
                        if rest != 0 {
                            fields.push(format!("seconds={rest}"));
                        }
                        out.push_str(&format!(
                            ", tzinfo=datetime.timezone(datetime.timedelta({})))",
                            fields.join(", ")
                        ));
                        return Ok(out);
                    }
                }
                out.push(')');
                out
            }
            Value::List(items) => {
                let id = Rc::as_ptr(items) as usize;
                if seen.contains(&id) {
                    return Ok("[...]".into());
                }
                seen.push(id);
                let parts = items
                    .borrow()
                    .iter()
                    .map(|v| v.repr_into(seen))
                    .collect::<std::result::Result<Vec<_>, _>>();
                seen.pop();
                format!("[{}]", parts?.join(", "))
            }
            Value::Tuple(items) => {
                let parts = items
                    .iter()
                    .map(|v| v.repr_into(seen))
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                if parts.len() == 1 {
                    format!("({},)", parts[0])
                } else {
                    format!("({})", parts.join(", "))
                }
            }
            Value::Dict(pairs) => {
                let id = Rc::as_ptr(pairs) as usize;
                if seen.contains(&id) {
                    return Ok("{...}".into());
                }
                seen.push(id);
                let mut parts = Vec::new();
                for (k, v) in pairs.borrow().iter() {
                    match (k.repr_into(seen), v.repr_into(seen)) {
                        (Ok(k), Ok(v)) => parts.push(format!("{k}: {v}")),
                        (Err(e), _) | (_, Err(e)) => {
                            seen.pop();
                            return Err(e);
                        }
                    }
                }
                seen.pop();
                format!("{{{}}}", parts.join(", "))
            }
            // shortcut: Python orders a set by hash, which varies between runs
            // for strings; insertion order is shown instead.
            // Compare order-independently if output is checked against Python's for a set.
            Value::Set(items) => {
                let items = items.borrow();
                if items.is_empty() {
                    "set()".into()
                } else {
                    let parts = items
                        .iter()
                        .map(|v| v.repr_into(seen))
                        .collect::<std::result::Result<Vec<_>, _>>()?;
                    format!("{{{}}}", parts.join(", "))
                }
            }
        })
    }

    fn number(&self) -> Option<std::result::Result<BigInt, f64>> {
        match self {
            Value::Bool(b) => Some(Ok(BigInt::from(u8::from(*b)))),
            Value::Int(i) => Some(Ok(i.clone())),
            Value::Float(f, _) => Some(Err(*f)),
            _ => None,
        }
    }

    /// Python's `==` for the hashable values a mapping key can hold.
    fn py_eq(&self, other: &Value) -> bool {
        if let (Some(a), Some(b)) = (self.number(), other.number()) {
            return match (a, b) {
                (Ok(a), Ok(b)) => a == b,
                (Err(a), Err(b)) => {
                    if let (Value::Float(_, x), Value::Float(_, y)) = (self, other)
                        && x == y
                    {
                        return true;
                    }
                    a == b
                }
                (Ok(i), Err(f)) | (Err(f), Ok(i)) => {
                    f.is_finite()
                        && f.fract() == 0.0
                        && num_bigint::BigInt::from_f64_exact(f).is_some_and(|g| g == i)
                }
            };
        }
        match (self, other) {
            (Value::None, Value::None) => true,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Bytes(a), Value::Bytes(b)) => a == b,
            (Value::Date(a), Value::Date(b)) => a == b,
            (Value::DateTime(a), Value::DateTime(b)) => match (a.tz.is_some(), b.tz.is_some()) {
                (false, false) => a == b,
                (true, true) => a.utc_micros() == b.utc_micros(),
                _ => false,
            },
            (Value::Tuple(a), Value::Tuple(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.py_eq(y))
            }
            _ => false,
        }
    }
}

trait FromF64Exact {
    fn from_f64_exact(f: f64) -> Option<BigInt>;
}

impl FromF64Exact for BigInt {
    fn from_f64_exact(f: f64) -> Option<BigInt> {
        num_traits::FromPrimitive::from_f64(f)
    }
}

/// The deepest nesting the Python builder composed before its recursion limit.
pub const MAX_DEPTH: usize = 490;

/// Load one document as `yaml.load(text, Loader=_UniqueKeyLoader)` did.
pub fn load(text: &str) -> Result<Value> {
    let mut loader = Loader::new(text)?;
    loader.get_single_data()
}

struct Loader {
    buffer: Vec<char>,
    pointer: usize,
    line: usize,
    column: usize,
    // Scanner.
    done: bool,
    flow_level: usize,
    tokens: VecDeque<Token>,
    tokens_taken: usize,
    indent: i64,
    indents: Vec<i64>,
    allow_simple_key: bool,
    possible_simple_keys: Vec<(usize, SimpleKey)>,
    // Parser.
    current_event: Option<Event>,
    tag_handles: Vec<(String, String)>,
    states: Vec<State>,
    marks: Vec<Mark>,
    state: Option<State>,
    // Composer and constructor.
    anchors: HashMap<String, usize>,
    nodes: Vec<Node>,
    depth: usize,
    constructed: HashMap<usize, Value>,
    recursive: HashSet<usize>,
    pending: Vec<Pending>,
}

enum Pending {
    Seq(usize, Rc<RefCell<Vec<Value>>>),
    Omap(usize, Rc<RefCell<Vec<Value>>>, &'static str),
    Set(usize, Rc<RefCell<Vec<Value>>>),
}

fn default_tags() -> Vec<(String, String)> {
    vec![
        ("!".into(), "!".into()),
        ("!!".into(), "tag:yaml.org,2002:".into()),
    ]
}

impl Loader {
    fn new(text: &str) -> Result<Self> {
        // Reader.check_printable.
        for (position, c) in text.chars().enumerate() {
            let cp = c as u32;
            let printable = matches!(cp, 0x09 | 0x0A | 0x0D | 0x20..=0x7E | 0x85 | 0xA0..=0xD7FF | 0xE000..=0xFFFD)
                || cp >= 0x10000;
            if !printable {
                return Err(Error::Yaml(format!(
                    "unacceptable character #x{cp:04x}: special characters are not allowed\n  in \"{NAME}\", position {position}"
                )));
            }
        }
        let mut buffer: Vec<char> = text.chars().collect();
        buffer.push('\0');
        let start = Mark {
            line: 0,
            column: 0,
            pointer: 0,
        };
        let mut tokens = VecDeque::new();
        tokens.push_back(Token::new(Kind::StreamStart, start, start));
        Ok(Loader {
            buffer,
            pointer: 0,
            line: 0,
            column: 0,
            done: false,
            flow_level: 0,
            tokens,
            tokens_taken: 0,
            indent: -1,
            indents: Vec::new(),
            allow_simple_key: true,
            possible_simple_keys: Vec::new(),
            current_event: None,
            tag_handles: Vec::new(),
            states: Vec::new(),
            marks: Vec::new(),
            state: Some(State::StreamStart),
            anchors: HashMap::new(),
            nodes: Vec::new(),
            depth: 0,
            constructed: HashMap::new(),
            recursive: HashSet::new(),
            pending: Vec::new(),
        })
    }

    // Errors.

    fn snippet(&self, mark: Mark) -> String {
        let breaks = |c: char| BREAKS_Z.contains(c);
        let mut head = "";
        let mut start = mark.pointer;
        while start > 0 && !breaks(self.buffer[start - 1]) {
            start -= 1;
            if mark.pointer - start > 36 {
                head = " ... ";
                start += 5;
                break;
            }
        }
        let mut tail = "";
        let mut end = mark.pointer;
        while end < self.buffer.len() && !breaks(self.buffer[end]) {
            end += 1;
            if end - mark.pointer > 36 {
                tail = " ... ";
                end -= 5;
                break;
            }
        }
        let text: String = self.buffer[start..end].iter().collect();
        format!(
            "    {head}{text}{tail}\n{}^",
            " ".repeat(4 + mark.pointer - start + head.len())
        )
    }

    fn mark_str(&self, mark: Mark) -> String {
        format!(
            "  in \"{NAME}\", line {}, column {}:\n{}",
            mark.line + 1,
            mark.column + 1,
            self.snippet(mark)
        )
    }

    fn marked(
        &self,
        context: Option<&str>,
        context_mark: Option<Mark>,
        problem: Option<String>,
        problem_mark: Option<Mark>,
    ) -> Error {
        let mut lines = Vec::new();
        if let Some(c) = context {
            lines.push(c.to_string());
        }
        if let Some(cm) = context_mark {
            let same = problem.is_some()
                && problem_mark.is_some_and(|pm| pm.line == cm.line && pm.column == cm.column);
            if !same {
                lines.push(self.mark_str(cm));
            }
        }
        if let Some(p) = problem {
            lines.push(p);
        }
        if let Some(pm) = problem_mark {
            lines.push(self.mark_str(pm));
        }
        Error::Yaml(lines.join("\n"))
    }

    // Reader.

    fn peek(&self, index: usize) -> char {
        self.buffer
            .get(self.pointer + index)
            .copied()
            .unwrap_or('\0')
    }

    fn prefix(&self, length: usize) -> String {
        let end = (self.pointer + length).min(self.buffer.len());
        self.buffer[self.pointer..end].iter().collect()
    }

    fn forward(&mut self, length: usize) {
        for _ in 0..length {
            let ch = self.buffer[self.pointer];
            self.pointer += 1;
            if "\n\u{85}\u{2028}\u{2029}".contains(ch) || ch == '\r' && self.peek(0) != '\n' {
                self.line += 1;
                self.column = 0;
            } else if ch != '\u{FEFF}' {
                self.column += 1;
            }
        }
    }

    fn get_mark(&self) -> Mark {
        Mark {
            line: self.line,
            column: self.column,
            pointer: self.pointer,
        }
    }

    // Scanner: public methods.

    /// Scan until the next token is settled: none of the pending simple
    /// keys can still turn it into a mapping key.
    fn fetch_tokens(&mut self) -> Result<()> {
        while self.need_more_tokens()? {
            self.fetch_more_tokens()?;
        }
        Ok(())
    }

    fn check_token(&mut self, choices: &[Kind]) -> Result<bool> {
        self.fetch_tokens()?;
        Ok(self
            .tokens
            .front()
            .is_some_and(|t| choices.is_empty() || choices.contains(&t.kind)))
    }

    fn peek_token(&mut self) -> Result<Token> {
        self.fetch_tokens()?;
        Ok(self
            .tokens
            .front()
            .cloned()
            .expect("a token follows until the stream ends"))
    }

    fn get_token(&mut self) -> Result<Token> {
        self.fetch_tokens()?;
        self.tokens_taken += 1;
        Ok(self
            .tokens
            .pop_front()
            .expect("a token follows until the stream ends"))
    }

    fn need_more_tokens(&mut self) -> Result<bool> {
        if self.done {
            return Ok(false);
        }
        if self.tokens.is_empty() {
            return Ok(true);
        }
        self.stale_possible_simple_keys()?;
        Ok(self.next_possible_simple_key() == Some(self.tokens_taken))
    }

    fn fetch_more_tokens(&mut self) -> Result<()> {
        self.scan_to_next_token();
        self.stale_possible_simple_keys()?;
        self.unwind_indent(self.column as i64);
        let ch = self.peek(0);
        match ch {
            '\0' => return self.fetch_stream_end(),
            '%' if self.check_directive() => return self.fetch_directive(),
            '-' if self.check_document_start() => {
                return self.fetch_document_indicator(Kind::DocumentStart);
            }
            '.' if self.check_document_end() => {
                return self.fetch_document_indicator(Kind::DocumentEnd);
            }
            _ => {}
        }
        match ch {
            '[' => self.fetch_flow_collection_start(Kind::FlowSequenceStart),
            '{' => self.fetch_flow_collection_start(Kind::FlowMappingStart),
            ']' => self.fetch_flow_collection_end(Kind::FlowSequenceEnd),
            '}' => self.fetch_flow_collection_end(Kind::FlowMappingEnd),
            ',' => self.fetch_flow_entry(),
            '-' if self.check_block_entry() => self.fetch_block_entry(),
            '?' if self.check_key() => self.fetch_key(),
            ':' if self.check_value() => self.fetch_value(),
            '*' => self.fetch_anchor(Kind::Alias),
            '&' => self.fetch_anchor(Kind::Anchor),
            '!' => self.fetch_tag(),
            '|' if self.flow_level == 0 => self.fetch_block_scalar('|'),
            '>' if self.flow_level == 0 => self.fetch_block_scalar('>'),
            '\'' => self.fetch_flow_scalar('\''),
            '"' => self.fetch_flow_scalar('"'),
            _ if self.check_plain() => self.fetch_plain(),
            _ => Err(self.marked(
                Some("while scanning for the next token"),
                None,
                Some(format!(
                    "found character {} that cannot start any token",
                    r(ch)
                )),
                Some(self.get_mark()),
            )),
        }
    }

    // Simple keys.

    fn next_possible_simple_key(&self) -> Option<usize> {
        self.possible_simple_keys
            .iter()
            .map(|(_, k)| k.token_number)
            .min()
    }

    fn stale_possible_simple_keys(&mut self) -> Result<()> {
        let mut i = 0;
        while i < self.possible_simple_keys.len() {
            let key = self.possible_simple_keys[i].1;
            if key.line != self.line || self.pointer - key.index > 1024 {
                if key.required {
                    return Err(self.marked(
                        Some("while scanning a simple key"),
                        Some(key.mark),
                        Some("could not find expected ':'".into()),
                        Some(self.get_mark()),
                    ));
                }
                self.possible_simple_keys.remove(i);
            } else {
                i += 1;
            }
        }
        Ok(())
    }

    fn save_possible_simple_key(&mut self) -> Result<()> {
        let required = self.flow_level == 0 && self.indent == self.column as i64;
        if self.allow_simple_key {
            self.remove_possible_simple_key()?;
            let key = SimpleKey {
                token_number: self.tokens_taken + self.tokens.len(),
                required,
                index: self.pointer,
                line: self.line,
                column: self.column,
                mark: self.get_mark(),
            };
            self.possible_simple_keys.push((self.flow_level, key));
        }
        Ok(())
    }

    fn remove_possible_simple_key(&mut self) -> Result<()> {
        if let Some(i) = self
            .possible_simple_keys
            .iter()
            .position(|(l, _)| *l == self.flow_level)
        {
            let key = self.possible_simple_keys[i].1;
            if key.required {
                return Err(self.marked(
                    Some("while scanning a simple key"),
                    Some(key.mark),
                    Some("could not find expected ':'".into()),
                    Some(self.get_mark()),
                ));
            }
            self.possible_simple_keys.remove(i);
        }
        Ok(())
    }

    // Indentation.

    fn unwind_indent(&mut self, column: i64) {
        if self.flow_level > 0 {
            return;
        }
        while self.indent > column {
            let mark = self.get_mark();
            self.indent = self.indents.pop().expect("an indentation level to restore");
            self.tokens
                .push_back(Token::new(Kind::BlockEnd, mark, mark));
        }
    }

    fn add_indent(&mut self, column: i64) -> bool {
        if self.indent < column {
            self.indents.push(self.indent);
            self.indent = column;
            return true;
        }
        false
    }

    // Fetchers.

    fn fetch_stream_end(&mut self) -> Result<()> {
        self.unwind_indent(-1);
        self.remove_possible_simple_key()?;
        self.allow_simple_key = false;
        self.possible_simple_keys.clear();
        let mark = self.get_mark();
        self.tokens
            .push_back(Token::new(Kind::StreamEnd, mark, mark));
        self.done = true;
        Ok(())
    }

    fn fetch_directive(&mut self) -> Result<()> {
        self.unwind_indent(-1);
        self.remove_possible_simple_key()?;
        self.allow_simple_key = false;
        let token = self.scan_directive()?;
        self.tokens.push_back(token);
        Ok(())
    }

    fn fetch_document_indicator(&mut self, kind: Kind) -> Result<()> {
        self.unwind_indent(-1);
        self.remove_possible_simple_key()?;
        self.allow_simple_key = false;
        let start = self.get_mark();
        self.forward(3);
        let end = self.get_mark();
        self.tokens.push_back(Token::new(kind, start, end));
        Ok(())
    }

    fn fetch_flow_collection_start(&mut self, kind: Kind) -> Result<()> {
        self.save_possible_simple_key()?;
        self.flow_level += 1;
        self.allow_simple_key = true;
        let start = self.get_mark();
        self.forward(1);
        let end = self.get_mark();
        self.tokens.push_back(Token::new(kind, start, end));
        Ok(())
    }

    fn fetch_flow_collection_end(&mut self, kind: Kind) -> Result<()> {
        self.remove_possible_simple_key()?;
        // Python's flow level can go negative on a stray closer; it then
        // behaves as block context again, which saturating mirrors for the
        // checks that read it.
        self.flow_level = self.flow_level.saturating_sub(1);
        self.allow_simple_key = false;
        let start = self.get_mark();
        self.forward(1);
        let end = self.get_mark();
        self.tokens.push_back(Token::new(kind, start, end));
        Ok(())
    }

    fn fetch_flow_entry(&mut self) -> Result<()> {
        self.allow_simple_key = true;
        self.remove_possible_simple_key()?;
        let start = self.get_mark();
        self.forward(1);
        let end = self.get_mark();
        self.tokens
            .push_back(Token::new(Kind::FlowEntry, start, end));
        Ok(())
    }

    fn fetch_block_entry(&mut self) -> Result<()> {
        if self.flow_level == 0 {
            if !self.allow_simple_key {
                return Err(self.marked(
                    None,
                    None,
                    Some("sequence entries are not allowed here".into()),
                    Some(self.get_mark()),
                ));
            }
            if self.add_indent(self.column as i64) {
                let mark = self.get_mark();
                self.tokens
                    .push_back(Token::new(Kind::BlockSequenceStart, mark, mark));
            }
        }
        self.allow_simple_key = true;
        self.remove_possible_simple_key()?;
        let start = self.get_mark();
        self.forward(1);
        let end = self.get_mark();
        self.tokens
            .push_back(Token::new(Kind::BlockEntry, start, end));
        Ok(())
    }

    fn fetch_key(&mut self) -> Result<()> {
        if self.flow_level == 0 {
            if !self.allow_simple_key {
                return Err(self.marked(
                    None,
                    None,
                    Some("mapping keys are not allowed here".into()),
                    Some(self.get_mark()),
                ));
            }
            if self.add_indent(self.column as i64) {
                let mark = self.get_mark();
                self.tokens
                    .push_back(Token::new(Kind::BlockMappingStart, mark, mark));
            }
        }
        self.allow_simple_key = self.flow_level == 0;
        self.remove_possible_simple_key()?;
        let start = self.get_mark();
        self.forward(1);
        let end = self.get_mark();
        self.tokens.push_back(Token::new(Kind::Key, start, end));
        Ok(())
    }

    fn fetch_value(&mut self) -> Result<()> {
        if let Some(i) = self
            .possible_simple_keys
            .iter()
            .position(|(l, _)| *l == self.flow_level)
        {
            let key = self.possible_simple_keys.remove(i).1;
            let at = key.token_number - self.tokens_taken;
            self.tokens
                .insert(at, Token::new(Kind::Key, key.mark, key.mark));
            if self.flow_level == 0 && self.add_indent(key.column as i64) {
                self.tokens
                    .insert(at, Token::new(Kind::BlockMappingStart, key.mark, key.mark));
            }
            self.allow_simple_key = false;
        } else {
            if self.flow_level == 0 && !self.allow_simple_key {
                return Err(self.marked(
                    None,
                    None,
                    Some("mapping values are not allowed here".into()),
                    Some(self.get_mark()),
                ));
            }
            if self.flow_level == 0 && self.add_indent(self.column as i64) {
                let mark = self.get_mark();
                self.tokens
                    .push_back(Token::new(Kind::BlockMappingStart, mark, mark));
            }
            self.allow_simple_key = self.flow_level == 0;
            self.remove_possible_simple_key()?;
        }
        let start = self.get_mark();
        self.forward(1);
        let end = self.get_mark();
        self.tokens.push_back(Token::new(Kind::Value, start, end));
        Ok(())
    }

    fn fetch_anchor(&mut self, kind: Kind) -> Result<()> {
        self.save_possible_simple_key()?;
        self.allow_simple_key = false;
        let token = self.scan_anchor(kind)?;
        self.tokens.push_back(token);
        Ok(())
    }

    fn fetch_tag(&mut self) -> Result<()> {
        self.save_possible_simple_key()?;
        self.allow_simple_key = false;
        let token = self.scan_tag()?;
        self.tokens.push_back(token);
        Ok(())
    }

    fn fetch_block_scalar(&mut self, style: char) -> Result<()> {
        self.allow_simple_key = true;
        self.remove_possible_simple_key()?;
        let token = self.scan_block_scalar(style)?;
        self.tokens.push_back(token);
        Ok(())
    }

    fn fetch_flow_scalar(&mut self, style: char) -> Result<()> {
        self.save_possible_simple_key()?;
        self.allow_simple_key = false;
        let token = self.scan_flow_scalar(style)?;
        self.tokens.push_back(token);
        Ok(())
    }

    fn fetch_plain(&mut self) -> Result<()> {
        self.save_possible_simple_key()?;
        self.allow_simple_key = false;
        let token = self.scan_plain()?;
        self.tokens.push_back(token);
        Ok(())
    }

    // Checkers.

    fn check_directive(&self) -> bool {
        self.column == 0
    }

    fn check_document_start(&self) -> bool {
        self.column == 0 && self.prefix(3) == "---" && BLANK_Z.contains(self.peek(3))
    }

    fn check_document_end(&self) -> bool {
        self.column == 0 && self.prefix(3) == "..." && BLANK_Z.contains(self.peek(3))
    }

    fn check_block_entry(&self) -> bool {
        BLANK_Z.contains(self.peek(1))
    }

    fn check_key(&self) -> bool {
        self.flow_level > 0 || BLANK_Z.contains(self.peek(1))
    }

    fn check_value(&self) -> bool {
        self.flow_level > 0 || BLANK_Z.contains(self.peek(1))
    }

    fn check_plain(&self) -> bool {
        let ch = self.peek(0);
        !"\0 \t\r\n\u{85}\u{2028}\u{2029}-?:,[]{}#&*!|>'\"%@`".contains(ch)
            || !BLANK_Z.contains(self.peek(1))
                && (ch == '-' || self.flow_level == 0 && "?:".contains(ch))
    }

    // Scanners.

    fn scan_to_next_token(&mut self) {
        if self.pointer == 0 && self.peek(0) == '\u{FEFF}' {
            self.forward(1);
        }
        loop {
            while self.peek(0) == ' ' {
                self.forward(1);
            }
            if self.peek(0) == '#' {
                while !BREAKS_Z.contains(self.peek(0)) {
                    self.forward(1);
                }
            }
            if self.scan_line_break().is_empty() {
                break;
            }
            if self.flow_level == 0 {
                self.allow_simple_key = true;
            }
        }
    }

    fn scan_directive(&mut self) -> Result<Token> {
        let start = self.get_mark();
        self.forward(1);
        let name = self.scan_directive_name(start)?;
        let value;
        let end;
        if name == "YAML" {
            value = self.scan_yaml_directive_value(start)?;
            end = self.get_mark();
        } else if name == "TAG" {
            value = self.scan_tag_directive_value(start)?;
            end = self.get_mark();
        } else {
            value = DirectiveValue::None;
            end = self.get_mark();
            while !BREAKS_Z.contains(self.peek(0)) {
                self.forward(1);
            }
        }
        self.scan_directive_ignored_line(start)?;
        let mut token = Token::new(Kind::Directive, start, end);
        token.name = name;
        token.directive = value;
        Ok(token)
    }

    fn directive_error(&self, start: Mark, problem: String) -> Error {
        self.marked(
            Some("while scanning a directive"),
            Some(start),
            Some(problem),
            Some(self.get_mark()),
        )
    }

    fn scan_directive_name(&mut self, start: Mark) -> Result<String> {
        let mut length = 0;
        let mut ch = self.peek(length);
        while ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            length += 1;
            ch = self.peek(length);
        }
        if length == 0 {
            return Err(self.directive_error(
                start,
                format!(
                    "expected alphabetic or numeric character, but found {}",
                    r(ch)
                ),
            ));
        }
        let value = self.prefix(length);
        self.forward(length);
        let ch = self.peek(0);
        if !SPACE_Z.contains(ch) {
            return Err(self.directive_error(
                start,
                format!(
                    "expected alphabetic or numeric character, but found {}",
                    r(ch)
                ),
            ));
        }
        Ok(value)
    }

    fn scan_yaml_directive_value(&mut self, start: Mark) -> Result<DirectiveValue> {
        while self.peek(0) == ' ' {
            self.forward(1);
        }
        let major = self.scan_yaml_directive_number(start)?;
        if self.peek(0) != '.' {
            return Err(self.directive_error(
                start,
                format!("expected a digit or '.', but found {}", r(self.peek(0))),
            ));
        }
        self.forward(1);
        let minor = self.scan_yaml_directive_number(start)?;
        if !SPACE_Z.contains(self.peek(0)) {
            return Err(self.directive_error(
                start,
                format!("expected a digit or ' ', but found {}", r(self.peek(0))),
            ));
        }
        let _ = minor;
        Ok(DirectiveValue::Yaml(major))
    }

    fn scan_yaml_directive_number(&mut self, start: Mark) -> Result<BigInt> {
        let ch = self.peek(0);
        if !ch.is_ascii_digit() {
            return Err(
                self.directive_error(start, format!("expected a digit, but found {}", r(ch)))
            );
        }
        let mut length = 0;
        while self.peek(length).is_ascii_digit() {
            length += 1;
        }
        let text = self.prefix(length);
        let value = py::parse_int(&text, 10).map_err(Error::Internal)?;
        self.forward(length);
        Ok(value)
    }

    fn scan_tag_directive_value(&mut self, start: Mark) -> Result<DirectiveValue> {
        while self.peek(0) == ' ' {
            self.forward(1);
        }
        let handle = self.scan_tag_handle("directive", start)?;
        if self.peek(0) != ' ' {
            return Err(self.directive_error(
                start,
                format!("expected ' ', but found {}", r(self.peek(0))),
            ));
        }
        while self.peek(0) == ' ' {
            self.forward(1);
        }
        let prefix = self.scan_tag_uri("directive", start)?;
        if !SPACE_Z.contains(self.peek(0)) {
            return Err(self.directive_error(
                start,
                format!("expected ' ', but found {}", r(self.peek(0))),
            ));
        }
        Ok(DirectiveValue::Tag(handle, prefix))
    }

    fn scan_directive_ignored_line(&mut self, start: Mark) -> Result<()> {
        while self.peek(0) == ' ' {
            self.forward(1);
        }
        if self.peek(0) == '#' {
            while !BREAKS_Z.contains(self.peek(0)) {
                self.forward(1);
            }
        }
        let ch = self.peek(0);
        if !BREAKS_Z.contains(ch) {
            return Err(self.directive_error(
                start,
                format!("expected a comment or a line break, but found {}", r(ch)),
            ));
        }
        self.scan_line_break();
        Ok(())
    }

    fn scan_anchor(&mut self, kind: Kind) -> Result<Token> {
        let start = self.get_mark();
        let name = if self.peek(0) == '*' {
            "alias"
        } else {
            "anchor"
        };
        self.forward(1);
        let mut length = 0;
        let mut ch = self.peek(length);
        while ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            length += 1;
            ch = self.peek(length);
        }
        let context = format!("while scanning an {name}");
        if length == 0 {
            return Err(self.marked(
                Some(&context),
                Some(start),
                Some(format!(
                    "expected alphabetic or numeric character, but found {}",
                    r(ch)
                )),
                Some(self.get_mark()),
            ));
        }
        let value = self.prefix(length);
        self.forward(length);
        let ch = self.peek(0);
        if !"\0 \t\r\n\u{85}\u{2028}\u{2029}?:,]}%@`".contains(ch) {
            return Err(self.marked(
                Some(&context),
                Some(start),
                Some(format!(
                    "expected alphabetic or numeric character, but found {}",
                    r(ch)
                )),
                Some(self.get_mark()),
            ));
        }
        let end = self.get_mark();
        let mut token = Token::new(kind, start, end);
        token.value = value;
        Ok(token)
    }

    fn scan_tag(&mut self) -> Result<Token> {
        let start = self.get_mark();
        let mut ch = self.peek(1);
        let handle;
        let suffix;
        if ch == '<' {
            handle = None;
            self.forward(2);
            suffix = self.scan_tag_uri("tag", start)?;
            if self.peek(0) != '>' {
                return Err(self.marked(
                    Some("while parsing a tag"),
                    Some(start),
                    Some(format!("expected '>', but found {}", r(self.peek(0)))),
                    Some(self.get_mark()),
                ));
            }
            self.forward(1);
        } else if BLANK_Z.contains(ch) {
            handle = None;
            suffix = "!".to_string();
            self.forward(1);
        } else {
            let mut length = 1;
            let mut use_handle = false;
            while !SPACE_Z.contains(ch) {
                if ch == '!' {
                    use_handle = true;
                    break;
                }
                length += 1;
                ch = self.peek(length);
            }
            if use_handle {
                handle = Some(self.scan_tag_handle("tag", start)?);
            } else {
                handle = Some("!".to_string());
                self.forward(1);
            }
            suffix = self.scan_tag_uri("tag", start)?;
        }
        let ch = self.peek(0);
        if !SPACE_Z.contains(ch) {
            return Err(self.marked(
                Some("while scanning a tag"),
                Some(start),
                Some(format!("expected ' ', but found {}", r(ch))),
                Some(self.get_mark()),
            ));
        }
        let end = self.get_mark();
        let mut token = Token::new(Kind::Tag, start, end);
        token.handle = handle;
        token.value = suffix;
        Ok(token)
    }

    fn scan_block_scalar(&mut self, style: char) -> Result<Token> {
        let folded = style == '>';
        let mut chunks = String::new();
        let start = self.get_mark();
        self.forward(1);
        let (chomping, increment) = self.scan_block_scalar_indicators(start)?;
        self.scan_block_scalar_ignored_line(start)?;
        let min_indent = (self.indent + 1).max(1) as usize;
        let (mut breaks, mut end, indent) = match increment {
            None => {
                let (breaks, max_indent, end) = self.scan_block_scalar_indentation();
                (breaks, end, min_indent.max(max_indent))
            }
            Some(increment) => {
                let indent = min_indent + increment - 1;
                let (breaks, end) = self.scan_block_scalar_breaks(indent);
                (breaks, end, indent)
            }
        };
        let mut line_break = String::new();
        while self.column == indent && self.peek(0) != '\0' {
            chunks.push_str(&breaks.concat());
            let leading_non_space = !" \t".contains(self.peek(0));
            let mut length = 0;
            while !BREAKS_Z.contains(self.peek(length)) {
                length += 1;
            }
            chunks.push_str(&self.prefix(length));
            self.forward(length);
            line_break = self.scan_line_break();
            let (b, e) = self.scan_block_scalar_breaks(indent);
            breaks = b;
            end = e;
            if self.column == indent && self.peek(0) != '\0' {
                if folded
                    && line_break == "\n"
                    && leading_non_space
                    && !" \t".contains(self.peek(0))
                {
                    if breaks.is_empty() {
                        chunks.push(' ');
                    }
                } else {
                    chunks.push_str(&line_break);
                }
            } else {
                break;
            }
        }
        if chomping != Some(false) {
            chunks.push_str(&line_break);
        }
        if chomping == Some(true) {
            chunks.push_str(&breaks.concat());
        }
        let mut token = Token::new(Kind::Scalar, start, end);
        token.value = chunks;
        token.style = Some(style);
        Ok(token)
    }

    fn scan_block_scalar_indicators(
        &mut self,
        start: Mark,
    ) -> Result<(Option<bool>, Option<usize>)> {
        let mut chomping = None;
        let mut increment = None;
        let zero = |me: &Self| {
            me.marked(
                Some("while scanning a block scalar"),
                Some(start),
                Some("expected indentation indicator in the range 1-9, but found 0".into()),
                Some(me.get_mark()),
            )
        };
        let mut ch = self.peek(0);
        if ch == '+' || ch == '-' {
            chomping = Some(ch == '+');
            self.forward(1);
            ch = self.peek(0);
            if ch.is_ascii_digit() {
                let value = ch.to_digit(10).expect("digit") as usize;
                if value == 0 {
                    return Err(zero(self));
                }
                increment = Some(value);
                self.forward(1);
            }
        } else if ch.is_ascii_digit() {
            let value = ch.to_digit(10).expect("digit") as usize;
            if value == 0 {
                return Err(zero(self));
            }
            increment = Some(value);
            self.forward(1);
            ch = self.peek(0);
            if ch == '+' || ch == '-' {
                chomping = Some(ch == '+');
                self.forward(1);
            }
        }
        let ch = self.peek(0);
        if !SPACE_Z.contains(ch) {
            return Err(self.marked(
                Some("while scanning a block scalar"),
                Some(start),
                Some(format!(
                    "expected chomping or indentation indicators, but found {}",
                    r(ch)
                )),
                Some(self.get_mark()),
            ));
        }
        Ok((chomping, increment))
    }

    fn scan_block_scalar_ignored_line(&mut self, start: Mark) -> Result<()> {
        while self.peek(0) == ' ' {
            self.forward(1);
        }
        if self.peek(0) == '#' {
            while !BREAKS_Z.contains(self.peek(0)) {
                self.forward(1);
            }
        }
        let ch = self.peek(0);
        if !BREAKS_Z.contains(ch) {
            return Err(self.marked(
                Some("while scanning a block scalar"),
                Some(start),
                Some(format!(
                    "expected a comment or a line break, but found {}",
                    r(ch)
                )),
                Some(self.get_mark()),
            ));
        }
        self.scan_line_break();
        Ok(())
    }

    fn scan_block_scalar_indentation(&mut self) -> (Vec<String>, usize, Mark) {
        let mut chunks = Vec::new();
        let mut max_indent = 0;
        let mut end = self.get_mark();
        while " \r\n\u{85}\u{2028}\u{2029}".contains(self.peek(0)) && self.peek(0) != '\0' {
            if self.peek(0) != ' ' {
                chunks.push(self.scan_line_break());
                end = self.get_mark();
            } else {
                self.forward(1);
                if self.column > max_indent {
                    max_indent = self.column;
                }
            }
        }
        (chunks, max_indent, end)
    }

    fn scan_block_scalar_breaks(&mut self, indent: usize) -> (Vec<String>, Mark) {
        let mut chunks = Vec::new();
        let mut end = self.get_mark();
        while self.column < indent && self.peek(0) == ' ' {
            self.forward(1);
        }
        while BREAKS.contains(self.peek(0)) && self.peek(0) != '\0' {
            chunks.push(self.scan_line_break());
            end = self.get_mark();
            while self.column < indent && self.peek(0) == ' ' {
                self.forward(1);
            }
        }
        (chunks, end)
    }

    fn scan_flow_scalar(&mut self, style: char) -> Result<Token> {
        let double = style == '"';
        let mut chunks = String::new();
        let start = self.get_mark();
        let quote = self.peek(0);
        self.forward(1);
        self.scan_flow_scalar_non_spaces(double, start, &mut chunks)?;
        while self.peek(0) != quote {
            self.scan_flow_scalar_spaces(double, start, &mut chunks)?;
            self.scan_flow_scalar_non_spaces(double, start, &mut chunks)?;
        }
        self.forward(1);
        let end = self.get_mark();
        let mut token = Token::new(Kind::Scalar, start, end);
        token.value = chunks;
        token.style = Some(style);
        Ok(token)
    }

    fn scan_flow_scalar_non_spaces(
        &mut self,
        double: bool,
        start: Mark,
        chunks: &mut String,
    ) -> Result<()> {
        loop {
            let mut length = 0;
            while !"'\"\\\0 \t\r\n\u{85}\u{2028}\u{2029}".contains(self.peek(length)) {
                length += 1;
            }
            if length > 0 {
                chunks.push_str(&self.prefix(length));
                self.forward(length);
            }
            let ch = self.peek(0);
            if !double && ch == '\'' && self.peek(1) == '\'' {
                chunks.push('\'');
                self.forward(2);
            } else if double && ch == '\'' || !double && (ch == '"' || ch == '\\') {
                chunks.push(ch);
                self.forward(1);
            } else if double && ch == '\\' {
                self.forward(1);
                let ch = self.peek(0);
                let replacement = match ch {
                    '0' => Some('\0'),
                    'a' => Some('\x07'),
                    'b' => Some('\x08'),
                    't' | '\t' => Some('\t'),
                    'n' => Some('\n'),
                    'v' => Some('\x0b'),
                    'f' => Some('\x0c'),
                    'r' => Some('\r'),
                    'e' => Some('\x1b'),
                    ' ' => Some(' '),
                    '"' => Some('"'),
                    '\\' => Some('\\'),
                    '/' => Some('/'),
                    'N' => Some('\u{85}'),
                    '_' => Some('\u{a0}'),
                    'L' => Some('\u{2028}'),
                    'P' => Some('\u{2029}'),
                    _ => None,
                };
                if let Some(c) = replacement {
                    chunks.push(c);
                    self.forward(1);
                } else if let Some(length) = match ch {
                    'x' => Some(2),
                    'u' => Some(4),
                    'U' => Some(8),
                    _ => None,
                } {
                    self.forward(1);
                    for k in 0..length {
                        if !self.peek(k).is_ascii_hexdigit() {
                            return Err(self.marked(
                                Some("while scanning a double-quoted scalar"),
                                Some(start),
                                Some(format!(
                                    "expected escape sequence of {length} hexadecimal numbers, but found {}",
                                    r(self.peek(k))
                                )),
                                Some(self.get_mark()),
                            ));
                        }
                    }
                    let code = u32::from_str_radix(&self.prefix(length), 16).expect("hex digits");
                    match char::from_u32(code) {
                        Some(c) => chunks.push(c),
                        // Python's chr() accepts a lone surrogate, which no
                        // later step of the builder can write out; chr()
                        // refuses anything above U+10FFFF.
                        None if (0xD800..=0xDFFF).contains(&code) => {
                            return Err(Error::Internal(format!(
                                "UnicodeEncodeError: a double-quoted escape produced the lone surrogate \\u{code:04x}"
                            )));
                        }
                        None => {
                            return Err(Error::Internal(
                                "ValueError: chr() arg not in range(0x110000)".into(),
                            ));
                        }
                    }
                    self.forward(length);
                } else if BREAKS.contains(ch) && ch != '\0' {
                    self.scan_line_break();
                    self.scan_flow_scalar_breaks(start, chunks)?;
                } else {
                    return Err(self.marked(
                        Some("while scanning a double-quoted scalar"),
                        Some(start),
                        Some(format!("found unknown escape character {}", r(ch))),
                        Some(self.get_mark()),
                    ));
                }
            } else {
                return Ok(());
            }
        }
    }

    fn scan_flow_scalar_spaces(
        &mut self,
        _double: bool,
        start: Mark,
        chunks: &mut String,
    ) -> Result<()> {
        let mut length = 0;
        while " \t".contains(self.peek(length)) && self.peek(length) != '\0' {
            length += 1;
        }
        let whitespaces = self.prefix(length);
        self.forward(length);
        let ch = self.peek(0);
        if ch == '\0' {
            return Err(self.marked(
                Some("while scanning a quoted scalar"),
                Some(start),
                Some("found unexpected end of stream".into()),
                Some(self.get_mark()),
            ));
        } else if BREAKS.contains(ch) {
            let line_break = self.scan_line_break();
            let mut breaks = String::new();
            let had = self.scan_flow_scalar_breaks(start, &mut breaks)?;
            if line_break != "\n" {
                chunks.push_str(&line_break);
            } else if !had {
                chunks.push(' ');
            }
            chunks.push_str(&breaks);
        } else {
            chunks.push_str(&whitespaces);
        }
        Ok(())
    }

    /// Returns whether any break was read.
    fn scan_flow_scalar_breaks(&mut self, start: Mark, chunks: &mut String) -> Result<bool> {
        let mut any = false;
        loop {
            let prefix = self.prefix(3);
            if (prefix == "---" || prefix == "...") && BLANK_Z.contains(self.peek(3)) {
                return Err(self.marked(
                    Some("while scanning a quoted scalar"),
                    Some(start),
                    Some("found unexpected document separator".into()),
                    Some(self.get_mark()),
                ));
            }
            while self.peek(0) == ' ' || self.peek(0) == '\t' {
                self.forward(1);
            }
            if BREAKS.contains(self.peek(0)) && self.peek(0) != '\0' {
                chunks.push_str(&self.scan_line_break());
                any = true;
            } else {
                return Ok(any);
            }
        }
    }

    fn scan_plain(&mut self) -> Result<Token> {
        let mut chunks = String::new();
        let start = self.get_mark();
        let mut end = start;
        let indent = self.indent + 1;
        let mut spaces: Option<String> = Some(String::new());
        loop {
            let mut length = 0;
            if self.peek(0) == '#' {
                break;
            }
            loop {
                let ch = self.peek(length);
                let next = self.peek(length + 1);
                if BLANK_Z.contains(ch)
                    || ch == ':'
                        && (BLANK_Z.contains(next) || self.flow_level > 0 && ",[]{}".contains(next))
                    || self.flow_level > 0 && ",?[]{}".contains(ch)
                {
                    break;
                }
                length += 1;
            }
            if length == 0 {
                break;
            }
            self.allow_simple_key = false;
            chunks.push_str(spaces.as_deref().unwrap_or(""));
            chunks.push_str(&self.prefix(length));
            self.forward(length);
            end = self.get_mark();
            spaces = self.scan_plain_spaces();
            match &spaces {
                None => break,
                Some(s) if s.is_empty() => break,
                Some(_) => {}
            }
            if self.peek(0) == '#' || self.flow_level == 0 && (self.column as i64) < indent {
                break;
            }
        }
        let mut token = Token::new(Kind::Scalar, start, end);
        token.value = chunks;
        token.plain = true;
        Ok(token)
    }

    /// The whitespace between plain scalar words; None ends the scalar at a document separator.
    fn scan_plain_spaces(&mut self) -> Option<String> {
        let mut chunks = String::new();
        let mut length = 0;
        while self.peek(length) == ' ' {
            length += 1;
        }
        let whitespaces = self.prefix(length);
        self.forward(length);
        let ch = self.peek(0);
        let separator = |me: &Self| {
            let prefix = me.prefix(3);
            (prefix == "---" || prefix == "...") && BLANK_Z.contains(me.peek(3))
        };
        if BREAKS.contains(ch) && ch != '\0' {
            let line_break = self.scan_line_break();
            self.allow_simple_key = true;
            if separator(self) {
                return None;
            }
            let mut breaks = String::new();
            let mut any = false;
            while " \r\n\u{85}\u{2028}\u{2029}".contains(self.peek(0)) && self.peek(0) != '\0' {
                if self.peek(0) == ' ' {
                    self.forward(1);
                } else {
                    breaks.push_str(&self.scan_line_break());
                    any = true;
                    if separator(self) {
                        return None;
                    }
                }
            }
            if line_break != "\n" {
                chunks.push_str(&line_break);
            } else if !any {
                chunks.push(' ');
            }
            chunks.push_str(&breaks);
        } else if !whitespaces.is_empty() {
            chunks.push_str(&whitespaces);
        }
        Some(chunks)
    }

    fn scan_tag_handle(&mut self, name: &str, start: Mark) -> Result<String> {
        let context = format!("while scanning a {name}");
        let ch = self.peek(0);
        if ch != '!' {
            return Err(self.marked(
                Some(&context),
                Some(start),
                Some(format!("expected '!', but found {}", r(ch))),
                Some(self.get_mark()),
            ));
        }
        let mut length = 1;
        let mut ch = self.peek(length);
        if ch != ' ' {
            while ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                length += 1;
                ch = self.peek(length);
            }
            if ch != '!' {
                self.forward(length);
                return Err(self.marked(
                    Some(&context),
                    Some(start),
                    Some(format!("expected '!', but found {}", r(ch))),
                    Some(self.get_mark()),
                ));
            }
            length += 1;
        }
        let value = self.prefix(length);
        self.forward(length);
        Ok(value)
    }

    fn scan_tag_uri(&mut self, name: &str, start: Mark) -> Result<String> {
        let mut chunks = String::new();
        let mut any = false;
        let mut length = 0;
        let mut ch = self.peek(length);
        while ch.is_ascii_alphanumeric() || "-;/?:@&=+$,_.!~*'()[]%".contains(ch) && ch != '\0' {
            if ch == '%' {
                chunks.push_str(&self.prefix(length));
                any = true;
                self.forward(length);
                length = 0;
                chunks.push_str(&self.scan_uri_escapes(name, start)?);
            } else {
                length += 1;
            }
            ch = self.peek(length);
        }
        if length > 0 {
            chunks.push_str(&self.prefix(length));
            any = true;
            self.forward(length);
        }
        if !any {
            return Err(self.marked(
                Some(&format!("while parsing a {name}")),
                Some(start),
                Some(format!("expected URI, but found {}", r(ch))),
                Some(self.get_mark()),
            ));
        }
        Ok(chunks)
    }

    fn scan_uri_escapes(&mut self, name: &str, start: Mark) -> Result<String> {
        let mut codes = Vec::new();
        let mark = self.get_mark();
        let context = format!("while scanning a {name}");
        while self.peek(0) == '%' {
            self.forward(1);
            for k in 0..2 {
                if !self.peek(k).is_ascii_hexdigit() {
                    return Err(self.marked(
                        Some(&context),
                        Some(start),
                        Some(format!(
                            "expected URI escape sequence of 2 hexadecimal numbers, but found {}",
                            r(self.peek(k))
                        )),
                        Some(self.get_mark()),
                    ));
                }
            }
            codes.push(u8::from_str_radix(&self.prefix(2), 16).expect("hex digits"));
            self.forward(2);
        }
        py::decode_utf8(&codes)
            .map_err(|e| self.marked(Some(&context), Some(start), Some(e.to_string()), Some(mark)))
    }

    fn scan_line_break(&mut self) -> String {
        let ch = self.peek(0);
        if ch == '\r' || ch == '\n' || ch == '\u{85}' {
            if self.prefix(2) == "\r\n" {
                self.forward(2);
            } else {
                self.forward(1);
            }
            return "\n".into();
        } else if ch == '\u{2028}' || ch == '\u{2029}' {
            self.forward(1);
            return ch.to_string();
        }
        String::new()
    }

    // Parser.

    fn check_event(&mut self, choices: &[EventKind]) -> Result<bool> {
        self.fill_event()?;
        Ok(self
            .current_event
            .as_ref()
            .is_some_and(|e| choices.is_empty() || choices.contains(&e.kind)))
    }

    fn fill_event(&mut self) -> Result<()> {
        if self.current_event.is_none()
            && let Some(state) = self.state
        {
            let event = self.run_state(state)?;
            self.current_event = Some(event);
        }
        Ok(())
    }

    fn peek_event(&mut self) -> Result<Event> {
        self.fill_event()?;
        Ok(self
            .current_event
            .clone()
            .expect("an event follows until the stream ends"))
    }

    fn get_event(&mut self) -> Result<Event> {
        self.fill_event()?;
        Ok(self
            .current_event
            .take()
            .expect("an event follows until the stream ends"))
    }

    fn pop_state(&mut self) -> Option<State> {
        self.states.pop()
    }

    fn run_state(&mut self, state: State) -> Result<Event> {
        match state {
            State::StreamStart => {
                let token = self.get_token()?;
                self.state = Some(State::ImplicitDocumentStart);
                Ok(Event::new(EventKind::StreamStart, token.start))
            }
            State::ImplicitDocumentStart => {
                if !self.check_token(&[Kind::Directive, Kind::DocumentStart, Kind::StreamEnd])? {
                    self.tag_handles = default_tags();
                    let token = self.peek_token()?;
                    self.states.push(State::DocumentEnd);
                    self.state = Some(State::BlockNode);
                    Ok(Event::new(EventKind::DocumentStart, token.start))
                } else {
                    self.parse_document_start()
                }
            }
            State::DocumentStart => self.parse_document_start(),
            State::DocumentEnd => {
                let token = self.peek_token()?;
                if self.check_token(&[Kind::DocumentEnd])? {
                    self.get_token()?;
                }
                self.state = Some(State::DocumentStart);
                Ok(Event::new(EventKind::DocumentEnd, token.start))
            }
            State::DocumentContent => {
                if self.check_token(&[
                    Kind::Directive,
                    Kind::DocumentStart,
                    Kind::DocumentEnd,
                    Kind::StreamEnd,
                ])? {
                    let mark = self.peek_token()?.start;
                    self.state = self.pop_state();
                    Ok(self.empty_scalar(mark))
                } else {
                    self.parse_node(true, false)
                }
            }
            State::BlockNode => self.parse_node(true, false),
            State::BlockSequenceFirstEntry => {
                let token = self.get_token()?;
                self.marks.push(token.start);
                self.parse_block_sequence_entry()
            }
            State::BlockSequenceEntry => self.parse_block_sequence_entry(),
            State::IndentlessSequenceEntry => {
                if self.check_token(&[Kind::BlockEntry])? {
                    let token = self.get_token()?;
                    if !self.check_token(&[
                        Kind::BlockEntry,
                        Kind::Key,
                        Kind::Value,
                        Kind::BlockEnd,
                    ])? {
                        self.states.push(State::IndentlessSequenceEntry);
                        return self.parse_node(true, false);
                    }
                    self.state = Some(State::IndentlessSequenceEntry);
                    return Ok(self.empty_scalar(token.end));
                }
                let token = self.peek_token()?;
                self.state = self.pop_state();
                Ok(Event::new(EventKind::SequenceEnd, token.start))
            }
            State::BlockMappingFirstKey => {
                let token = self.get_token()?;
                self.marks.push(token.start);
                self.parse_block_mapping_key()
            }
            State::BlockMappingKey => self.parse_block_mapping_key(),
            State::BlockMappingValue => {
                if self.check_token(&[Kind::Value])? {
                    let token = self.get_token()?;
                    if !self.check_token(&[Kind::Key, Kind::Value, Kind::BlockEnd])? {
                        self.states.push(State::BlockMappingKey);
                        return self.parse_node(true, true);
                    }
                    self.state = Some(State::BlockMappingKey);
                    return Ok(self.empty_scalar(token.end));
                }
                self.state = Some(State::BlockMappingKey);
                let token = self.peek_token()?;
                Ok(self.empty_scalar(token.start))
            }
            State::FlowSequenceFirstEntry => {
                let token = self.get_token()?;
                self.marks.push(token.start);
                self.parse_flow_sequence_entry(true)
            }
            State::FlowSequenceEntry => self.parse_flow_sequence_entry(false),
            State::FlowSequenceEntryMappingKey => {
                let token = self.get_token()?;
                if !self.check_token(&[Kind::Value, Kind::FlowEntry, Kind::FlowSequenceEnd])? {
                    self.states.push(State::FlowSequenceEntryMappingValue);
                    return self.parse_node(false, false);
                }
                self.state = Some(State::FlowSequenceEntryMappingValue);
                Ok(self.empty_scalar(token.end))
            }
            State::FlowSequenceEntryMappingValue => {
                if self.check_token(&[Kind::Value])? {
                    let token = self.get_token()?;
                    if !self.check_token(&[Kind::FlowEntry, Kind::FlowSequenceEnd])? {
                        self.states.push(State::FlowSequenceEntryMappingEnd);
                        return self.parse_node(false, false);
                    }
                    self.state = Some(State::FlowSequenceEntryMappingEnd);
                    return Ok(self.empty_scalar(token.end));
                }
                self.state = Some(State::FlowSequenceEntryMappingEnd);
                let token = self.peek_token()?;
                Ok(self.empty_scalar(token.start))
            }
            State::FlowSequenceEntryMappingEnd => {
                self.state = Some(State::FlowSequenceEntry);
                let token = self.peek_token()?;
                Ok(Event::new(EventKind::MappingEnd, token.start))
            }
            State::FlowMappingFirstKey => {
                let token = self.get_token()?;
                self.marks.push(token.start);
                self.parse_flow_mapping_key(true)
            }
            State::FlowMappingKey => self.parse_flow_mapping_key(false),
            State::FlowMappingValue => {
                if self.check_token(&[Kind::Value])? {
                    let token = self.get_token()?;
                    if !self.check_token(&[Kind::FlowEntry, Kind::FlowMappingEnd])? {
                        self.states.push(State::FlowMappingKey);
                        return self.parse_node(false, false);
                    }
                    self.state = Some(State::FlowMappingKey);
                    return Ok(self.empty_scalar(token.end));
                }
                self.state = Some(State::FlowMappingKey);
                let token = self.peek_token()?;
                Ok(self.empty_scalar(token.start))
            }
            State::FlowMappingEmptyValue => {
                self.state = Some(State::FlowMappingKey);
                let mark = self.peek_token()?.start;
                Ok(self.empty_scalar(mark))
            }
        }
    }

    fn parse_document_start(&mut self) -> Result<Event> {
        while self.check_token(&[Kind::DocumentEnd])? {
            self.get_token()?;
        }
        if !self.check_token(&[Kind::StreamEnd])? {
            let start = self.peek_token()?.start;
            self.process_directives()?;
            if !self.check_token(&[Kind::DocumentStart])? {
                let token = self.peek_token()?;
                return Err(self.marked(
                    None,
                    None,
                    Some(format!(
                        "expected '<document start>', but found {}",
                        py::repr_str(token.kind.id())
                    )),
                    Some(token.start),
                ));
            }
            self.get_token()?;
            self.states.push(State::DocumentEnd);
            self.state = Some(State::DocumentContent);
            Ok(Event::new(EventKind::DocumentStart, start))
        } else {
            let token = self.get_token()?;
            self.state = None;
            Ok(Event::new(EventKind::StreamEnd, token.start))
        }
    }

    fn process_directives(&mut self) -> Result<()> {
        let mut yaml_version = false;
        self.tag_handles = Vec::new();
        while self.check_token(&[Kind::Directive])? {
            let token = self.get_token()?;
            match &token.directive {
                DirectiveValue::Yaml(major) if token.name == "YAML" => {
                    if yaml_version {
                        return Err(self.marked(
                            None,
                            None,
                            Some("found duplicate YAML directive".into()),
                            Some(token.start),
                        ));
                    }
                    if *major != BigInt::from(1) {
                        return Err(self.marked(
                            None,
                            None,
                            Some(
                                "found incompatible YAML document (version 1.* is required)".into(),
                            ),
                            Some(token.start),
                        ));
                    }
                    yaml_version = true;
                }
                DirectiveValue::Tag(handle, prefix) if token.name == "TAG" => {
                    if self.tag_handles.iter().any(|(h, _)| h == handle) {
                        return Err(self.marked(
                            None,
                            None,
                            Some(format!("duplicate tag handle {}", py::repr_str(handle))),
                            Some(token.start),
                        ));
                    }
                    self.tag_handles.push((handle.clone(), prefix.clone()));
                }
                _ => {}
            }
        }
        for (key, value) in default_tags() {
            if !self.tag_handles.iter().any(|(h, _)| *h == key) {
                self.tag_handles.push((key, value));
            }
        }
        Ok(())
    }

    fn parse_node(&mut self, block: bool, indentless_sequence: bool) -> Result<Event> {
        if self.check_token(&[Kind::Alias])? {
            let token = self.get_token()?;
            let mut event = Event::new(EventKind::Alias, token.start);
            event.anchor = Some(token.value);
            self.state = self.pop_state();
            return Ok(event);
        }
        let mut anchor = None;
        let mut tag_token: Option<(Option<String>, String)> = None;
        let mut start = None;
        let mut tag_mark = None;
        if self.check_token(&[Kind::Anchor])? {
            let token = self.get_token()?;
            start = Some(token.start);
            anchor = Some(token.value);
            if self.check_token(&[Kind::Tag])? {
                let token = self.get_token()?;
                tag_mark = Some(token.start);
                tag_token = Some((token.handle, token.value));
            }
        } else if self.check_token(&[Kind::Tag])? {
            let token = self.get_token()?;
            start = Some(token.start);
            tag_mark = Some(token.start);
            tag_token = Some((token.handle, token.value));
            if self.check_token(&[Kind::Anchor])? {
                let token = self.get_token()?;
                anchor = Some(token.value);
            }
        }
        let mut tag = None;
        if let Some((handle, suffix)) = tag_token {
            match handle {
                Some(handle) => {
                    let Some(prefix) = self
                        .tag_handles
                        .iter()
                        .find(|(h, _)| *h == handle)
                        .map(|(_, p)| p.clone())
                    else {
                        return Err(self.marked(
                            Some("while parsing a node"),
                            start,
                            Some(format!(
                                "found undefined tag handle {}",
                                py::repr_str(&handle)
                            )),
                            tag_mark,
                        ));
                    };
                    tag = Some(format!("{prefix}{suffix}"));
                }
                None => tag = Some(suffix),
            }
        }
        let start = match start {
            Some(s) => s,
            None => self.peek_token()?.start,
        };
        let implicit = tag.is_none() || tag.as_deref() == Some("!");
        let mut event;
        if indentless_sequence && self.check_token(&[Kind::BlockEntry])? {
            event = Event::new(EventKind::SequenceStart, start);
            self.state = Some(State::IndentlessSequenceEntry);
        } else if self.check_token(&[Kind::Scalar])? {
            let token = self.get_token()?;
            event = Event::new(EventKind::Scalar, start);
            event.implicit = if token.plain && tag.is_none() || tag.as_deref() == Some("!") {
                (true, false)
            } else if tag.is_none() {
                (false, true)
            } else {
                (false, false)
            };
            event.value = token.value;
            self.state = self.pop_state();
        } else if self.check_token(&[Kind::FlowSequenceStart])? {
            event = Event::new(EventKind::SequenceStart, start);
            self.state = Some(State::FlowSequenceFirstEntry);
        } else if self.check_token(&[Kind::FlowMappingStart])? {
            event = Event::new(EventKind::MappingStart, start);
            self.state = Some(State::FlowMappingFirstKey);
        } else if block && self.check_token(&[Kind::BlockSequenceStart])? {
            event = Event::new(EventKind::SequenceStart, start);
            self.state = Some(State::BlockSequenceFirstEntry);
        } else if block && self.check_token(&[Kind::BlockMappingStart])? {
            event = Event::new(EventKind::MappingStart, start);
            self.state = Some(State::BlockMappingFirstKey);
        } else if anchor.is_some() || tag.is_some() {
            event = Event::new(EventKind::Scalar, start);
            event.implicit = (implicit, false);
            self.state = self.pop_state();
        } else {
            let node = if block { "block" } else { "flow" };
            let token = self.peek_token()?;
            return Err(self.marked(
                Some(&format!("while parsing a {node} node")),
                Some(start),
                Some(format!(
                    "expected the node content, but found {}",
                    py::repr_str(token.kind.id())
                )),
                Some(token.start),
            ));
        }
        if matches!(
            event.kind,
            EventKind::SequenceStart | EventKind::MappingStart
        ) {
            event.implicit = (implicit, false);
        }
        event.anchor = anchor;
        event.tag = tag;
        Ok(event)
    }

    fn parse_block_sequence_entry(&mut self) -> Result<Event> {
        if self.check_token(&[Kind::BlockEntry])? {
            let token = self.get_token()?;
            if !self.check_token(&[Kind::BlockEntry, Kind::BlockEnd])? {
                self.states.push(State::BlockSequenceEntry);
                return self.parse_node(true, false);
            }
            self.state = Some(State::BlockSequenceEntry);
            return Ok(self.empty_scalar(token.end));
        }
        if !self.check_token(&[Kind::BlockEnd])? {
            let token = self.peek_token()?;
            return Err(self.marked(
                Some("while parsing a block collection"),
                self.marks.last().copied(),
                Some(format!(
                    "expected <block end>, but found {}",
                    py::repr_str(token.kind.id())
                )),
                Some(token.start),
            ));
        }
        let token = self.get_token()?;
        self.state = self.pop_state();
        self.marks.pop();
        Ok(Event::new(EventKind::SequenceEnd, token.start))
    }

    fn parse_block_mapping_key(&mut self) -> Result<Event> {
        if self.check_token(&[Kind::Key])? {
            let token = self.get_token()?;
            if !self.check_token(&[Kind::Key, Kind::Value, Kind::BlockEnd])? {
                self.states.push(State::BlockMappingValue);
                return self.parse_node(true, true);
            }
            self.state = Some(State::BlockMappingValue);
            return Ok(self.empty_scalar(token.end));
        }
        if !self.check_token(&[Kind::BlockEnd])? {
            let token = self.peek_token()?;
            return Err(self.marked(
                Some("while parsing a block mapping"),
                self.marks.last().copied(),
                Some(format!(
                    "expected <block end>, but found {}",
                    py::repr_str(token.kind.id())
                )),
                Some(token.start),
            ));
        }
        let token = self.get_token()?;
        self.state = self.pop_state();
        self.marks.pop();
        Ok(Event::new(EventKind::MappingEnd, token.start))
    }

    fn parse_flow_sequence_entry(&mut self, first: bool) -> Result<Event> {
        if !self.check_token(&[Kind::FlowSequenceEnd])? {
            if !first {
                if self.check_token(&[Kind::FlowEntry])? {
                    self.get_token()?;
                } else {
                    let token = self.peek_token()?;
                    return Err(self.marked(
                        Some("while parsing a flow sequence"),
                        self.marks.last().copied(),
                        Some(format!(
                            "expected ',' or ']', but got {}",
                            py::repr_str(token.kind.id())
                        )),
                        Some(token.start),
                    ));
                }
            }
            if self.check_token(&[Kind::Key])? {
                let token = self.peek_token()?;
                let mut event = Event::new(EventKind::MappingStart, token.start);
                event.implicit = (true, false);
                self.state = Some(State::FlowSequenceEntryMappingKey);
                return Ok(event);
            } else if !self.check_token(&[Kind::FlowSequenceEnd])? {
                self.states.push(State::FlowSequenceEntry);
                return self.parse_node(false, false);
            }
        }
        let token = self.get_token()?;
        self.state = self.pop_state();
        self.marks.pop();
        Ok(Event::new(EventKind::SequenceEnd, token.start))
    }

    fn parse_flow_mapping_key(&mut self, first: bool) -> Result<Event> {
        if !self.check_token(&[Kind::FlowMappingEnd])? {
            if !first {
                if self.check_token(&[Kind::FlowEntry])? {
                    self.get_token()?;
                } else {
                    let token = self.peek_token()?;
                    return Err(self.marked(
                        Some("while parsing a flow mapping"),
                        self.marks.last().copied(),
                        Some(format!(
                            "expected ',' or '}}', but got {}",
                            py::repr_str(token.kind.id())
                        )),
                        Some(token.start),
                    ));
                }
            }
            if self.check_token(&[Kind::Key])? {
                let token = self.get_token()?;
                if !self.check_token(&[Kind::Value, Kind::FlowEntry, Kind::FlowMappingEnd])? {
                    self.states.push(State::FlowMappingValue);
                    return self.parse_node(false, false);
                }
                self.state = Some(State::FlowMappingValue);
                return Ok(self.empty_scalar(token.end));
            } else if !self.check_token(&[Kind::FlowMappingEnd])? {
                self.states.push(State::FlowMappingEmptyValue);
                return self.parse_node(false, false);
            }
        }
        let token = self.get_token()?;
        self.state = self.pop_state();
        self.marks.pop();
        Ok(Event::new(EventKind::MappingEnd, token.start))
    }

    fn empty_scalar(&self, mark: Mark) -> Event {
        let mut event = Event::new(EventKind::Scalar, mark);
        event.implicit = (true, false);
        event
    }

    // Composer.

    fn get_single_node(&mut self) -> Result<Option<usize>> {
        self.get_event()?;
        let mut document = None;
        if !self.check_event(&[EventKind::StreamEnd])? {
            document = Some(self.compose_document()?);
        }
        if !self.check_event(&[EventKind::StreamEnd])? {
            let event = self.get_event()?;
            let start = document.map(|d| self.nodes[d].start);
            return Err(self.marked(
                Some("expected a single document in the stream"),
                start,
                Some("but found another document".into()),
                Some(event.start),
            ));
        }
        self.get_event()?;
        Ok(document)
    }

    fn compose_document(&mut self) -> Result<usize> {
        self.get_event()?;
        let node = self.compose_node()?;
        self.get_event()?;
        self.anchors.clear();
        Ok(node)
    }

    fn compose_node(&mut self) -> Result<usize> {
        if self.check_event(&[EventKind::Alias])? {
            let event = self.get_event()?;
            let anchor = event.anchor.expect("an alias names its anchor");
            return match self.anchors.get(&anchor) {
                Some(&node) => Ok(node),
                None => Err(self.marked(
                    None,
                    None,
                    Some(format!("found undefined alias {}", py::repr_str(&anchor))),
                    Some(event.start),
                )),
            };
        }
        let event = self.peek_event()?;
        if let Some(anchor) = &event.anchor
            && let Some(&first) = self.anchors.get(anchor)
        {
            return Err(self.marked(
                Some(&format!(
                    "found duplicate anchor {}; first occurrence",
                    py::repr_str(anchor)
                )),
                Some(self.nodes[first].start),
                Some("second occurrence".into()),
                Some(event.start),
            ));
        }
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(Error::TooDeep);
        }
        let node = match event.kind {
            EventKind::Scalar => {
                let event = self.get_event()?;
                let tag = match event.tag.as_deref() {
                    None | Some("!") => resolve_scalar(&event.value, event.implicit),
                    Some(t) => t.to_string(),
                };
                self.add_node(
                    Node {
                        tag,
                        value: NodeValue::Scalar(event.value),
                        start: event.start,
                    },
                    event.anchor,
                )
            }
            EventKind::SequenceStart => {
                let start = self.get_event()?;
                let tag = match start.tag.as_deref() {
                    None | Some("!") => "tag:yaml.org,2002:seq".to_string(),
                    Some(t) => t.to_string(),
                };
                let id = self.add_node(
                    Node {
                        tag,
                        value: NodeValue::Sequence(Vec::new()),
                        start: start.start,
                    },
                    start.anchor,
                );
                while !self.check_event(&[EventKind::SequenceEnd])? {
                    let child = self.compose_node()?;
                    if let NodeValue::Sequence(items) = &mut self.nodes[id].value {
                        items.push(child);
                    }
                }
                self.get_event()?;
                id
            }
            kind => {
                assert_eq!(
                    kind,
                    EventKind::MappingStart,
                    "the parser yields a node event here"
                );
                let start = self.get_event()?;
                let tag = match start.tag.as_deref() {
                    None | Some("!") => "tag:yaml.org,2002:map".to_string(),
                    Some(t) => t.to_string(),
                };
                let id = self.add_node(
                    Node {
                        tag,
                        value: NodeValue::Mapping(Vec::new()),
                        start: start.start,
                    },
                    start.anchor,
                );
                while !self.check_event(&[EventKind::MappingEnd])? {
                    let key = self.compose_node()?;
                    let value = self.compose_node()?;
                    if let NodeValue::Mapping(pairs) = &mut self.nodes[id].value {
                        pairs.push((key, value));
                    }
                }
                self.get_event()?;
                id
            }
        };
        self.depth -= 1;
        Ok(node)
    }

    fn add_node(&mut self, node: Node, anchor: Option<String>) -> usize {
        let id = self.nodes.len();
        self.nodes.push(node);
        if let Some(anchor) = anchor {
            self.anchors.insert(anchor, id);
        }
        id
    }

    // Constructor.

    fn get_single_data(&mut self) -> Result<Value> {
        match self.get_single_node()? {
            Some(node) => self.construct_document(node),
            None => Ok(Value::None),
        }
    }

    fn construct_document(&mut self, node: usize) -> Result<Value> {
        let data = self.construct_object(node)?;
        while !self.pending.is_empty() {
            let pending = std::mem::take(&mut self.pending);
            for item in pending {
                self.run_pending(item)?;
            }
        }
        Ok(data)
    }

    fn construct_object(&mut self, node: usize) -> Result<Value> {
        if let Some(value) = self.constructed.get(&node) {
            return Ok(value.clone());
        }
        if self.recursive.contains(&node) {
            return Err(self.marked(
                None,
                None,
                Some("found unconstructable recursive node".into()),
                Some(self.nodes[node].start),
            ));
        }
        self.recursive.insert(node);
        let tag = self.nodes[node].tag.clone();
        let data = match tag.as_str() {
            "tag:yaml.org,2002:null" => {
                self.construct_scalar(node)?;
                Value::None
            }
            "tag:yaml.org,2002:bool" => {
                let value = self.construct_scalar(node)?;
                match value.to_lowercase().as_str() {
                    "yes" | "true" | "on" => Value::Bool(true),
                    "no" | "false" | "off" => Value::Bool(false),
                    _ => {
                        return Err(Error::Internal(format!(
                            "KeyError: {}",
                            py::repr_str(&value.to_lowercase())
                        )));
                    }
                }
            }
            "tag:yaml.org,2002:int" => Value::Int(self.construct_int(node)?),
            "tag:yaml.org,2002:float" => {
                // PyYAML returns one shared object for every `.nan`, so those
                // compare equal by identity; any other float is its own object.
                let (value, shared_nan) = self.construct_float(node)?;
                Value::Float(value, if shared_nan { usize::MAX } else { node })
            }
            "tag:yaml.org,2002:binary" => Value::Bytes(self.construct_binary(node)?),
            "tag:yaml.org,2002:timestamp" => self.construct_timestamp(node)?,
            "tag:yaml.org,2002:omap" | "tag:yaml.org,2002:pairs" => {
                let list = Rc::new(RefCell::new(Vec::new()));
                let what = if tag.ends_with("omap") {
                    "an ordered map"
                } else {
                    "pairs"
                };
                self.pending.push(Pending::Omap(node, list.clone(), what));
                Value::List(list)
            }
            "tag:yaml.org,2002:set" => {
                let set = Rc::new(RefCell::new(Vec::new()));
                self.pending.push(Pending::Set(node, set.clone()));
                Value::Set(set)
            }
            "tag:yaml.org,2002:str" => Value::Str(self.construct_scalar(node)?),
            "tag:yaml.org,2002:seq" => {
                let list = Rc::new(RefCell::new(Vec::new()));
                self.pending.push(Pending::Seq(node, list.clone()));
                Value::List(list)
            }
            "tag:yaml.org,2002:map" => self.construct_unique_mapping(node)?,
            _ => {
                return Err(self.marked(
                    None,
                    None,
                    Some(format!(
                        "could not determine a constructor for the tag {}",
                        py::repr_str(&tag)
                    )),
                    Some(self.nodes[node].start),
                ));
            }
        };
        self.constructed.insert(node, data.clone());
        self.recursive.remove(&node);
        Ok(data)
    }

    fn run_pending(&mut self, item: Pending) -> Result<()> {
        match item {
            Pending::Seq(node, list) => {
                let NodeValue::Sequence(children) = self.nodes[node].value.clone() else {
                    return Err(self.marked(
                        None,
                        None,
                        Some(format!(
                            "expected a sequence node, but found {}",
                            self.nodes[node].id()
                        )),
                        Some(self.nodes[node].start),
                    ));
                };
                let mut items = Vec::new();
                for child in children {
                    items.push(self.construct_object(child)?);
                }
                list.borrow_mut().extend(items);
            }
            Pending::Omap(node, list, what) => {
                let context = format!("while constructing {what}");
                let start = self.nodes[node].start;
                let NodeValue::Sequence(children) = self.nodes[node].value.clone() else {
                    return Err(self.marked(
                        Some(&context),
                        Some(start),
                        Some(format!(
                            "expected a sequence, but found {}",
                            self.nodes[node].id()
                        )),
                        Some(start),
                    ));
                };
                for sub in children {
                    let NodeValue::Mapping(pairs) = self.nodes[sub].value.clone() else {
                        return Err(self.marked(
                            Some(&context),
                            Some(start),
                            Some(format!(
                                "expected a mapping of length 1, but found {}",
                                self.nodes[sub].id()
                            )),
                            Some(self.nodes[sub].start),
                        ));
                    };
                    if pairs.len() != 1 {
                        return Err(self.marked(
                            Some(&context),
                            Some(start),
                            Some(format!(
                                "expected a single mapping item, but found {} items",
                                pairs.len()
                            )),
                            Some(self.nodes[sub].start),
                        ));
                    }
                    let key = self.construct_object(pairs[0].0)?;
                    let value = self.construct_object(pairs[0].1)?;
                    list.borrow_mut().push(Value::Tuple(vec![key, value]));
                }
            }
            Pending::Set(node, set) => {
                let pairs = self.construct_plain_mapping(node)?;
                set.borrow_mut().extend(pairs.into_iter().map(|(k, _)| k));
            }
        }
        Ok(())
    }

    fn construct_scalar(&self, node: usize) -> Result<String> {
        match &self.nodes[node].value {
            NodeValue::Scalar(s) => Ok(s.clone()),
            NodeValue::Mapping(pairs) => {
                for &(k, v) in pairs {
                    if self.nodes[k].tag == "tag:yaml.org,2002:value" {
                        return self.construct_scalar(v);
                    }
                }
                Err(self.marked(
                    None,
                    None,
                    Some("expected a scalar node, but found mapping".into()),
                    Some(self.nodes[node].start),
                ))
            }
            NodeValue::Sequence(_) => Err(self.marked(
                None,
                None,
                Some("expected a scalar node, but found sequence".into()),
                Some(self.nodes[node].start),
            )),
        }
    }

    fn construct_int(&self, node: usize) -> Result<BigInt> {
        let raw = self.construct_scalar(node)?;
        let mut value: String = raw.replace('_', "");
        let index_error = || Error::Internal("IndexError: string index out of range".into());
        let first = value.chars().next().ok_or_else(index_error)?;
        let negative = first == '-';
        if first == '+' || first == '-' {
            value.remove(0);
        }
        let internal = |e: String| Error::Internal(format!("ValueError: {e}"));
        let magnitude = if value == "0" {
            BigInt::zero()
        } else if let Some(rest) = value.strip_prefix("0b") {
            py::parse_int(rest, 2).map_err(internal)?
        } else if let Some(rest) = value.strip_prefix("0x") {
            py::parse_int(rest, 16).map_err(internal)?
        } else if value.chars().next().ok_or_else(index_error)? == '0' {
            py::parse_int(&value, 8).map_err(internal)?
        } else if value.contains(':') {
            let mut total = BigInt::zero();
            let mut base = BigInt::from(1);
            for part in value.split(':').rev() {
                total += py::parse_int(part, 10).map_err(internal)? * &base;
                base *= 60;
            }
            total
        } else {
            py::parse_int(&value, 10).map_err(internal)?
        };
        Ok(if negative { -magnitude } else { magnitude })
    }

    fn construct_float(&self, node: usize) -> Result<(f64, bool)> {
        let raw = self.construct_scalar(node)?;
        let mut value = raw.replace('_', "").to_lowercase();
        let first = value
            .chars()
            .next()
            .ok_or_else(|| Error::Internal("IndexError: string index out of range".into()))?;
        let sign = if first == '-' { -1.0 } else { 1.0 };
        if first == '+' || first == '-' {
            value.remove(0);
        }
        let internal = |e: String| Error::Internal(format!("ValueError: {e}"));
        if value == ".nan" {
            return Ok((f64::NAN, true));
        }
        let magnitude = if value == ".inf" {
            f64::INFINITY
        } else if value.contains(':') {
            let mut total = 0.0;
            let mut base = 1.0;
            for part in value.split(':').rev() {
                total += py::parse_float(part).map_err(internal)? * base;
                base *= 60.0;
            }
            total
        } else {
            py::parse_float(&value).map_err(internal)?
        };
        Ok((sign * magnitude, false))
    }

    fn construct_binary(&self, node: usize) -> Result<Vec<u8>> {
        let value = self.construct_scalar(node)?;
        let start = self.nodes[node].start;
        if let Some(position) = value.chars().position(|c| !c.is_ascii()) {
            let chars: Vec<char> = value.chars().collect();
            let mut end = position;
            while end < chars.len() && !chars[end].is_ascii() {
                end += 1;
            }
            let reason = if end == position + 1 {
                let c = chars[position] as u32;
                let shown = if c <= 0xff {
                    format!("\\x{c:02x}")
                } else if c <= 0xffff {
                    format!("\\u{c:04x}")
                } else {
                    format!("\\U{c:08x}")
                };
                format!(
                    "'ascii' codec can't encode character '{shown}' in position {position}: ordinal not in range(128)"
                )
            } else {
                format!(
                    "'ascii' codec can't encode characters in position {position}-{}: ordinal not in range(128)",
                    end - 1
                )
            };
            return Err(self.marked(
                None,
                None,
                Some(format!(
                    "failed to convert base64 data into ascii: {reason}"
                )),
                Some(start),
            ));
        }
        base64_decode(value.as_bytes()).map_err(|e| {
            self.marked(
                None,
                None,
                Some(format!("failed to decode base64 data: {e}")),
                Some(start),
            )
        })
    }

    fn construct_timestamp(&self, node: usize) -> Result<Value> {
        self.construct_scalar(node)?;
        let NodeValue::Scalar(text) = &self.nodes[node].value else {
            return Err(Error::Internal(
                "TypeError: expected string or bytes-like object, got 'list'".into(),
            ));
        };
        let Some(caps) = TIMESTAMP.captures(text) else {
            return Err(Error::Internal(
                "AttributeError: 'NoneType' object has no attribute 'groupdict'".into(),
            ));
        };
        let num = |name: &str| {
            caps.name(name)
                .map(|m| m.as_str().parse::<i64>().expect("ASCII digits"))
        };
        let internal = |e: String| Error::Internal(format!("ValueError: {e}"));
        let date = Date::new(
            num("year").unwrap(),
            num("month").unwrap(),
            num("day").unwrap(),
        );
        if caps.name("hour").is_none_or(|m| m.as_str().is_empty()) {
            return date.map(Value::Date).map_err(internal);
        }
        let hour = num("hour").unwrap();
        let minute = num("minute").unwrap();
        let second = num("second").unwrap();
        let mut microsecond = 0;
        if let Some(f) = caps
            .name("fraction")
            .map(|m| m.as_str())
            .filter(|f| !f.is_empty())
        {
            let mut digits: String = f.chars().take(6).collect();
            while digits.len() < 6 {
                digits.push('0');
            }
            microsecond = digits.parse::<u32>().expect("ASCII digits");
        }
        let tz = if let Some(sign) = caps.name("tz_sign") {
            let seconds = num("tz_hour").unwrap() * 3600 + num("tz_minute").unwrap_or(0) * 60;
            let seconds = if sign.as_str() == "-" {
                -seconds
            } else {
                seconds
            };
            if seconds.abs() >= 86_400 {
                return Err(internal(
                    "offset must be a timedelta strictly between -timedelta(hours=24) and timedelta(hours=24).".into(),
                ));
            }
            Some(if seconds == 0 {
                Tz::Utc
            } else {
                Tz::Offset(seconds)
            })
        } else if caps.name("tz").is_some() {
            Some(Tz::Utc)
        } else {
            None
        };
        // datetime() checks the date before the time.
        let date = date.map_err(internal)?;
        if !(0..24).contains(&hour) {
            return Err(internal("hour must be in 0..23".into()));
        }
        if !(0..60).contains(&minute) {
            return Err(internal("minute must be in 0..59".into()));
        }
        if !(0..60).contains(&second) {
            return Err(internal("second must be in 0..59".into()));
        }
        Ok(Value::DateTime(DateTime {
            date,
            hour: hour as u32,
            minute: minute as u32,
            second: second as u32,
            microsecond,
            tz,
        }))
    }

    fn flatten_mapping(&mut self, node: usize) -> Result<()> {
        let mut merge: Vec<(usize, usize)> = Vec::new();
        let mut index = 0;
        loop {
            let NodeValue::Mapping(pairs) = &self.nodes[node].value else {
                return Ok(());
            };
            if index >= pairs.len() {
                break;
            }
            let (key, value) = pairs[index];
            let key_tag = self.nodes[key].tag.clone();
            if key_tag == "tag:yaml.org,2002:merge" {
                if let NodeValue::Mapping(pairs) = &mut self.nodes[node].value {
                    pairs.remove(index);
                }
                match self.nodes[value].value.clone() {
                    NodeValue::Mapping(_) => {
                        self.flatten_mapping(value)?;
                        if let NodeValue::Mapping(sub) = &self.nodes[value].value {
                            merge.extend(sub.iter().copied());
                        }
                    }
                    NodeValue::Sequence(subnodes) => {
                        let mut submerge = Vec::new();
                        for sub in subnodes {
                            if !matches!(self.nodes[sub].value, NodeValue::Mapping(_)) {
                                return Err(self.marked(
                                    Some("while constructing a mapping"),
                                    Some(self.nodes[node].start),
                                    Some(format!(
                                        "expected a mapping for merging, but found {}",
                                        self.nodes[sub].id()
                                    )),
                                    Some(self.nodes[sub].start),
                                ));
                            }
                            self.flatten_mapping(sub)?;
                            if let NodeValue::Mapping(pairs) = &self.nodes[sub].value {
                                submerge.push(pairs.clone());
                            }
                        }
                        for pairs in submerge.into_iter().rev() {
                            merge.extend(pairs);
                        }
                    }
                    NodeValue::Scalar(_) => {
                        return Err(self.marked(
                            Some("while constructing a mapping"),
                            Some(self.nodes[node].start),
                            Some(format!(
                                "expected a mapping or list of mappings for merging, but found {}",
                                self.nodes[value].id()
                            )),
                            Some(self.nodes[value].start),
                        ));
                    }
                }
            } else if key_tag == "tag:yaml.org,2002:value" {
                self.nodes[key].tag = "tag:yaml.org,2002:str".into();
                index += 1;
            } else {
                index += 1;
            }
        }
        if !merge.is_empty()
            && let NodeValue::Mapping(pairs) = &mut self.nodes[node].value
        {
            merge.append(pairs);
            *pairs = merge;
        }
        Ok(())
    }

    /// The builder's mapping constructor: SafeConstructor's, refusing a repeated key.
    fn construct_unique_mapping(&mut self, node: usize) -> Result<Value> {
        // Flattening leaves anything but a mapping node as it is.
        self.flatten_mapping(node)?;
        let NodeValue::Mapping(pairs) = self.nodes[node].value.clone() else {
            return Err(self.marked(
                None,
                None,
                Some(format!(
                    "expected a mapping node, but found {}",
                    self.nodes[node].id()
                )),
                Some(self.nodes[node].start),
            ));
        };
        let mut mapping: Vec<(Value, Value)> = Vec::new();
        for (key_node, value_node) in pairs {
            let key = self.construct_object(key_node)?;
            if !key.hashable() {
                return Err(self.marked(
                    Some("while constructing a mapping"),
                    Some(self.nodes[node].start),
                    Some("found unhashable key".into()),
                    Some(self.nodes[key_node].start),
                ));
            }
            if mapping.iter().any(|(k, _)| k.py_eq(&key)) {
                let shown = key
                    .repr()
                    .map_err(|e| Error::Internal(format!("ValueError: {e}")))?;
                return Err(self.marked(
                    Some("while constructing a mapping"),
                    Some(self.nodes[node].start),
                    Some(format!("found duplicate key {shown}")),
                    Some(self.nodes[key_node].start),
                ));
            }
            let value = self.construct_object(value_node)?;
            mapping.push((key, value));
        }
        Ok(Value::Dict(Rc::new(RefCell::new(mapping))))
    }

    /// SafeConstructor.construct_mapping, which `!!set` uses: a later key replaces an earlier value.
    fn construct_plain_mapping(&mut self, node: usize) -> Result<Vec<(Value, Value)>> {
        if matches!(self.nodes[node].value, NodeValue::Mapping(_)) {
            self.flatten_mapping(node)?;
        }
        let NodeValue::Mapping(pairs) = self.nodes[node].value.clone() else {
            return Err(self.marked(
                None,
                None,
                Some(format!(
                    "expected a mapping node, but found {}",
                    self.nodes[node].id()
                )),
                Some(self.nodes[node].start),
            ));
        };
        let mut mapping: Vec<(Value, Value)> = Vec::new();
        for (key_node, value_node) in pairs {
            let key = self.construct_object(key_node)?;
            if !key.hashable() {
                return Err(self.marked(
                    Some("while constructing a mapping"),
                    Some(self.nodes[node].start),
                    Some("found unhashable key".into()),
                    Some(self.nodes[key_node].start),
                ));
            }
            let value = self.construct_object(value_node)?;
            match mapping.iter_mut().find(|(k, _)| k.py_eq(&key)) {
                Some(slot) => slot.1 = value,
                None => mapping.push((key, value)),
            }
        }
        Ok(mapping)
    }
}

/// `binascii.a2b_base64` without strict mode, as `base64.decodebytes` calls it.
fn base64_decode(data: &[u8]) -> std::result::Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut quad = 0;
    let mut left = 0u32;
    let mut pads = 0;
    // Padding that completes a group ends the data only when nothing follows:
    // CPython decodes later data characters as if the padding were absent.
    let mut padded = false;
    for &ch in data {
        if ch == b'=' {
            pads += 1;
            padded |= quad >= 2 && quad + pads >= 4;
            continue;
        }
        let value = match ch {
            b'A'..=b'Z' => ch - b'A',
            b'a'..=b'z' => ch - b'a' + 26,
            b'0'..=b'9' => ch - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => continue,
        } as u32;
        pads = 0;
        padded = false;
        match quad {
            0 => {
                quad = 1;
                left = value;
            }
            1 => {
                quad = 2;
                out.push(((left << 2) | (value >> 4)) as u8);
                left = value & 0x0f;
            }
            2 => {
                quad = 3;
                out.push(((left << 4) | (value >> 2)) as u8);
                left = value & 0x03;
            }
            _ => {
                quad = 0;
                out.push(((left << 6) | value) as u8);
                left = 0;
            }
        }
    }
    match quad {
        0 => Ok(out),
        _ if padded => Ok(out),
        1 => Err(format!(
            "Invalid base64-encoded string: number of data characters ({}) cannot be 1 more than a multiple of 4",
            out.len() / 3 * 4 + 1
        )),
        _ => Err("Incorrect padding".into()),
    }
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a valid built-in pattern")
}

// The resolver's patterns. Python's `$` also matches before a final newline.
static BOOL: LazyLock<Regex> = LazyLock::new(|| {
    re(r"^(?:yes|Yes|YES|no|No|NO|true|True|TRUE|false|False|FALSE|on|On|ON|off|Off|OFF)\n?\z")
});
static FLOAT: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^(?:[-+]?(?:[0-9][0-9_]*)\.[0-9_]*(?:[eE][-+][0-9]+)?|\.[0-9][0-9_]*(?:[eE][-+][0-9]+)?|[-+]?[0-9][0-9_]*(?::[0-5]?[0-9])+\.[0-9_]*|[-+]?\.(?:inf|Inf|INF)|\.(?:nan|NaN|NAN))\n?\z",
    )
});
static INT: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^(?:[-+]?0b[0-1_]+|[-+]?0[0-7_]+|[-+]?(?:0|[1-9][0-9_]*)|[-+]?0x[0-9a-fA-F_]+|[-+]?[1-9][0-9_]*(?::[0-5]?[0-9])+)\n?\z",
    )
});
static MERGE: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:<<)\n?\z"));
static NULL: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:~|null|Null|NULL|)\n?\z"));
static TIMESTAMP_IMPLICIT: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^(?:[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]|[0-9][0-9][0-9][0-9]-[0-9][0-9]?-[0-9][0-9]?(?:[Tt]|[ \t]+)[0-9][0-9]?:[0-9][0-9]:[0-9][0-9](?:\.[0-9]*)?(?:[ \t]*(?:Z|[-+][0-9][0-9]?(?::[0-9][0-9])?))?)\n?\z",
    )
});
static VALUE: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:=)\n?\z"));
static YAML: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:!|&|\*)\n?\z"));
static TIMESTAMP: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^(?P<year>[0-9][0-9][0-9][0-9])-(?P<month>[0-9][0-9]?)-(?P<day>[0-9][0-9]?)(?:(?:[Tt]|[ \t]+)(?P<hour>[0-9][0-9]?):(?P<minute>[0-9][0-9]):(?P<second>[0-9][0-9])(?:\.(?P<fraction>[0-9]*))?(?:[ \t]*(?P<tz>Z|(?P<tz_sign>[-+])(?P<tz_hour>[0-9][0-9]?)(?::(?P<tz_minute>[0-9][0-9]))?))?)?\n?\z",
    )
});

/// Resolver.resolve for a scalar node.
fn resolve_scalar(value: &str, implicit: (bool, bool)) -> String {
    if implicit.0 {
        let first = value.chars().next();
        let candidates: &[(&str, &LazyLock<Regex>)] = match first {
            None => &[("null", &NULL)],
            Some(c) => match c {
                'y' | 'Y' | 'n' | 'N' | 't' | 'T' | 'f' | 'F' | 'o' | 'O' => {
                    if c == 'n' || c == 'N' {
                        &[("bool", &BOOL), ("null", &NULL)]
                    } else {
                        &[("bool", &BOOL)]
                    }
                }
                '-' | '+' => &[("float", &FLOAT), ("int", &INT)],
                '.' => &[("float", &FLOAT)],
                '0'..='9' => &[
                    ("float", &FLOAT),
                    ("int", &INT),
                    ("timestamp", &TIMESTAMP_IMPLICIT),
                ],
                '<' => &[("merge", &MERGE)],
                '~' => &[("null", &NULL)],
                '=' => &[("value", &VALUE)],
                '!' | '&' | '*' => &[("yaml", &YAML)],
                _ => &[],
            },
        };
        for (name, regex) in candidates {
            if regex.is_match(value) {
                return format!("tag:yaml.org,2002:{name}");
            }
        }
    }
    "tag:yaml.org,2002:str".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether `text` fails to load with a YAML error containing `needle`.
    fn refused_with(text: &str, needle: &str) -> bool {
        matches!(load(text), Err(Error::Yaml(message)) if message.contains(needle))
    }

    #[test]
    fn duplicate_key_message_matches_pyyaml() {
        assert_eq!(
            load("a: 1\na: 2\n").unwrap_err(),
            Error::Yaml(
                "while constructing a mapping\n  in \"<unicode string>\", line 1, column 1:\n    a: 1\n    ^\n\
                 found duplicate key 'a'\n  in \"<unicode string>\", line 2, column 1:\n    a: 2\n    ^"
                    .into()
            )
        );
    }

    #[test]
    fn scalars_resolve_as_yaml_1_1() {
        let v =
            load("a: yes\nb: 012\nc: 1:30\nd: 2026-10-17\ne: ~\nf: .5\ng: '1'\nh: 0x1f\n").unwrap();
        let get = |k: &str| v.get(k).unwrap().repr().unwrap();
        assert_eq!(get("a"), "True");
        assert_eq!(get("b"), "10");
        assert_eq!(get("c"), "90");
        assert_eq!(get("d"), "datetime.date(2026, 10, 17)");
        assert_eq!(get("e"), "None");
        assert_eq!(get("f"), "0.5");
        assert_eq!(get("g"), "'1'");
        assert_eq!(get("h"), "31");
    }

    #[test]
    fn binary_data_after_padding_reads_as_cpython_reads_it() {
        // yaml.safe_load(f'!!binary "{text}"') with PyYAML 6.0.3, CPython 3.15.0.
        let padding = "failed to decode base64 data: Incorrect padding";
        let one_more = "failed to decode base64 data: Invalid base64-encoded string: \
                        number of data characters (5) cannot be 1 more than a multiple of 4";
        for (text, expected) in [
            ("aGk=", Ok("b'hi'")),
            ("aGk=x", Ok("b'hi1'")),
            ("aGk==x", Ok("b'hi1'")),
            ("aGk=!x", Ok("b'hi1'")),
            ("aG==k=", Ok("b'hi'")),
            ("aG==aG==", Ok(r"b'hf\x86'")),
            ("YQ=Y=", Ok(r"b'a\x06'")),
            ("aGk=!", Ok("b'hi'")),
            ("aGk=aGk=", Err(padding)),
            ("aGk=xyz", Err(padding)),
            ("YQ=x", Err(padding)),
            ("aGk=xy", Err(one_more)),
            (r"aGk=\nYQ==", Err(one_more)),
        ] {
            let got = load(&format!("!!binary \"{text}\"\n"));
            match expected {
                Ok(repr) => assert_eq!(got.unwrap().repr().unwrap(), repr, "{text}"),
                Err(reason) => assert!(
                    matches!(&got, Err(Error::Yaml(m)) if m.starts_with(reason)),
                    "{text}: {got:?}"
                ),
            }
        }
    }

    #[test]
    fn equal_numbers_are_duplicate_keys() {
        assert!(refused_with("1: a\ntrue: b\n", "found duplicate key True"));
    }

    #[test]
    fn reader_refuses_control_characters() {
        assert_eq!(
            load("a: \u{1}").unwrap_err(),
            Error::Yaml(
                "unacceptable character #x0001: special characters are not allowed\n  in \"<unicode string>\", position 3"
                    .into()
            )
        );
    }

    #[test]
    fn merge_override_is_a_duplicate() {
        assert!(refused_with(
            "x: &a {k: 1}\ny:\n  <<: *a\n  k: 2\n",
            "found duplicate key 'k'"
        ));
    }

    #[test]
    fn nesting_has_a_limit() {
        // The binary runs the loader on a large stack; so does this test.
        let check = || {
            let deep = format!("{}{}", "[".repeat(MAX_DEPTH + 1), "]".repeat(MAX_DEPTH + 1));
            assert_eq!(load(&deep).unwrap_err(), Error::TooDeep);
            let ok = format!("{}{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
            assert!(load(&ok).is_ok());
        };
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(check)
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn token_ids_are_pyyaml_token_ids() {
        // Parser errors name the token found by its PyYAML class id.
        let ids = [
            (Kind::Directive, "<directive>"),
            (Kind::DocumentStart, "<document start>"),
            (Kind::DocumentEnd, "<document end>"),
            (Kind::StreamStart, "<stream start>"),
            (Kind::StreamEnd, "<stream end>"),
            (Kind::BlockSequenceStart, "<block sequence start>"),
            (Kind::BlockMappingStart, "<block mapping start>"),
            (Kind::BlockEnd, "<block end>"),
            (Kind::FlowSequenceStart, "["),
            (Kind::FlowMappingStart, "{"),
            (Kind::FlowSequenceEnd, "]"),
            (Kind::FlowMappingEnd, "}"),
            (Kind::Key, "?"),
            (Kind::Value, ":"),
            (Kind::BlockEntry, "-"),
            (Kind::FlowEntry, ","),
            (Kind::Alias, "<alias>"),
            (Kind::Anchor, "<anchor>"),
            (Kind::Tag, "<tag>"),
            (Kind::Scalar, "<scalar>"),
        ];
        for (kind, id) in ids {
            assert_eq!(kind.id(), id);
        }
    }

    #[test]
    fn crlf_and_cr_line_breaks_read_as_pyyaml_reads_them() {
        // The loaders translate line endings first; load() itself still
        // reads every break PyYAML's reader accepts.
        let v = load("a: 1\r\nb: |\r\n  x\r\n  y\r\n").unwrap();
        assert_eq!(v.repr().unwrap(), r"{'a': 1, 'b': 'x\ny\n'}");
        let v = load("a: 1\rb: 2\r").unwrap();
        assert_eq!(v.repr().unwrap(), "{'a': 1, 'b': 2}");
        let v = load("a: \"x\r\n\r\n  y\"\r\n").unwrap();
        assert_eq!(v.repr().unwrap(), r"{'a': 'x\ny'}");
    }

    #[test]
    fn a_plain_scalar_never_starts_at_a_comment() {
        // The scanner skips a comment before it fetches a token, so this
        // guard is defensive: positioned at '#', a plain scalar takes nothing.
        let mut loader = Loader::new("#x").unwrap();
        let token = loader.scan_plain().unwrap();
        assert_eq!(token.kind, Kind::Scalar);
        assert_eq!(token.value, "");
        assert_eq!(loader.peek(0), '#');
    }

    #[test]
    fn values_answer_as_python_objects() {
        let list = load("a: [1]\n").unwrap().get("a").unwrap();
        assert!(list.get("a").is_none(), "only a dict has keys");
        let one = Value::Tuple(vec![Value::Str("x".into())]);
        let pair = |n: i64| Value::Tuple(vec![Value::Str("x".into()), Value::Int(n.into())]);
        assert_eq!(one.repr().unwrap(), "('x',)");
        assert_eq!(pair(1).repr().unwrap(), "('x', 1)");
        assert!(pair(1).py_eq(&pair(1)));
        assert!(!pair(1).py_eq(&pair(2)));
        assert!(!one.py_eq(&pair(1)));
    }
}
