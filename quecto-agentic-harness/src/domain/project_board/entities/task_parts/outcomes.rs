//! Semantic outcome settlement and indexed, reasoned exceptions. This grammar
//! belongs to the description, independently of any file or transport codec.
use super::super::super::value_objects::timestamp::Timestamp;
use super::super::claim::Identity;
use super::{Description, SchemaError, board_text, line};

/// A deliberate exception to a finished task's settled outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeException {
    pub reason: String,
    pub by: Identity,
    pub at: Timestamp,
}

pub const EXCEPTIONS: &str = "Outcome exceptions";
pub const EXCEPTION_HEADER: &str = "| Outcome index | Reason | By | At |\n|---|---|---|---|";

impl Description {
    /// Settle only implicit outcomes: explicit exceptions remain unmet.
    pub fn settle(&mut self) {
        for outcome in &mut self.done_when {
            outcome.done = outcome.exception.is_none();
        }
    }

    /// Change an existing outcome, never its text or position.
    pub fn set_outcome(
        &mut self,
        index: usize,
        exception: Option<OutcomeException>,
    ) -> Result<(), SchemaError> {
        let outcome = self
            .done_when
            .get_mut(index)
            .ok_or_else(|| SchemaError::new("outcome_index", "must name an existing outcome"))?;
        if let Some(exception) = &exception {
            board_text::entered_line("reason", &exception.reason)?;
            line(
                "reason",
                &exception.reason,
                super::super::task::MAX_REASON_CHARS,
            )?;
            exception.by.validate("by")?;
        }
        outcome.done = exception.is_none();
        outcome.exception = exception;
        Ok(())
    }

    /// Validate indexed exception cells from either description or artifact grammar.
    pub fn apply_exception_rows(&mut self, rows: &[Vec<String>]) -> Result<(), SchemaError> {
        let mut next = self.clone();
        let mut previous = None;
        for cells in rows {
            let [index, reason, by, at] = cells.as_slice() else {
                return Err(SchemaError::new("description", "four exception cells"));
            };
            let index: usize = index
                .parse()
                .ok()
                .filter(|index: &usize| index.to_string() == cells[0])
                .ok_or_else(|| SchemaError::new("outcome_index", "a canonical index"))?;
            let valid = previous.is_none_or(|before| before < index)
                && matches!(
                    next.done_when.get(index),
                    Some(super::Outcome { done: false, .. })
                );
            if valid {
                let (name, email) = by
                    .strip_suffix("`)")
                    .and_then(|by| by.rsplit_once(" (`"))
                    .ok_or_else(|| SchemaError::new("by", "Name (`email`)"))?;
                let exception = OutcomeException {
                    reason: reason.clone(),
                    by: Identity {
                        name: name.into(),
                        email: email.into(),
                    },
                    at: Timestamp::parse(at)?,
                };
                next.set_outcome(index, Some(exception))?;
                previous = Some(index);
            } else {
                return Err(SchemaError::new(
                    "outcome_index",
                    "distinct ordered existing unchecked outcomes",
                ));
            }
        }
        *self = next;
        Ok(())
    }

    /// Semantic cells, before Markdown escaping; persistence supplies its own table codec.
    pub fn exception_rows(&self) -> Vec<Vec<String>> {
        self.done_when
            .iter()
            .enumerate()
            .filter_map(|(index, outcome)| {
                outcome.exception.as_ref().map(|exception| {
                    vec![
                        index.to_string(),
                        exception.reason.clone(),
                        format!("{} (`{}`)", exception.by.name, exception.by.email),
                        exception.at.as_str().to_string(),
                    ]
                })
            })
            .collect()
    }
}

/// The supplemental table's text cells use literal backslashes and escaped pipes.
pub(super) fn exception_cell(text: &str) -> String {
    let mut out = String::new();
    let mut code = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '`' {
            let run = backticks(&mut chars);
            code = fence(code, run);
            out.push_str(&"`".repeat(run));
            continue;
        }
        if c == '|'
            || (c == '\\'
                && code.is_none()
                && chars.peek().is_none_or(|c| c.is_ascii_punctuation()))
        {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

pub(super) fn exception_cells(row: &str) -> Option<Vec<String>> {
    let row = row.strip_prefix("| ")?.strip_suffix(" |")?;
    let mut cells = vec![String::new()];
    let mut chars = row.chars().peekable();
    let mut code = None;
    while let Some(c) = chars.next() {
        match c {
            '`' => {
                let run = backticks(&mut chars);
                code = fence(code, run);
                cells.last_mut()?.push_str(&"`".repeat(run));
            }
            '\\' if chars.peek() == Some(&'|')
                || (code.is_none() && chars.peek() == Some(&'\\')) =>
            {
                cells.last_mut()?.push(chars.next()?);
            }
            '|' => cells.push(String::new()),
            _ => cells.last_mut()?.push(c),
        }
    }
    Some(
        cells
            .into_iter()
            .map(|cell| cell.trim().to_string())
            .collect(),
    )
}

/// A run of backticks opens/closes only a fence of the same width.
fn fence(open: Option<usize>, run: usize) -> Option<usize> {
    match open {
        None => Some(run),
        Some(width) if width == run => None,
        Some(width) => Some(width),
    }
}
fn backticks(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> usize {
    let mut run = 1;
    while chars.peek() == Some(&'`') {
        chars.next();
        run += 1;
    }
    run
}
