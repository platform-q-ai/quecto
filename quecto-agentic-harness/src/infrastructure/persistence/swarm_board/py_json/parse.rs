//! `json.loads`, reproduced (#2268): CPython's scanner grammar, its
//! `NaN`/`Infinity` extensions, exact integers, correctly rounded floats,
//! lone surrogates from `\u` escapes, last-wins repeated keys, and its error
//! messages and positions. An explicit stack replaces the recursion, so the
//! nesting bound is [`DECODE_MAX_DEPTH`], not the thread's stack.

use std::collections::HashMap;

use super::{DECODE_MAX_DEPTH, PyInt, PyJson, PyJsonError, PyObject, PyStr};

pub(super) fn parse(text: &str) -> Result<PyJson, PyJsonError> {
    let mut parser = Parser { text, pos: 0 };
    if text.starts_with('\u{feff}') {
        return Err(parser.error("Unexpected UTF-8 BOM (decode using utf-8-sig)", 0));
    }
    parser.skip_whitespace();
    let value = parser.document()?;
    parser.skip_whitespace();
    if parser.pos == text.len() {
        Ok(value)
    } else {
        Err(parser.error("Extra data", parser.pos))
    }
}

struct Parser<'a> {
    text: &'a str,
    /// Byte offset of the next unread character.
    pos: usize,
}

/// A container being read: its items so far, and for an object the key
/// awaiting its value and each key's position (a repeated key keeps its
/// first position and takes the last value, as a Python `dict` does).
enum Frame {
    List(Vec<PyJson>),
    Object {
        entries: Vec<(PyStr, PyJson)>,
        positions: HashMap<PyStr, usize>,
        key: PyStr,
    },
}

