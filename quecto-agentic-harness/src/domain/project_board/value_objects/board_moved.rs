//! Where a board moved (epic #2705 B13): the `format/MOVED` file of a
//! board fenced at format 4, naming the board's id and the board-server
//! URLs that serve it now.
//!
//! **Grammar.** UTF-8 text of LF-ended lines, at most [`MAX_MOVED_BYTES`]:
//! first `board_id <id>`, the id as a record names the board (16
//! lowercase hex digits, [`BoardId`]); then 1 to [`MAX_MOVED_URLS`] lines
//! `url <url>`, each distinct, in the order a client tries them. Each line
//! is its key, one space and its value; no other key, line, space or byte
//! is taken. A URL is `http://` or `https://`, a host (DNS labels, a
//! dotted IPv4 address or a bracketed IPv6 address: no userinfo, so no
//! credential), an optional port from 1 to 65535, and an optional path of
//! [`URL_PATH_PUNCTUATION`], letters and digits, at most [`MAX_URL_BYTES`]
//! in all. Parsing is an allowlist: anything else is malformed.
use std::fmt;

use super::telemetry::BoardId;

/// The most URLs a MOVED file names.
pub const MAX_MOVED_URLS: usize = 8;
/// The longest URL a MOVED file names.
pub const MAX_URL_BYTES: usize = 256;
/// The most bytes a MOVED file holds.
pub const MAX_MOVED_BYTES: usize = 4096;
/// The punctuation a URL's path may hold besides letters and digits.
pub const URL_PATH_PUNCTUATION: &str = "/-._~%:=&+,?";
/// The key of the board id's line.
const BOARD_KEY: &str = "board_id";
/// The key of each URL's line.
const URL_KEY: &str = "url";

// Every file of the most URLs, each of the longest, fits the byte bound.
const _: () = assert!(
    BOARD_KEY.len() + 18 + MAX_MOVED_URLS * (URL_KEY.len() + MAX_URL_BYTES + 2) <= MAX_MOVED_BYTES
);

/// A board moved to a board server: its id, and the URLs that serve it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardMoved {
    board: BoardId,
    urls: Vec<String>,
}

impl BoardMoved {
    /// # Errors
    /// No URL, more than [`MAX_MOVED_URLS`], one twice, or one that is not
    /// an http(s) URL of the grammar.
    pub fn new(board: BoardId, urls: Vec<String>) -> Result<Self, String> {
        if urls.is_empty() || urls.len() > MAX_MOVED_URLS {
            return Err(format!(
                "it names {} URLs; a board moves to 1 to {MAX_MOVED_URLS}",
                urls.len()
            ));
        }
        for (at, url) in urls.iter().enumerate() {
            served_url(url)?;
            if urls[..at].contains(url) {
                return Err(format!("it names {url} twice"));
            }
        }
        let moved = Self { board, urls };
        assert!(
            moved.file().len() <= MAX_MOVED_BYTES,
            "every MOVED of the grammar fits its byte bound"
        );
        Ok(moved)
    }

    /// The MOVED file `bytes` spells, when it is exactly the file
    /// [`Self::file`] writes for it.
    ///
    /// # Errors
    /// Why `bytes` is not a MOVED file of the grammar.
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_MOVED_BYTES {
            return Err(format!(
                "it is {} bytes; the most read is {MAX_MOVED_BYTES}",
                bytes.len()
            ));
        }
        let text = std::str::from_utf8(bytes).map_err(|_| "it is not UTF-8".to_string())?;
        let Some(body) = text.strip_suffix('\n') else {
            return Err("its last line does not end in a newline".into());
        };
        let mut lines = body.split('\n').map(|line| line.split_once(' '));
        let board = match lines.next() {
            Some(Some((BOARD_KEY, id))) => BoardId::of_hex(id)
                .ok_or_else(|| format!("{id:?} is not a board id (16 lowercase hex digits)"))?,
            _ => return Err(format!("its first line is not `{BOARD_KEY} <id>`")),
        };
        let urls = lines
            .map(|line| match line {
                Some((URL_KEY, url)) => Ok(url.to_string()),
                _ => Err(format!("a line after the first is not `{URL_KEY} <url>`")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let moved = Self::new(board, urls)?;
        assert_eq!(moved.file(), bytes, "a MOVED parsed is the file it writes");
        Ok(moved)
    }

    /// The MOVED file: the board id's line, then each URL's.
    pub fn file(&self) -> Vec<u8> {
        let mut file = format!("{BOARD_KEY} {}\n", self.board.as_str());
        for url in &self.urls {
            file.push_str(&format!("{URL_KEY} {url}\n"));
        }
        file.into_bytes()
    }

    pub fn board(&self) -> &BoardId {
        &self.board
    }

    pub fn urls(&self) -> &[String] {
        &self.urls
    }
}

/// The refusal of a store that does not read a moved board.
impl fmt::Display for BoardMoved {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "this board moved to a board server at {} (board id {}); this build reads it only \
             through that server and never writes to its `quecto/board` branch again: point this \
             board at that server's URL to reach it",
            self.urls.join(", "),
            self.board.as_str()
        )
    }
}

