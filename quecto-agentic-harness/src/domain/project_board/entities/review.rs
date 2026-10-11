//! A review: the rounds of findings on one change (#2481's artifact
//! model), exactly one review per change. Each round is pinned to the
//! commit it judged, so a branch rewritten later never changes what was
//! reviewed; it names its reviewer (a person or an agent), its verdict and
//! its findings. A review is begun by its first round, so it always has
//! one, and its state is its last round's verdict. It is stored in one
//! file named by its task and change (M3), so a change has no second.
use super::super::value_objects::code_ref::CommitId;
use super::super::value_objects::schema_error::{SchemaError, filled_markdown};
use super::super::value_objects::slug::Slug;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::{ReviewVerdict, Severity};
use super::claim::Identity;
use super::task_parts::{MAX_FINDING_CHARS, location};

/// The most rounds a review holds.
pub const MAX_ROUNDS: usize = 64;
/// The most findings a round holds.
pub const MAX_FINDINGS: usize = 128;
/// All of a review's text together, in UTF-8 bytes: its findings and
/// their places. This budget keeps every review inside the one file it is
/// stored in, with room to spare (its codec, M2, proves the fit).
pub const MAX_REVIEW_TEXT_BYTES: usize = 320 * 1024;

/// One finding of a round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// `path:line` or `path:line:col`, when it has a place.
    pub location: Option<String>,
    pub severity: Severity,
    /// Markdown.
    pub text: String,
}

/// One round: who judged which commit, how, and what they found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Round {
    /// The commit judged.
    pub commit: CommitId,
    pub reviewer: Identity,
    pub at: Timestamp,
    pub verdict: ReviewVerdict,
    pub findings: Vec<Finding>,
}

/// A review's content, unchecked: what [`Review::new`] validates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewFields {
    /// The task whose change it reviews, and that change's number.
    pub task: Slug,
    pub change: u32,
    /// Oldest first; at least one.
    pub rounds: Vec<Round>,
    pub created: Timestamp,
    pub updated: Timestamp,
}

impl ReviewFields {
    /// The UTF-8 bytes of every free text in the review: its findings and
    /// their places.
    pub fn text_bytes(&self) -> usize {
        self.rounds
            .iter()
            .flat_map(|round| &round.findings)
            .map(|finding| finding.text.len() + finding.location.as_ref().map_or(0, String::len))
            .sum()
    }
}

/// A review whose fields keep every rule of its schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review(ReviewFields);

impl Review {
    pub fn new(fields: ReviewFields) -> Result<Self, SchemaError> {
        validate(&fields)?;
        Ok(Self(fields))
    }

    pub fn fields(&self) -> &ReviewFields {
        &self.0
    }

    pub fn into_fields(self) -> ReviewFields {
        self.0
    }

    /// Its last round's verdict.
    pub fn state(&self) -> ReviewVerdict {
        match self.0.rounds.last() {
            Some(round) => round.verdict,
            None => unreachable!("a review is built with at least one round"),
        }
    }
}

fn validate(review: &ReviewFields) -> Result<(), SchemaError> {
    match review.created <= review.updated {
        true => {}
        false => return Err(SchemaError::new("updated", "must not be before created")),
    }
    match review.rounds.len() {
        1..=MAX_ROUNDS => {}
        _ => {
            return Err(SchemaError::new(
                "rounds",
                format!("must hold from 1 to {MAX_ROUNDS} rounds"),
            ));
        }
    }
    let mut last = &review.created;
    for (index, round) in review.rounds.iter().enumerate() {
        let field = format!("rounds/{index}");
        validate_round(&field, round)?;
        // Rounds happen in order, while the review exists.
        match *last <= round.at && round.at <= review.updated {
            true => last = &round.at,
            false => {
                return Err(SchemaError::new(
                    format!("{field}/at"),
                    "must follow the round before, by the review's last change",
                ));
            }
        }
    }
    let text = review.text_bytes();
    match text <= MAX_REVIEW_TEXT_BYTES {
        true => Ok(()),
        false => Err(SchemaError::new(
            "text",
            format!("the review's text is {text} bytes; the most is {MAX_REVIEW_TEXT_BYTES}"),
        )),
    }
}

fn validate_round(field: &str, round: &Round) -> Result<(), SchemaError> {
    round.reviewer.validate(&format!("{field}/reviewer"))?;
    match round.findings.len() {
        0..=MAX_FINDINGS => {}
        _ => {
            return Err(SchemaError::new(
                format!("{field}/findings"),
                format!("must hold at most {MAX_FINDINGS} findings"),
            ));
        }
    }
    for (index, finding) in round.findings.iter().enumerate() {
        let at = format!("{field}/findings/{index}");
        if let Some(place) = &finding.location {
            location(&format!("{at}/location"), place)?;
        }
        filled_markdown(&format!("{at}/text"), &finding.text, MAX_FINDING_CHARS)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "review_tests.rs"]
mod tests;
