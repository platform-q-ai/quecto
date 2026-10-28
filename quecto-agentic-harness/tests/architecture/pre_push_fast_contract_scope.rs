//! Bounded workflow containers: required identities are never sought in scalar text.
// Root defaults are deliberately unsupported, so inherited shell semantics cannot
// bypass the executable-command allowlist. This is not a general YAML parser.
pub(super) fn workflow_jobs<'a>(lines: &'a [&'a str]) -> Result<&'a [&'a str], String> {
    let mut jobs_start = None;
    for (index, line) in lines.iter().enumerate() {
        match line.chars().next() {
            None | Some(' ' | '#') => continue,
            Some(_) => {}
        }
        let (key, value) = line
            .split_once(':')
            .ok_or_else(|| format!("malformed root field: {line}"))?;
        let supported = (["name", "run-name"].contains(&key) && value.starts_with(' '))
            || (["on", "concurrency", "jobs", "permissions", "env"].contains(&key)
                && value.is_empty());
        if supported {
            if key == "jobs" {
                match jobs_start {
                    None => jobs_start = Some(index),
                    Some(_) => return Err("duplicate root jobs mapping".to_owned()),
                }
            }
        } else {
            return Err(format!("unsupported root field: {key}"));
        }
    }
    for (key, allowed) in [
        ("permissions", "  contents: read"),
        ("env", "  PROJECT_DISPLAY_LABEL: quecto"),
    ] {
        let marker = format!("{key}:");
        let starts: Vec<_> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| **line == marker)
            .map(|(i, _)| i)
            .collect();
        match starts.as_slice() {
            [] => {}
            [start] => {
                let fields: Vec<_> = lines[start + 1..]
                    .iter()
                    .take_while(|line| matches!(line.chars().next(), None | Some(' ' | '#')))
                    .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
                    .copied()
                    .collect();
                if fields.as_slice() == [allowed] {
                    continue;
                }
                return Err(format!(
                    "unsupported root {key}: {fields:?}; only {allowed:?} is allowed"
                ));
            }
            _ => return Err(format!("duplicate root {key} mapping")),
        }
    }
    let start = jobs_start.ok_or("missing root jobs mapping")?;
    assert_eq!(lines[start], "jobs:", "supported jobs mapping marker");
    let tail = &lines[start + 1..];
    let end = tail
        .iter()
        .position(|line| match line.chars().next() {
            None | Some(' ' | '#') => false,
            Some(_) => true,
        })
        .unwrap_or(tail.len());
    Ok(&tail[..end])
}

pub(super) fn required_job<'a>(lines: &'a [&str], id: &str) -> Option<&'a [&'a str]> {
    let marker = format!("  {id}:");
    let starts: Vec<_> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| **line == marker)
        .map(|(i, _)| i)
        .collect();
    let [start] = starts.as_slice() else {
        return None;
    };
    let tail = &lines[start + 1..];
    let end = tail
        .iter()
        .position(
            |line| match line.strip_prefix("  ").and_then(|line| line.chars().next()) {
                None | Some(' ' | '#') => false,
                Some(_) => true,
            },
        )
        .unwrap_or(tail.len());
    Some(&tail[..end])
}