/// What starts at the current position.
enum Start {
    Scalar(PyJson),
    List,
    Object,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.pos).copied()
    }

    fn rest(&self) -> &'a str {
        &self.text[self.pos..]
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    /// `json.JSONDecodeError(message, doc, pos)`: line, column and offset
    /// count characters, as Python's do.
    fn error(&self, message: &str, byte: usize) -> PyJsonError {
        let before = &self.text[..byte];
        let offset = before.chars().count();
        let line = before.matches('\n').count() + 1;
        let column = match before.rfind('\n') {
            Some(newline) => before[newline + 1..].chars().count() + 1,
            None => offset + 1,
        };
        PyJsonError::Syntax {
            message: message.to_owned(),
            line,
            column,
            offset,
        }
    }

    /// One whole value, containers read with an explicit stack.
    fn document(&mut self) -> Result<PyJson, PyJsonError> {
        let mut stack: Vec<Frame> = Vec::new();
        loop {
            let mut value = match self.start()? {
                Start::Scalar(value) => value,
                Start::List => {
                    open(&stack, "decoding a JSON array from a unicode string")?;
                    self.skip_whitespace();
                    if self.peek() == Some(b']') {
                        self.pos += 1;
                        PyJson::List(Vec::new())
                    } else {
                        stack.push(Frame::List(Vec::new()));
                        continue;
                    }
                }
                Start::Object => {
                    open(&stack, "decoding a JSON object from a unicode string")?;
                    self.skip_whitespace();
                    if self.peek() == Some(b'}') {
                        self.pos += 1;
                        PyJson::Object(PyObject::new())
                    } else {
                        let key = self.key()?;
                        stack.push(Frame::Object {
                            entries: Vec::new(),
                            positions: HashMap::new(),
                            key,
                        });
                        continue;
                    }
                }
            };
            // Hand the finished value to its container, closing every
            // container that ends here, until one wants another value.
            loop {
                let Some(frame) = stack.last_mut() else {
                    return Ok(value);
                };
                self.skip_whitespace();
                let closed = match frame {
                    Frame::List(items) => {
                        items.push(value);
                        self.after_item(b']', "array")?
                    }
                    Frame::Object {
                        entries,
                        positions,
                        key: pending,
                    } => {
                        let key = std::mem::replace(pending, PyStr::from(""));
                        match positions.get(&key) {
                            Some(&position) => entries[position].1 = value,
                            None => {
                                positions.insert(key.clone(), entries.len());
                                entries.push((key, value));
                            }
                        }
                        let closed = self.after_item(b'}', "object")?;
                        if !closed {
                            *pending = self.key()?;
                        }
                        closed
                    }
                };
                if !closed {
                    break;
                }
                value = match stack.pop().expect("the frame just read") {
                    Frame::List(items) => PyJson::List(items),
                    Frame::Object { entries, .. } => PyJson::Object(PyObject::from_unique(entries)),
                };
            }
        }
    }

    /// After a container item: `true` when the container closes, `false`
    /// after a `,` (whitespace after it skipped).
    fn after_item(&mut self, close: u8, kind: &str) -> Result<bool, PyJsonError> {
        match self.peek() {
            Some(byte) if byte == close => {
                self.pos += 1;
                Ok(true)
            }
            Some(b',') => {
                let comma = self.pos;
                self.pos += 1;
                self.skip_whitespace();
                if self.peek() == Some(close) {
                    let message = format!("Illegal trailing comma before end of {kind}");
                    return Err(self.error(&message, comma));
                }
                Ok(false)
            }
            _ => Err(self.error("Expecting ',' delimiter", self.pos)),
        }
    }

    /// An object key, its `:` and the whitespace before the value.
    fn key(&mut self) -> Result<PyStr, PyJsonError> {
        if self.peek() != Some(b'"') {
            return Err(self.error(
                "Expecting property name enclosed in double quotes",
                self.pos,
            ));
        }
        let key = self.string()?;
        self.skip_whitespace();
        if self.peek() != Some(b':') {
            return Err(self.error("Expecting ':' delimiter", self.pos));
        }
        self.pos += 1;
        self.skip_whitespace();
        Ok(key)
    }

    fn start(&mut self) -> Result<Start, PyJsonError> {
        match self.peek() {
            Some(b'[') => {
                self.pos += 1;
                Ok(Start::List)
            }
            Some(b'{') => {
                self.pos += 1;
                Ok(Start::Object)
            }
            Some(b'"') => Ok(Start::Scalar(PyJson::Str(self.string()?))),
            Some(b'-' | b'0'..=b'9') if !self.rest().starts_with("-Infinity") => {
                self.number().map(Start::Scalar)
            }
            Some(_) => match constant(self.rest()) {
                Some((length, value)) => {
                    self.pos += length;
                    Ok(Start::Scalar(value))
                }
                None => Err(self.error("Expecting value", self.pos)),
            },
            None => Err(self.error("Expecting value", self.pos)),
        }
    }

    /// `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][-+]?[0-9]+)?`, where a fraction or
    /// exponent without digits is not consumed (Python then reports what
    /// follows). A float is correctly rounded (`1e400` is infinite), an
    /// integer is exact.
    fn number(&mut self) -> Result<PyJson, PyJsonError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => self.digits(),
            _ => return Err(self.error("Expecting value", start)),
        }
        let mut float = false;
        let bytes = self.text.as_bytes();
        if self.peek() == Some(b'.') && bytes.get(self.pos + 1).is_some_and(u8::is_ascii_digit) {
            self.pos += 1;
            self.digits();
            float = true;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let mark = self.pos;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.digits();
                float = true;
            } else {
                self.pos = mark;
            }
        }
        let literal = &self.text[start..self.pos];
        if float {
            let value: f64 = literal
                .parse()
                .expect("a JSON float literal is Rust float syntax");
            Ok(PyJson::Float(value))
        } else {
            PyInt::parse(literal).map(PyJson::Int)
        }
    }

    fn digits(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.pos += 1;
        }
    }

    /// A string starting at its opening quote (`strict=True`: raw control
    /// characters are refused).
    fn string(&mut self) -> Result<PyStr, PyJsonError> {
        let start = self.pos;
        self.pos += 1;
        let mut out = StrBuilder::default();
        loop {
            let rest = self.rest();
            let run = rest
                .bytes()
                .position(|byte| byte == b'"' || byte == b'\\' || byte < 0x20)
                .ok_or_else(|| self.error("Unterminated string starting at", start))?;
            out.push_str(&rest[..run]);
            self.pos += run;
            match self.peek() {
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out.finish());
                }
                Some(b'\\') => self.escape(&mut out)?,
                _ => return Err(self.error("Invalid control character at", self.pos)),
            }
        }
    }

    /// One escape starting at its backslash.
    fn escape(&mut self, out: &mut StrBuilder) -> Result<(), PyJsonError> {
        let backslash = self.pos;
        let simple = match self.text.as_bytes().get(backslash + 1) {
            Some(b'"') => Some('"'),
            Some(b'\\') => Some('\\'),
            Some(b'/') => Some('/'),
            Some(b'b') => Some('\u{8}'),
            Some(b'f') => Some('\u{c}'),
            Some(b'n') => Some('\n'),
            Some(b'r') => Some('\r'),
            Some(b't') => Some('\t'),
            Some(b'u') => None,
            Some(_) => return Err(self.error("Invalid \\escape", backslash)),
            None => return Err(self.error("Unterminated string starting at", backslash)),
        };
        if let Some(character) = simple {
            out.push_str(character.encode_utf8(&mut [0; 4]));
            self.pos += 2;
            return Ok(());
        }
        let unit = self.hex4(backslash + 2)?;
        self.pos = backslash + 6;
        if (0xD800..=0xDBFF).contains(&unit) && self.rest().starts_with("\\u") {
            let low = self.hex4(self.pos + 2)?;
            if (0xDC00..=0xDFFF).contains(&low) {
                self.pos += 6;
                out.push_point(0x1_0000 + ((unit - 0xD800) << 10) + (low - 0xDC00));
                return Ok(());
            }
        }
        out.push_point(unit);
        Ok(())
    }

    /// Four hex digits at `at`, either case; an error points at the `u`
    /// before them, as Python's does.
    fn hex4(&self, at: usize) -> Result<u32, PyJsonError> {
        self.text
            .get(at..at + 4)
            .filter(|digits| digits.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .map(|digits| u32::from_str_radix(digits, 16).expect("four hex digits"))
            .ok_or_else(|| self.error(&format!("Invalid \\u{} escape", "X".repeat(4)), at - 1))
    }
}

