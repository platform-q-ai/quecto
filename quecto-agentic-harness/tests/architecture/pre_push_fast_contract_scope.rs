//! Bounded workflow containers: required identities are never sought in scalar text.
// Root defaults are deliberately unsupported, so inherited shell semantics cannot
// bypass the executable-command allowlist. This is not a general YAML parser.
pub(super) fn workflow_jobs<'a>(lines: &'a [&'a str]) -> Option<&'a [&'a str]> {
    let mut jobs_start = None;
    for (index, line) in lines.iter().enumerate() {
        match line.chars().next() {
            None | Some(' ' | '#') => continue,
            Some(_) => {}
        }
        let (key, value) = line.split_once(':')?;
        let supported = (["name", "run-name"].contains(&key) && value.starts_with(' '))
            || (["on", "concurrency", "jobs"].contains(&key) && value.is_empty());
        if supported {
            if key == "jobs" {
                match jobs_start {
                    None => jobs_start = Some(index),
                    Some(_) => return None,
                }
            }
        } else {
            return None;
        }
    }
    let start = jobs_start?;
    assert_eq!(lines[start], "jobs:", "supported jobs mapping marker");
    let tail = &lines[start + 1..];
    let end = tail
        .iter()
        .position(|line| match line.chars().next() {
            None | Some(' ' | '#') => false,
            Some(_) => true,
        })
        .unwrap_or(tail.len());
    Some(&tail[..end])
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
