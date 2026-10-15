//! Strict string validation with a fixed finite-name prefix, never serde scratch.
//! Input is already UTF-8; unescaped bytes are borrowed and escapes are checked
//! (including surrogate pairs) without materializing decoded provider strings.

pub(super) struct Token {
    prefix: [u8; 64],
    len: usize,
    matched: u8,
    refusal: bool,
}
impl Default for Token {
    fn default() -> Self {
        Self {
            prefix: [0; 64],
            len: 0,
            matched: 0,
            refusal: false,
        }
    }
}
const REFUSAL: &[u8] = b"model is not supported when using Codex with a ChatGPT account";
impl Token {
    pub(super) fn model_prefix(&self) -> bool {
        self.prefix.starts_with(b"model:") && self.len >= 6
    }
    pub(super) fn refusal(&self) -> bool {
        self.refusal
    }

    pub(super) fn is(&self, expected: &str) -> bool {
        self.len == expected.len() && self.prefix.get(..self.len) == Some(expected.as_bytes())
    }
    pub(super) fn nonempty(&self) -> bool {
        self.len > 0
    }
    fn push(&mut self, byte: u8) {
        if let Some(slot) = self.prefix.get_mut(self.len) {
            *slot = byte;
        }
        self.len = self.len.saturating_add(1);
        if REFUSAL.get(usize::from(self.matched)) == Some(&byte) {
            self.matched += 1;
            if usize::from(self.matched) == REFUSAL.len() {
                self.refusal = true;
                self.matched = 0;
            }
        } else {
            self.matched = u8::from(byte == REFUSAL[0]);
        }
    }
}
pub(super) fn string(bytes: &[u8], at: &mut usize) -> Result<Token, ()> {
    take(bytes, at, b'"')?;
    let mut token = Token::default();
    loop {
        let byte = *bytes.get(*at).ok_or(())?;
        *at += 1;
        match byte {
            b'"' => return Ok(token),
            b'\\' => {
                let escape = *bytes.get(*at).ok_or(())?;
                *at += 1;
                match escape {
                    b'"' | b'\\' | b'/' => token.push(escape),
                    b'b' => token.push(8),
                    b'f' => token.push(12),
                    b'n' => token.push(b'\n'),
                    b'r' => token.push(b'\r'),
                    b't' => token.push(b'\t'),
                    b'u' => unicode(bytes, at, &mut token)?,
                    _ => return Err(()),
                }
            }
            0x20..=0xff => token.push(byte),
            _ => return Err(()),
        }
    }
}
fn unicode(bytes: &[u8], at: &mut usize, token: &mut Token) -> Result<(), ()> {
    let first = hex(bytes, at)?;
    let scalar = match first {
        0xd800..=0xdbff => {
            take(bytes, at, b'\\')?;
            take(bytes, at, b'u')?;
            let second = hex(bytes, at)?;
            match second {
                0xdc00..=0xdfff => 0x10000 + ((first - 0xd800) << 10) + second - 0xdc00,
                _ => return Err(()),
            }
        }
        0xdc00..=0xdfff => return Err(()),
        _ => first,
    };
    let character = char::from_u32(scalar).ok_or(())?;
    let mut encoded = [0; 4];
    for byte in character.encode_utf8(&mut encoded).bytes() {
        token.push(byte);
    }
    Ok(())
}
fn hex(bytes: &[u8], at: &mut usize) -> Result<u32, ()> {
    let mut value = 0;
    for _ in 0..4 {
        let digit = match bytes.get(*at).ok_or(())? {
            byte @ b'0'..=b'9' => u32::from(byte - b'0'),
            byte @ b'a'..=b'f' => u32::from(byte - b'a' + 10),
            byte @ b'A'..=b'F' => u32::from(byte - b'A' + 10),
            _ => return Err(()),
        };
        *at += 1;
        value = value * 16 + digit;
    }
    Ok(value)
}
pub(super) fn take(bytes: &[u8], at: &mut usize, expected: u8) -> Result<(), ()> {
    match bytes.get(*at) {
        Some(byte) if *byte == expected => {
            *at += 1;
            Ok(())
        }
        _ => Err(()),
    }
}