/// The literal `rest` starts with, including Python's `NaN`, `Infinity`
/// and `-Infinity`, and its length.
fn constant(rest: &str) -> Option<(usize, PyJson)> {
    let constants = [
        ("null", PyJson::Null),
        ("true", PyJson::Bool(true)),
        ("false", PyJson::Bool(false)),
        ("NaN", PyJson::Float(f64::NAN)),
        ("Infinity", PyJson::Float(f64::INFINITY)),
        ("-Infinity", PyJson::Float(f64::NEG_INFINITY)),
    ];
    constants
        .into_iter()
        .find(|(word, _)| rest.starts_with(word))
        .map(|(word, value)| (word.len(), value))
}

/// Refuses a container that would nest beyond [`DECODE_MAX_DEPTH`].
fn open(stack: &[Frame], action: &'static str) -> Result<(), PyJsonError> {
    if stack.len() < DECODE_MAX_DEPTH {
        Ok(())
    } else {
        Err(PyJsonError::TooDeep {
            limit: DECODE_MAX_DEPTH,
            action,
        })
    }
}

/// A string being read: UTF-8 until the first lone surrogate, then code
/// points.
#[derive(Default)]
struct StrBuilder {
    text: String,
    points: Option<Vec<u32>>,
}

impl StrBuilder {
    fn push_str(&mut self, text: &str) {
        match &mut self.points {
            Some(points) => points.extend(text.chars().map(u32::from)),
            None => self.text.push_str(text),
        }
    }

    fn push_point(&mut self, point: u32) {
        match (&mut self.points, char::from_u32(point)) {
            (Some(points), _) => points.push(point),
            (None, Some(character)) => self.text.push(character),
            (None, None) => {
                let mut points: Vec<u32> = self.text.chars().map(u32::from).collect();
                points.push(point);
                self.points = Some(points);
            }
        }
    }

    fn finish(self) -> PyStr {
        match self.points {
            Some(points) => PyStr::from_code_points(points).expect("escapes stay within U+10FFFF"),
            None => PyStr::from(self.text),
        }
    }
}

#[cfg(test)]
#[path = "parse_tests.rs"]
mod tests;
