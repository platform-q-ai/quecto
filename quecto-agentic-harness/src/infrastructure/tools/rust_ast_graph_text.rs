use std::path::Path;

use serde::Serialize;

pub fn line_col(text: &str, byte: usize) -> (usize, usize) {
    let mut line = 1;
    let mut last = 0;
    for (i, b) in text.bytes().enumerate() {
        if i >= byte {
            break;
        }
        if b == b'\n' {
            line += 1;
            last = i + 1;
        }
    }
    (line, byte.saturating_sub(last) + 1)
}
pub fn line_end(text: &str, byte: usize) -> usize {
    text[byte..]
        .find('\n')
        .map(|i| byte + i)
        .unwrap_or(text.len())
}
pub fn snippet(text: &str, line: usize, ctx: usize) -> String {
    if ctx == 0 {
        return String::new();
    }
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return String::new();
    }
    let start = line.saturating_sub(ctx + 1);
    let end = (line + ctx).min(lines.len());
    let mut out = String::new();
    for (idx, l) in lines[start..end].iter().enumerate() {
        out.push_str(&format!("{:>4}: {}\n", start + idx + 1, l));
    }
    out
}
pub fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value)
        .unwrap_or_else(|e| format!(r#"{{"error":"failed to serialize response: {e}"}}"#))
}

pub fn mask_comments_and_strings(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let mut block_depth = 0usize;
    while i < bytes.len() {
        if block_depth > 0 {
            if starts(bytes, i, b"/*") {
                push_spaces(&mut out, bytes, i, 2);
                i += 2;
                block_depth += 1;
            } else if starts(bytes, i, b"*/") {
                push_spaces(&mut out, bytes, i, 2);
                i += 2;
                block_depth -= 1;
            } else {
                push_space_or_newline(&mut out, bytes[i]);
                i += 1;
            }
        } else if starts(bytes, i, b"//") {
            while i < bytes.len() {
                let b = bytes[i];
                push_space_or_newline(&mut out, b);
                i += 1;
                if b == b'\n' {
                    break;
                }
            }
        } else if starts(bytes, i, b"/*") {
            push_spaces(&mut out, bytes, i, 2);
            i += 2;
            block_depth = 1;
        } else if let Some(end) = raw_string_end(bytes, i) {
            push_spaces(&mut out, bytes, i, end - i);
            i = end;
        } else if starts(bytes, i, b"b\"") {
            let end = quoted_end(bytes, i + 1, b'"');
            push_spaces(&mut out, bytes, i, end - i);
            i = end;
        } else if bytes[i] == b'"' {
            let end = quoted_end(bytes, i, b'"');
            push_spaces(&mut out, bytes, i, end - i);
            i = end;
        } else if bytes[i] == b'\'' && is_char_literal_start(bytes, i) {
            let end = quoted_end(bytes, i, b'\'');
            push_spaces(&mut out, bytes, i, end - i);
            i = end;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).expect("mask preserves UTF-8 validity by only copying or ASCII-masking")
}

fn starts(bytes: &[u8], i: usize, needle: &[u8]) -> bool {
    bytes.get(i..i + needle.len()) == Some(needle)
}

fn push_space_or_newline(out: &mut Vec<u8>, b: u8) {
    out.push(if b == b'\n' { b'\n' } else { b' ' });
}

fn push_spaces(out: &mut Vec<u8>, bytes: &[u8], start: usize, len: usize) {
    for b in &bytes[start..start + len] {
        push_space_or_newline(out, *b);
    }
}

fn quoted_end(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        let b = bytes[i];
        i += 1;
        if b == b'\\' && i < bytes.len() {
            i += 1;
        } else if b == quote {
            break;
        }
    }
    i
}

fn is_char_literal_start(bytes: &[u8], i: usize) -> bool {
    !matches!(
        bytes.get(i + 1),
        Some(b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'0'..=b'9')
    )
}

fn raw_string_end(bytes: &[u8], i: usize) -> Option<usize> {
    let mut j = i;
    if bytes.get(j) == Some(&b'b') {
        j += 1;
    }
    if bytes.get(j) != Some(&b'r') {
        return None;
    }
    j += 1;
    let hashes_start = j;
    while bytes.get(j) == Some(&b'#') {
        j += 1;
    }
    if bytes.get(j) != Some(&b'"') {
        return None;
    }
    let hashes = j - hashes_start;
    j += 1;
    while j < bytes.len() {
        if bytes[j] == b'"'
            && bytes
                .get(j + 1..j + 1 + hashes)
                .is_some_and(|suffix| suffix.iter().all(|b| *b == b'#'))
        {
            return Some(j + 1 + hashes);
        }
        j += 1;
    }
    Some(bytes.len())
}

pub fn workspace_crates(workspace: &Path) -> Vec<String> {
    let cargo = workspace.join("Cargo.toml");
    let Ok(text) = std::fs::read_to_string(cargo) else {
        return vec![];
    };
    let mut crates = Vec::new();
    let mut in_package = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_package = t == "[package]";
        } else if in_package && t.starts_with("name") {
            if let Some(v) = t.split('=').nth(1) {
                crates.push(v.trim().trim_matches('"').to_string());
            }
        }
    }
    crates
}
pub fn rel_path(path: &Path, workspace: &Path) -> String {
    path.strip_prefix(workspace)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
pub fn module_path(path: &Path, workspace: &Path) -> String {
    let crate_src = nearest_crate_src(path, workspace).unwrap_or_else(|| workspace.join("src"));
    let rel = path
        .strip_prefix(&crate_src)
        .unwrap_or_else(|_| path.strip_prefix(workspace).unwrap_or(path))
        .to_string_lossy()
        .replace('\\', "/");
    let mut p = rel.trim_end_matches(".rs").to_string();
    if p == "lib" || p == "main" {
        return "crate".into();
    }
    if p.ends_with("/mod") {
        p.truncate(p.len() - 4);
    }
    p.replace('/', "::")
}

fn nearest_crate_src(path: &Path, workspace: &Path) -> Option<std::path::PathBuf> {
    for ancestor in path.ancestors() {
        if ancestor.file_name().and_then(|s| s.to_str()) == Some("src")
            && ancestor
                .parent()
                .is_some_and(|p| p.join("Cargo.toml").exists())
        {
            return Some(ancestor.to_path_buf());
        }
        if ancestor == workspace {
            break;
        }
    }
    None
}
