//! Iterative strict grammar, fixed state per depth, finite summaries only.
use super::{
    Fields, Packet,
    strings::{string, take},
};
#[derive(Clone, Copy, Default)]
struct Frame {
    state: u8,
    role: u8,
    key: u8,
    reserved: u8,
}
struct Scanner<'a> {
    raw: &'a str,
    at: usize,
    depth: usize,
    frames: [Frame; 128],
    packet: Packet,
}
const _: () = assert!(std::mem::size_of::<Frame>() == 4);
const _: () = assert!(std::mem::size_of::<Scanner<'static>>() <= 1024);
const _: () = assert!(std::mem::size_of::<super::Token>() <= 88);
const IGNORED: u8 = 9;
impl Scanner<'_> {
    fn ws(&mut self) {
        while matches!(
            self.raw.as_bytes().get(self.at),
            Some(b' ' | b'\t' | b'\r' | b'\n')
        ) {
            self.at += 1;
        }
    }
    fn byte(&self) -> Result<u8, ()> {
        self.raw.as_bytes().get(self.at).copied().ok_or(())
    }
    fn fields(&mut self, role: u8) -> Option<&mut Fields> {
        let index = match role {
            0 => 0,
            1 => 1,
            3 => 2,
            _ => return None,
        };
        Some(&mut self.packet.fields[index])
    }
    fn role(&mut self, parent: u8, key: u8, object: bool) -> u8 {
        let role = match (parent, key) {
            (0, 2) => 1,
            (0, 3) => 2,
            (2, 2) => 3,
            (2, 7) => 4,
            _ => IGNORED,
        };
        match role {
            1 => self.packet.fields[1] = Fields::default(),
            2 => {
                self.packet.fields[2] = Fields::default();
                self.packet.reason = 0;
            }
            3 => self.packet.fields[2] = Fields::default(),
            4 => self.packet.reason = 0,
            _ => {}
        }
        if object { role } else { IGNORED }
    }
    fn primitive(
        &mut self,
        parent: u8,
        key: u8,
        kind: u8,
        token: Option<super::Token>,
        numeric: u16,
    ) {
        if let Some(fields) = self.fields(parent) {
            match key {
                1 => {
                    fields.name = token.as_ref().map(super::symbol).unwrap_or(0);
                    if parent == 0 {
                        fields.kind = if token.is_some() { 2 } else { 0 };
                    }
                }
                4 => {
                    fields.code = token.as_ref().map(super::symbol).unwrap_or(0);
                    fields.numeric = numeric;
                }
                5 => {
                    fields.message_model = token.as_ref().is_some_and(super::Token::model_prefix);
                    fields.message_refusal = token.as_ref().is_some_and(super::Token::refusal);
                }
                6 => fields.detail_refusal = token.as_ref().is_some_and(super::Token::refusal),
                _ => {}
            }
        }
        if parent == 4 && key == 8 {
            self.packet.reason = token.as_ref().map(super::symbol).unwrap_or(0);
        }
        // Shape of selected error values is independent of their semantic fields.
        let slot = match (parent, key) {
            (0, 2) => Some(1),
            (2, 2) => Some(2),
            _ => None,
        };
        if let Some(slot) = slot {
            self.packet.fields[slot].kind = kind;
        }
    }
    fn value(&mut self, parent: u8, key: u8, root: bool) -> Result<(), ()> {
        self.ws();
        let byte = self.byte()?;
        let object = byte == b'{';
        let mut role = self.role(parent, key, object);
        if root {
            role = 0;
            self.packet.object = object;
        }
        match byte {
            b'{' | b'[' if self.depth < 127 => {
                self.primitive(parent, key, if object { 1 } else { 3 }, None, 0);
                self.frames[self.depth] = Frame {
                    state: if object { 0 } else { 4 },
                    role: if object { role } else { IGNORED },
                    ..Frame::default()
                };
                self.depth += 1;
                self.at += 1;
            }
            b'"' => {
                let token = string(self.raw.as_bytes(), &mut self.at)?;
                let kind = if token.nonempty() { 2 } else { 4 };
                self.primitive(parent, key, kind, Some(token), 0);
            }
            b't' => self.literal(parent, key, "true")?,
            b'f' => self.literal(parent, key, "false")?,
            b'n' => self.literal(parent, key, "null")?,
            b'-' | b'0'..=b'9' => {
                let start = self.at;
                while matches!(
                    self.raw.as_bytes().get(self.at),
                    Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                ) {
                    self.at += 1;
                }
                let raw = &self.raw[start..self.at];
                serde_json::from_str::<f64>(raw).map_err(|_| ())?;
                let numeric = raw
                    .parse::<u16>()
                    .ok()
                    .filter(|n| (400..=599).contains(n))
                    .unwrap_or(0);
                self.primitive(parent, key, 3, None, numeric);
            }
            _ => return Err(()),
        }
        Ok(())
    }
    fn literal(&mut self, parent: u8, key: u8, expected: &str) -> Result<(), ()> {
        if self
            .raw
            .get(self.at..)
            .is_some_and(|tail| tail.starts_with(expected))
        {
            self.at += expected.len();
            self.primitive(parent, key, if expected == "null" { 0 } else { 3 }, None, 0);
            Ok(())
        } else {
            Err(())
        }
    }
    fn step(&mut self) -> Result<(), ()> {
        self.ws();
        let frame = self.frames[self.depth - 1];
        debug_assert_eq!(frame.reserved, 0);
        match (frame.state, self.byte()?) {
            (0, b'}') | (4, b']') | (3, b'}') | (6, b']') => {
                self.depth -= 1;
                self.at += 1;
            }
            (0 | 1, b'"') => {
                let token = string(self.raw.as_bytes(), &mut self.at)?;
                let names = [
                    "type",
                    "error",
                    "response",
                    "code",
                    "message",
                    "detail",
                    "incomplete_details",
                    "reason",
                ];
                let key = names
                    .iter()
                    .position(|key| token.is(key))
                    .map(|n| n as u8 + 1)
                    .unwrap_or(0);
                self.ws();
                take(self.raw.as_bytes(), &mut self.at, b':')?;
                self.frames[self.depth - 1].state = 3;
                self.frames[self.depth - 1].key = key;
                self.value(frame.role, key, false)?;
            }
            (4 | 5, _) => {
                self.frames[self.depth - 1].state = 6;
                self.value(IGNORED, 0, false)?;
            }
            (3, b',') => {
                self.frames[self.depth - 1].state = 1;
                self.at += 1;
            }
            (6, b',') => {
                self.frames[self.depth - 1].state = 5;
                self.at += 1;
            }
            _ => return Err(()),
        }
        Ok(())
    }
}
pub(super) fn parse(raw: &str) -> Result<Packet, ()> {
    let mut scanner = Scanner {
        raw,
        at: 0,
        depth: 0,
        frames: [Frame::default(); 128],
        packet: Packet::default(),
    };
    scanner.value(IGNORED, 0, true)?;
    while scanner.depth > 0 {
        scanner.step()?;
    }
    scanner.ws();
    if scanner.at == raw.len() {
        Ok(scanner.packet)
    } else {
        Err(())
    }
}
