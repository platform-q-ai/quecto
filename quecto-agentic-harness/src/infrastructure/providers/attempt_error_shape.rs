//! Borrowed strict oversized-event recognition, before error Value allocation.
//! No owned provider keys/values, decoded string scratch, or payload tree. Only
//! root type/error shape survives; fixed token prefixes and bounded recursion
//! validate all other data. Framing storage remains the caller's separate bound.
#[path = "attempt_error_string.rs"]
mod strings;
use strings::{Token, string, take};

#[derive(Clone, Copy, Default)]
enum Kind {
    #[default]
    Untyped,
    Error,
    Failed,
    Incomplete,
    Other,
}
#[derive(Default)]
struct Shape {
    kind: Kind,
    error: bool,
    object: bool,
    nonempty_string: bool,
}
struct Scanner<'a> {
    input: &'a str,
    at: usize,
}
impl Scanner<'_> {
    fn whitespace(&mut self) {
        while matches!(
            self.input.as_bytes().get(self.at),
            Some(b' ' | b'\t' | b'\n' | b'\r')
        ) {
            self.at += 1;
        }
    }
    fn value(&mut self, depth: u8) -> Result<Shape, ()> {
        debug_assert!(depth <= 127, "serde-compatible container depth is bounded");
        debug_assert!(self.at <= self.input.len(), "scanner offset stays in input");
        self.whitespace();
        match self.input.as_bytes().get(self.at).ok_or(())? {
            b'{' if depth < 127 => self.object(depth + 1),
            b'[' if depth < 127 => {
                self.at += 1;
                self.whitespace();
                if self.input.as_bytes().get(self.at) == Some(&b']') {
                    self.at += 1;
                    return Ok(Shape::default());
                }
                loop {
                    self.value(depth + 1)?;
                    if self.end_or_comma(b']')? {
                        return Ok(Shape::default());
                    }
                }
            }
            b'"' => {
                let token = string(self.input.as_bytes(), &mut self.at)?;
                Ok(Shape {
                    kind: kind(&token),
                    nonempty_string: token.nonempty(),
                    ..Shape::default()
                })
            }
            b't' => self.literal("true"),
            b'f' => self.literal("false"),
            b'n' => self.literal("null"),
            b'-' | b'0'..=b'9' => self.number(),
            _ => Err(()),
        }
    }
    fn object(&mut self, depth: u8) -> Result<Shape, ()> {
        self.at += 1;
        let mut shape = Shape {
            object: true,
            ..Shape::default()
        };
        self.whitespace();
        if self.input.as_bytes().get(self.at) == Some(&b'}') {
            self.at += 1;
            return Ok(shape);
        }
        loop {
            self.whitespace();
            // Convert the finite key before descending; no key buffer remains
            // live during recursion. Duplicates overwrite, exactly like Value.
            let field = {
                let key = string(self.input.as_bytes(), &mut self.at)?;
                if key.is("type") {
                    1
                } else if key.is("error") {
                    2
                } else {
                    0
                }
            };
            self.whitespace();
            take(self.input.as_bytes(), &mut self.at, b':')?;
            let value = self.value(depth)?;
            match field {
                1 => {
                    shape.kind = if value.object {
                        Kind::Untyped
                    } else {
                        value.kind
                    }
                }
                2 => shape.error = value.object || value.nonempty_string,
                _ => {}
            }
            if self.end_or_comma(b'}')? {
                return Ok(shape);
            }
        }
    }
    fn end_or_comma(&mut self, end: u8) -> Result<bool, ()> {
        self.whitespace();
        match self.input.as_bytes().get(self.at) {
            Some(byte) if *byte == end => {
                self.at += 1;
                Ok(true)
            }
            Some(b',') => {
                self.at += 1;
                Ok(false)
            }
            _ => Err(()),
        }
    }
    fn literal(&mut self, literal: &str) -> Result<Shape, ()> {
        match self
            .input
            .get(self.at..)
            .is_some_and(|tail| tail.starts_with(literal))
        {
            true => {
                self.at += literal.len();
                Ok(Shape::default())
            }
            false => Err(()),
        }
    }
    fn number(&mut self) -> Result<Shape, ()> {
        let start = self.at;
        while matches!(
            self.input.as_bytes().get(self.at),
            Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
        ) {
            self.at += 1;
        }
        // Borrow numeric tokens only. The locked serde_json configuration uses
        // finite numeric accumulators (no arbitrary_precision/float_roundtrip).
        // This preserves Value's exact syntax, exponent and range acceptance;
        // no string token ever reaches serde's scratch decoder here.
        serde_json::from_str::<f64>(&self.input[start..self.at]).map_err(|_| ())?;
        Ok(Shape::default())
    }
}
fn kind(token: &Token) -> Kind {
    if token.is("error") {
        Kind::Error
    } else if token.is("response.failed") {
        Kind::Failed
    } else if token.is("response.incomplete") {
        Kind::Incomplete
    } else {
        Kind::Other
    }
}
pub(super) fn failure(
    data: &str,
    vendor: super::Vendor,
) -> Result<Option<super::TerminalEvent>, serde_json::Error> {
    let mut scanner = Scanner { input: data, at: 0 };
    let parsed = scanner.value(0);
    scanner.whitespace();
    let shape = match parsed {
        Ok(shape) if scanner.at == data.len() => shape,
        _ => return Err(serde::de::Error::custom("invalid oversized event JSON")),
    };
    use super::TerminalEvent;
    // Non-object JSON cannot contain an error field or event type. Internal
    // nested summaries must not grant root authority (e.g. type: {type:error}).
    if shape.object {
        Ok(match vendor {
            super::Vendor::OpenAi if shape.error => Some(TerminalEvent::Error),
            super::Vendor::Codex => match shape.kind {
                Kind::Error => Some(TerminalEvent::Error),
                Kind::Failed => Some(TerminalEvent::ResponseFailed),
                Kind::Incomplete => Some(TerminalEvent::ResponseIncomplete),
                Kind::Untyped if shape.error => Some(TerminalEvent::Error),
                _ => None,
            },
            _ => None,
        })
    } else {
        Ok(None)
    }
}
