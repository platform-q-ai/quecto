//! #2249 review round 2: real encodings end their escape in a letter, so a
//! key behind one must still be redacted — ANSI colour codes (a grep
//! `--color` dump), hex, quoted-printable and double URL encoding, the
//! remaining C escapes and PowerShell's backtick escapes. A broad corpus
//! pins that no key master redacted is now missed, except a key glued to
//! the letters of a plain word.
use super::*;

const KEY: &str = "sk-proj-Ab3dEf6hIj9kLm2nOp5qRs8tUv1wXy4z";

/// Master's pattern, before the word-start rule (#2241).
const MASTER: &str = r"(?i)(bearer\s+\S+|sk-[A-Za-z0-9_-]{8,}|AKIA[0-9A-Z]{12,}|gh[pousr]_[A-Za-z0-9]{20,}|xox[baprs]-[A-Za-z0-9-]{10,}|AIza[A-Za-z0-9_-]{20,}|(?:api[_-]?key|token|password|secret|access[_-]?token)\s*[=:]\s*\S+)";

/// Escapes and encodings a key travels behind, each ending in a letter.
const ENCODED_LEADS: &[&str] = &[
    "\x1b[01;31m\x1b[K",
    "\x1b[0m\x1b[1m\x1b[31m",
    "\x1b[m",
    "\x1b[?25h",
    r"\e[1m",
    r"\x1b[31m",
    r"\x1B[0K",
    r"\033[1;32m",
    r"\u001b[0m",
    r"\x3d",
    r"\x3D",
    r"\x0a",
    "=3D",
    "=0A",
    // #2249 review round 3: lowercase quoted-printable, and `cat -v`'s
    // caret notation for ESC.
    "=3d",
    "^[[01;31m",
    "^[[0m^[[1m^[[31m",
    "^[[K",
    "%253D",
    "%253d",
    "%25253A",
    r"\a",
    r"\e",
    r"\v",
    r"\U0000003D",
    r"\U0001F60a",
    "`n",
    "`t",
    "`r",
    "`0",
    "`b",
    "`e",
    "`f",
    "`v",
];

#[test]
fn a_key_behind_an_escape_that_ends_in_a_letter_is_redacted() {
    for lead in ENCODED_LEADS {
        for before in ["", "x "] {
            let text = format!("{before}{lead}{KEY} end");
            assert_eq!(
                redact_secrets(&text),
                format!("{before}{lead}[REDACTED] end"),
                "{text:?}"
            );
        }
    }
    for (text, expected) in [
        (
            format!("printf '\\x3d{KEY}'"),
            "printf '\\x3d[REDACTED]'".to_owned(),
        ),
        (
            format!("echo $'k\\x3D{KEY}'"),
            "echo $'k\\x3D[REDACTED]'".to_owned(),
        ),
        (
            format!("Write-Host \"a`n{KEY}\""),
            "Write-Host \"a`n[REDACTED]\"".to_owned(),
        ),
    ] {
        assert_eq!(redact_secrets(&text), expected, "{text:?}");
    }
}

/// An encoded lead is only a lead in its exact shape: a plain word ending
/// in one of its letters stays text, and markdown code such as
/// `` `ask-questionsfreely` `` is no PowerShell bell escape.
#[test]
fn text_that_merely_resembles_an_escape_is_not_a_key() {
    for text in [
        "task-runner01",
        "disk-usage",
        "risk-assessment",
        "`ask-questionsfreely`",
        "x=3Dask-questionsfreely",
        "%3Dask-questionsfreely",
        "[01;31mask-questionsfreely",
    ] {
        assert_eq!(redact_secrets(text), text, "{text:?}");
    }
}

#[test]
fn every_encoded_lead_is_an_allowed_lead() {
    for lead in ENCODED_LEADS {
        // Only the last escape of a chain is the lead.
        let last = ["\x1b[", r"\e[", "^[["]
            .iter()
            .filter_map(|intro| lead.rfind(intro))
            .max()
            .map_or(*lead, |at| &lead[at..]);
        assert!(is_key_lead(last), "{last:?} of {lead:?}");
    }
    for lead in [
        r"\x3",
        r"\xg0",
        "=3",
        "=3g",
        "^[",
        "^[[1",
        "^[1m",
        "%2",
        "%25%3D",
        "%252",
        "%2G3D",
        r"\U0000003",
        "`a",
        "`x",
        "\x1b[",
        "\x1b[1",
        r"\e[1",
        "\x1b[1mm",
        "x",
        "Q",
        "`nn",
    ] {
        assert!(!is_key_lead(lead), "{lead:?}");
    }
}

/// The broad comparison: every lead a key can follow — every ASCII and a
/// few non-ASCII characters, every hex, percent, double-percent and
/// quoted-printable escape, the C, Unicode, ANSI and PowerShell escapes —
/// before every key shape. Whatever master redacted is still redacted,
/// except a key glued to a plain word's letters (`ta` + `sk-…`), which is
/// the one thing #2241 set out to keep.
#[test]
fn no_key_master_redacted_is_missed_except_after_a_plain_word() {
    let master = regex::Regex::new(MASTER).unwrap();
    let keys = [
        KEY,
        "sk-livedeadbeef0001",
        "SK-LIVEDEADBEEF0001",
        "sk-ant-api03-Ab3dEf6hIj9kLm2nOp5qRs8t",
        "AKIAIOSFODNN7EXAMPLE",
        "ghp_0123456789abcdef0123456789abcdef0123",
        "xoxb-0123456789-abcdefghij",
        "AIzaSyA0123456789abcdefghijklmnopq",
    ];
    let mut leads: Vec<String> = (0u8..=0x7f).map(|b| char::from(b).to_string()).collect();
    leads.extend(["é", "ſ", "\u{212A}", "中", "\u{a0}"].map(str::to_owned));
    for byte in 0u8..=0xff {
        leads.push(format!(r"\x{byte:02x}"));
        leads.push(format!(r"\x{byte:02X}"));
        leads.push(format!("%{byte:02x}"));
        leads.push(format!("%{byte:02X}"));
        leads.push(format!("%25{byte:02X}"));
        leads.push(format!("={byte:02X}"));
        leads.push(format!("={byte:02x}"));
        leads.push(format!(r"\u00{byte:02x}"));
        leads.push(format!(r"\U000000{byte:02X}"));
    }
    for c in "abefnrtv0".chars() {
        leads.push(format!(r"\{c}"));
    }
    for c in "befnrtv0".chars() {
        leads.push(format!("`{c}"));
    }
    for intro in ["\x1b", r"\e", r"\x1b", r"\x1B", r"\033", r"\u001b", "^["] {
        for csi in ["[m", "[0m", "[1;31m", "[K", "[?25h", "[2J"] {
            leads.push(format!("{intro}{csi}"));
        }
    }
    let words = [
        "ta", "di", "ri", "A", "fla", "whi", "Z", "x=3Da", "`a", r"\q", r"\xq",
    ];
    for key in keys {
        for lead in leads.iter().map(String::as_str).chain(words) {
            let text = format!("{lead}{key}");
            let masked_by_master = !master.replace_all(&text, "").contains(key);
            let redacted = redact_secrets(&text);
            // A lone ASCII letter is the end of a plain word too.
            let plain_word = words.contains(&lead)
                || (lead.len() == 1 && lead.chars().all(|c| c.is_ascii_alphabetic()));
            if masked_by_master && !(plain_word && key.to_ascii_lowercase().starts_with("sk-")) {
                assert!(!redacted.contains(key), "{text:?} -> {redacted:?}");
            }
        }
    }
}
