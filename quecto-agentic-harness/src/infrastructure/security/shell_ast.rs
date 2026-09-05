// Structural types produced by the execution-aware shell parser (#1620).

/// A single word of a simple command after quote removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Word {
    /// Word text with quotes removed and `$'...'` escapes expanded.
    pub text: String,
    /// The word contains an unquoted parameter expansion or command
    /// substitution, so its runtime value is not knowable statically.
    pub dynamic: bool,
    /// The word embeds a command or process substitution that runs a network
    /// fetch program (`curl`, `wget`, ...). Used by the pipe-to-shell rule to
    /// catch `bash <(curl …)` and `sh -c "$(curl …)"`.
    pub fetch_subst: bool,
    /// The word contains an unquoted glob metacharacter (`*`, `?`, `[`), so
    /// pathname expansion may rewrite it at runtime.
    pub glob: bool,
    /// Per-character flag, parallel to `text`: `true` when the character was
    /// quoted or escaped and is therefore exempt from brace/glob expansion.
    pub literal: Vec<bool>,
    /// Character index in `text` at which the first expansion occurred, so
    /// `text[..expansion_at]` is the literal prefix bash will keep verbatim
    /// (`/dev/sd$x` → `/dev/sd`). `None` for fully static words.
    pub expansion_at: Option<usize>,
}

impl Word {
    pub(crate) fn new() -> Self {
        Self {
            text: String::new(),
            dynamic: false,
            fetch_subst: false,
            glob: false,
            literal: Vec::new(),
            expansion_at: None,
        }
    }

    /// Record that an expansion (parameter, command or process substitution)
    /// occurs at the current end of `text`.
    pub(crate) fn mark_dynamic(&mut self) {
        self.dynamic = true;
        if self.expansion_at.is_none() {
            self.expansion_at = Some(self.text.chars().count());
        }
    }

    /// The part of the word that is known statically: the whole text for a
    /// static word, or the literal prefix before the first expansion.
    pub(crate) fn static_prefix(&self) -> &str {
        match self.expansion_at {
            None => &self.text,
            Some(n) => {
                let end = self
                    .text
                    .char_indices()
                    .nth(n)
                    .map_or(self.text.len(), |(b, _)| b);
                &self.text[..end]
            }
        }
    }

    /// A word carrying only literal `text` (used for synthesised words).
    pub(crate) fn literal(text: &str) -> Self {
        Self {
            literal: vec![true; text.chars().count()],
            text: text.to_string(),
            dynamic: false,
            fetch_subst: false,
            glob: false,
            expansion_at: None,
        }
    }

    pub(crate) fn push(&mut self, c: char, literal: bool) {
        self.text.push(c);
        self.literal.push(literal);
        if !literal && matches!(c, '*' | '?' | '[') {
            self.glob = true;
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.text.is_empty() && !self.dynamic
    }

    /// Bash brace expansion of this word (`a{b,c}` → `ab`, `ac`), honouring
    /// quoting. Returns `None` when the word has no unquoted `{…,…}` group.
    /// Sequence ranges (`{1..3}`) are not expanded; the caller treats the
    /// word as dynamic instead. Bounded to `max` results.
    pub(crate) fn brace_alternatives(&self, max: usize) -> Option<Vec<String>> {
        let chars: Vec<(char, bool)> = self
            .text
            .chars()
            .zip(self.literal.iter().copied())
            .collect();
        let out = expand_braces(&chars, max)?;
        (out.len() > 1).then_some(out)
    }
}

/// Recursive brace expansion over `(char, literal)` pairs.
fn expand_braces(chars: &[(char, bool)], max: usize) -> Option<Vec<String>> {
    // Find the first unquoted `{` that has a matching unquoted `}` with at
    // least one top-level unquoted `,` between them.
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ('{', false)
            && let Some((close, commas)) = find_brace_group(chars, i)
            && !commas.is_empty()
        {
            let prefix: String = chars[..i].iter().map(|(c, _)| *c).collect();
            let mut results = Vec::new();
            let mut start = i + 1;
            for &comma in commas.iter().chain(std::iter::once(&close)) {
                let alt = &chars[start..comma];
                for tail in expand_braces(&chars[close + 1..], max)
                    .unwrap_or_else(|| vec![chars[close + 1..].iter().map(|(c, _)| *c).collect()])
                {
                    for inner in expand_braces(alt, max)
                        .unwrap_or_else(|| vec![alt.iter().map(|(c, _)| *c).collect()])
                    {
                        results.push(format!("{prefix}{inner}{tail}"));
                        if results.len() > max {
                            return Some(results);
                        }
                    }
                }
                start = comma + 1;
            }
            return Some(results);
        }
        i += 1;
    }
    None
}