/// Whether `url` is one a MOVED file names: see the module's grammar.
fn served_url(url: &str) -> Result<(), String> {
    let refused = |why: &str| {
        Err(format!(
            "{url:?} is not an http(s) URL a board moves to: {why}"
        ))
    };
    if url.len() > MAX_URL_BYTES {
        return refused(&format!("it is past {MAX_URL_BYTES} bytes"));
    }
    let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        return refused("it is not http:// or https://");
    };
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let path_byte =
        |b: u8| b.is_ascii_alphanumeric() || URL_PATH_PUNCTUATION.as_bytes().contains(&b);
    match (served_authority(authority), path.bytes().all(path_byte)) {
        (true, true) => Ok(()),
        (false, _) => refused(
            "its host is not a DNS name, an IPv4 or a bracketed IPv6 address, with an optional port from 1 to 65535",
        ),
        (true, false) => refused("its path holds a character this file does not take"),
    }
}

/// Whether `authority` is a host and an optional `:port`: DNS labels
/// (ASCII letters and digits with inner hyphens, 1 to 63 bytes each, at
/// most 253 in all), a dotted IPv4 address, or an IPv6 address in
/// brackets; a port of 1 to 65535, written without a leading zero.
fn served_authority(authority: &str) -> bool {
    let (host_ok, port) = match authority.strip_prefix('[') {
        Some(bracketed) => match bracketed.split_once(']') {
            Some((v6, port)) => (ipv6(v6), port),
            None => return false,
        },
        None => {
            let (host, port) = authority.split_at(authority.find(':').unwrap_or(authority.len()));
            let numeric = host.bytes().all(|b| b.is_ascii_digit() || b == b'.');
            let host_ok = match numeric {
                true => ipv4(host),
                false => host.len() <= 253 && host.split('.').all(dns_label),
            };
            (host_ok, port)
        }
    };
    let port_ok = match port.strip_prefix(':') {
        None => port.is_empty(),
        Some(digits) => {
            digits
                .bytes()
                .next()
                .is_some_and(|b| (b'1'..=b'9').contains(&b))
                && digits.bytes().all(|b| b.is_ascii_digit())
                && digits.parse::<u16>().is_ok()
        }
    };
    host_ok && port_ok
}

/// Whether `host` is a dotted IPv4 address: four numbers of 0 to 255,
/// each with no leading zero.
fn ipv4(host: &str) -> bool {
    let parts: Vec<&str> = host.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|part| {
            let digits =
                !part.is_empty() && part.len() <= 3 && part.bytes().all(|b| b.is_ascii_digit());
            digits && (part.len() == 1 || !part.starts_with('0')) && part.parse::<u8>().is_ok()
        })
}

/// Whether `host` is an IPv6 address in hex groups: eight groups of 1 to
/// 4 hex digits, or fewer around one `::`. The dotted IPv4 tail is not
/// taken.
fn ipv6(host: &str) -> bool {
    let group =
        |text: &str| (1..=4).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_hexdigit());
    let groups = |text: &str| match text {
        "" => Some(0),
        _ => text.split(':').all(group).then(|| text.split(':').count()),
    };
    match host.split_once("::") {
        Some((head, tail)) => match (groups(head), groups(tail)) {
            (Some(head), Some(tail)) => head + tail <= 7,
            _ => false,
        },
        None => !host.is_empty() && groups(host) == Some(8),
    }
}

/// Whether `label` is a DNS label: 1 to 63 ASCII letters, digits and
/// inner hyphens.
fn dns_label(label: &str) -> bool {
    let bytes = label.as_bytes();
    let edge = |b: Option<&u8>| b.is_some_and(u8::is_ascii_alphanumeric);
    (1..=63).contains(&bytes.len())
        && edge(bytes.first())
        && edge(bytes.last())
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'-')
}

#[cfg(test)]
#[path = "board_moved_tests.rs"]
mod tests;