/// For an unquoted `{` at `open`, return the index of its matching `}` and
/// the indices of the top-level unquoted commas inside it.
fn find_brace_group(chars: &[(char, bool)], open: usize) -> Option<(usize, Vec<usize>)> {
    let mut depth = 0usize;
    let mut commas = Vec::new();
    for (j, &(c, literal)) in chars.iter().enumerate().skip(open) {
        if literal {
            continue;
        }
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((j, commas));
                }
            }
            ',' if depth == 1 => commas.push(j),
            _ => {}
        }
    }
    None
}

/// An unquoted redirection attached to a simple command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Redirect {
    /// The operator as written (`>`, `>>`, `>|`, `&>`, `<`, `<<<`, ...).
    pub op: String,
    /// The target word.
    pub target: Word,
}

impl Redirect {
    /// True for operators that open the target for writing.
    pub fn writes_target(&self) -> bool {
        matches!(self.op.as_str(), ">" | ">>" | ">|" | "&>" | "&>>" | ">&")
    }
}

/// One simple command: an argv plus its redirections.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SimpleCommand {
    /// Arguments including the program word at index 0. Leading `NAME=value`
    /// assignments are not included.
    pub words: Vec<Word>,
    pub redirects: Vec<Redirect>,
    /// Identifier of the pipeline this command belongs to. Commands joined by
    /// `|` share an id; `;`, `&&`, `||`, `&` and newlines start a new one.
    pub pipeline: usize,
    /// Position within its pipeline (0 = leftmost).
    pub pipe_index: usize,
    /// Set when this "command" is actually a function definition `name() {`.
    pub function_def: bool,
    /// Pipeline ids of the subshell / brace groups enclosing this command,
    /// outermost first. `(curl x) | sh` puts `curl` in its own pipeline but
    /// records the outer one here so `sh` can see it upstream.
    pub enclosing: Vec<usize>,
    /// Source text of this simple command, for error reporting.
    pub site: String,
}

impl SimpleCommand {
    /// Program name with any directory prefix stripped, lowercased.
    pub fn program(&self) -> Option<String> {
        self.words.first().map(|w| basename_lower(&w.text))
    }
}

/// Lowercased final path component of `text` (`/usr/bin/RM` → `rm`).
pub(crate) fn basename_lower(text: &str) -> String {
    text.rsplit('/').next().unwrap_or(text).to_ascii_lowercase()
}

/// Result of parsing a command line.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Parsed {
    pub commands: Vec<SimpleCommand>,
    /// Some construct could not be resolved statically: an unbalanced quote
    /// or bracket, nesting deeper than supported, or the work budget being
    /// exhausted. The caller must not treat the structural result as complete.
    pub unresolved: Vec<String>,
}

/// Total simple commands one `validate_command` call may produce across
/// brace expansion and nested-script parsing. Bounds the cost of adversarial
/// input such as `{bash,…}×64 -c '{bash,…}×64 -c …'`.
pub(crate) const WORK_BUDGET: usize = 2048;

/// Shared counters threaded through every nested parse of one command.
#[derive(Debug)]
pub(crate) struct ParseBudget {
    /// Next unused pipeline id.
    pub next_pipeline: usize,
    /// Simple commands that may still be emitted.
    pub remaining: usize,
}

impl Default for ParseBudget {
    fn default() -> Self {
        Self {
            next_pipeline: 0,
            remaining: WORK_BUDGET,
        }
    }
}
