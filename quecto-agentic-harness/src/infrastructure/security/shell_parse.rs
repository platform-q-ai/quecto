// Execution-aware shell command parsing for the dangerous-command denylist (#1620).
//
// This is deliberately NOT a full bash parser. It recovers the structure the
// denylist needs — which words sit in an executable position, which are literal
// arguments, which are redirect targets, which bytes are heredoc data — and it
// reports any syntax it cannot follow (dynamic command names, unbalanced
// quoting) so the caller can fall back to a conservative scan instead of
// silently letting the command through.

pub(crate) use super::ansi_c::expand_escape_sequence;
pub(crate) use super::shell_ast::{Parsed, Redirect, SimpleCommand, Word, basename_lower};

/// Programs whose output on a pipe or substitution is treated as a remote fetch.
pub(crate) const FETCH_PROGRAMS: &[&str] = &["curl", "wget", "fetch", "aria2c"];

const MAX_RECURSION_DEPTH: usize = 8;

/// Placeholder text for a heredoc whose body has not been read yet.
const HEREDOC_PENDING: &str = "\u{0}heredoc-pending";

/// Upper bound on simple commands produced by brace expansion of one command.
const MAX_BRACE_EXPANSIONS: usize = 64;

/// Parse a shell command line into its simple commands.
///
/// `next_pipeline` is a shared counter so that pipeline ids stay unique when
/// nested scripts (e.g. the argument of `bash -c`) are parsed separately.
pub(crate) fn parse(command: &str, next_pipeline: &mut usize, depth: usize) -> Parsed {
    let mut out = Parsed::default();
    parse_into(command, &mut out, next_pipeline, depth);
    out
}

struct Parser<'a> {
    chars: Vec<char>,
    /// Byte offset of each char index in `src` (len + 1 entries).
    byte_at: Vec<usize>,
    src: &'a str,
    i: usize,
    out: &'a mut Parsed,
    next_pipeline: &'a mut usize,
    depth: usize,

    // Current simple-command state.
    cur: SimpleCommand,
    word: Word,
    in_word: bool,
    cmd_start: usize,
    /// A redirect operator was just read; the next word is its target.
    pending_redirect: Option<String>,
    /// Heredoc delimiters awaiting their body (processed at next newline).
    /// Each declaration also leaves a `HEREDOC_PENDING` marker redirect on its
    /// command so the body can be attached once it has been read.
    pending_heredocs: Vec<(String, bool)>,
    pipeline: usize,
    pipe_index: usize,
    /// The next word may still be a `NAME=value` assignment.
    at_command_start: bool,
}

fn parse_into(command: &str, out: &mut Parsed, next_pipeline: &mut usize, depth: usize) {
    if depth > MAX_RECURSION_DEPTH {
        out.unresolved
            .push("nesting deeper than supported".to_string());
        return;
    }
    let pipeline = *next_pipeline;
    *next_pipeline += 1;
    let chars: Vec<char> = command.chars().collect();
    let mut byte_at = Vec::with_capacity(chars.len() + 1);
    let mut acc = 0;
    for c in &chars {
        byte_at.push(acc);
        acc += c.len_utf8();
    }
    byte_at.push(acc);
    let mut p = Parser {
        chars,
        byte_at,
        src: command,
        i: 0,
        out,
        next_pipeline,
        depth,
        cur: SimpleCommand::default(),
        word: Word::new(),
        in_word: false,
        cmd_start: 0,
        pending_redirect: None,
        pending_heredocs: Vec::new(),
        pipeline,
        pipe_index: 0,
        at_command_start: true,
    };
    p.run();
}

impl Parser<'_> {
    fn peek(&self, off: usize) -> Option<char> {
        self.chars.get(self.i + off).copied()
    }

    fn byte_offset(&self, char_idx: usize) -> usize {
        self.byte_at[char_idx.min(self.chars.len())]
    }

    fn run(&mut self) {
        while self.i < self.chars.len() {
            let c = self.chars[self.i];
            match c {
                ' ' | '\t' => {
                    self.end_word();
                    self.i += 1;
                }
                '\n' => {
                    self.end_word();
                    self.end_command(true);
                    self.i += 1;
                    self.consume_heredoc_bodies();
                }
                ';' => {
                    self.end_word();
                    self.end_command(true);
                    self.i += 1;
                    // `;;` in case statements — treat as separator too.
                }
                '&' => {
                    if self.peek(1) == Some('&') {
                        self.end_word();
                        self.end_command(true);
                        self.i += 2;
                    } else if self.peek(1) == Some('>') {
                        // `&>` / `&>>` redirect
                        self.end_word();
                        let op = if self.peek(2) == Some('>') {
                            self.i += 3;
                            "&>>"
                        } else {
                            self.i += 2;
                            "&>"
                        };
                        self.pending_redirect = Some(op.to_string());
                    } else {
                        self.end_word();
                        self.end_command(true);
                        self.i += 1;
                    }
                }
                '|' => {
                    self.end_word();
                    if self.peek(1) == Some('|') {
                        self.end_command(true);
                        self.i += 2;
                    } else {
                        self.end_command(false);
                        self.i += if self.peek(1) == Some('&') { 2 } else { 1 };
                    }
                }
                '\'' => {
                    self.i += 1;
                    self.in_word = true;
                    let mut closed = false;
                    while self.i < self.chars.len() {
                        if self.chars[self.i] == '\'' {
                            closed = true;
                            self.i += 1;
                            break;
                        }
                        self.word.push(self.chars[self.i], true);
                        self.i += 1;
                    }
                    if !closed {
                        self.unresolved("unterminated single quote");
                    }
                }
                '"' => {
                    self.i += 1;
                    self.in_word = true;
                    self.double_quoted();
                }
                '\\' => {
                    self.in_word = true;
                    match self.peek(1) {
                        Some('\n') => self.i += 2, // line continuation
                        Some(n) => {
                            self.word.push(n, true);
                            self.i += 2;
                        }
                        None => self.i += 1,
                    }
                }
                '$' => {
                    self.in_word = true;
                    self.dollar(false);
                }
                '`' => {
                    self.in_word = true;
                    self.i += 1;
                    let start = self.i;
                    let mut closed = false;
                    while self.i < self.chars.len() {
                        if self.chars[self.i] == '\\' {
                            self.i += 2;
                            continue;
                        }
                        if self.chars[self.i] == '`' {
                            closed = true;
                            break;
                        }
                        self.i += 1;
                    }
                    let inner: String = self.chars[start..self.i.min(self.chars.len())]
                        .iter()
                        .collect();
                    if closed {
                        self.i += 1;
                    } else {
                        self.unresolved("unterminated backtick");
                    }
                    self.substitution(&inner);
                }
                '<' | '>' => {
                    if self.peek(1) == Some('(') {
                        // Process substitution.
                        self.in_word = true;
                        self.i += 2;
                        let inner = self.balanced_parens();
                        self.substitution(&inner);
                        continue;
                    }
                    self.redirect_operator();
                }
                '#' if !self.in_word => {
                    while self.i < self.chars.len() && self.chars[self.i] != '\n' {
                        self.i += 1;
                    }
                }
                '(' => {
                    if self.in_word && !self.word.dynamic && self.peek(1) == Some(')') {
                        // `name()` function definition.
                        self.i += 2;
                        self.end_word();
                        self.cur.function_def = true;
                        self.end_command(true);
                        continue;
                    }
                    self.end_word();
                    self.end_command(true);
                    self.i += 1;
                }
                ')' => {
                    self.end_word();
                    self.end_command(true);
                    self.i += 1;
                }
                '{' | '}' if !self.in_word => {
                    // Brace group boundary when standalone.
                    match self.peek(1) {
                        Some(n) if n.is_whitespace() || n == ';' || n == '&' || n == '|' => {
                            self.end_command(true);
                            self.i += 1;
                        }
                        None => {
                            self.end_command(true);
                            self.i += 1;
                        }
                        _ => {
                            self.in_word = true;
                            self.word.push(c, false);
                            self.i += 1;
                        }
                    }
                }
                _ => {
                    self.in_word = true;
                    self.word.push(c, false);
                    self.i += 1;
                }
            }
        }
        self.end_word();
        self.end_command(true);
        if !self.pending_heredocs.is_empty() {
            // Heredoc declared but body never started: nothing executable there.
            self.pending_heredocs.clear();
        }
    }

    fn unresolved(&mut self, what: &str) {
        self.out.unresolved.push(what.to_string());
    }

    /// Body of a `"..."` string; `self.i` is just past the opening quote.
    fn double_quoted(&mut self) {
        let mut closed = false;
        while self.i < self.chars.len() {
            match self.chars[self.i] {
                '"' => {
                    closed = true;
                    self.i += 1;
                    break;
                }
                '\\' => match self.peek(1) {
                    Some(n @ ('"' | '\\' | '$' | '`')) => {
                        self.word.push(n, true);
                        self.i += 2;
                    }
                    Some('\n') => self.i += 2,
                    Some(_) | None => {
                        self.word.push('\\', true);
                        self.i += 1;
                    }
                },
                '$' => self.dollar(true),
                '`' => {
                    self.i += 1;
                    let start = self.i;
                    while self.i < self.chars.len() && self.chars[self.i] != '`' {
                        self.i += 1;
                    }
                    let inner: String = self.chars[start..self.i].iter().collect();
                    if self.i < self.chars.len() {
                        self.i += 1;
                    } else {
                        self.unresolved("unterminated backtick");
                    }
                    self.substitution(&inner);
                }
                c => {
                    self.word.push(c, true);
                    self.i += 1;
                }
            }
        }
        if !closed {
            self.unresolved("unterminated double quote");
        }
    }

    /// Handle `$` at `self.i`.
    fn dollar(&mut self, in_dq: bool) {
        match self.peek(1) {
            Some('\'') if !in_dq => {
                // ANSI-C quoting: expand escapes into literal text.
                self.i += 2;
                let mut closed = false;
                while self.i < self.chars.len() {
                    let c = self.chars[self.i];
                    if c == '\'' {
                        closed = true;
                        self.i += 1;
                        break;
                    }
                    if c == '\\' && self.i + 1 < self.chars.len() {
                        let (ch, adv) = expand_escape_sequence(&self.chars, self.i);
                        if let Some(ch) = ch {
                            self.word.push(ch, true);
                        }
                        self.i += adv;
                    } else {
                        self.word.push(c, true);
                        self.i += 1;
                    }
                }
                if !closed {
                    self.unresolved("unterminated $'...' quote");
                }
            }
            Some('(') => {
                self.i += 2;
                if self.peek(0) == Some('(') {
                    // Arithmetic `$(( ... ))`: skip to matching `))`.
                    self.i += 1;
                    let _ = self.balanced_parens();
                    if self.peek(0) == Some(')') {
                        self.i += 1;
                    }
                    self.word.dynamic = true;
                    return;
                }
                let inner = self.balanced_parens();
                self.substitution(&inner);
            }
            Some('{') => {
                self.i += 2;
                let mut depth = 1;
                while self.i < self.chars.len() && depth > 0 {
                    match self.chars[self.i] {
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        _ => {}
                    }
                    self.i += 1;
                }
                if depth > 0 {
                    self.unresolved("unterminated ${...}");
                }
                self.word.dynamic = true;
            }
            Some(c) if c.is_ascii_alphanumeric() || c == '_' || "@*#?-$!".contains(c) => {
                self.i += 2;
                if c.is_ascii_alphabetic() || c == '_' {
                    while self.i < self.chars.len()
                        && (self.chars[self.i].is_ascii_alphanumeric() || self.chars[self.i] == '_')
                    {
                        self.i += 1;
                    }
                }
                self.word.dynamic = true;
            }
            _ => {
                self.word.push('$', false);
                self.i += 1;
            }
        }
    }

    /// Consume up to the `)` matching an already-consumed `(`, honouring
    /// nested parens and quotes. Returns the inner text.
    fn balanced_parens(&mut self) -> String {
        let start = self.i;
        let mut depth = 1usize;
        while self.i < self.chars.len() {
            match self.chars[self.i] {
                '\\' => {
                    self.i += 2;
                    continue;
                }
                '\'' => {
                    self.i += 1;
                    while self.i < self.chars.len() && self.chars[self.i] != '\'' {
                        self.i += 1;
                    }
                }
                '"' => {
                    self.i += 1;
                    while self.i < self.chars.len() && self.chars[self.i] != '"' {
                        if self.chars[self.i] == '\\' {
                            self.i += 1;
                        }
                        self.i += 1;
                    }
                }
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        let inner: String = self.chars[start..self.i].iter().collect();
                        self.i += 1;
                        return inner;
                    }
                }
                _ => {}
            }
            self.i += 1;
        }
        self.unresolved("unbalanced parenthesis");
        self.chars[start..].iter().collect()
    }

    /// Parse a command/process substitution body as nested commands.
    fn substitution(&mut self, inner: &str) {
        self.word.dynamic = true;
        let before = self.out.commands.len();
        parse_into(inner, self.out, self.next_pipeline, self.depth + 1);
        if self.out.commands[before..].iter().any(|c| {
            c.program()
                .map(|p| FETCH_PROGRAMS.contains(&p.as_str()))
                .unwrap_or(false)
        }) {
            self.word.fetch_subst = true;
        }
    }

    /// Handle `<` or `>` at `self.i` (not process substitution).
    fn redirect_operator(&mut self) {
        // A word made only of digits immediately before the operator is an fd.
        if self.in_word
            && !self.word.text.is_empty()
            && self.word.text.chars().all(|c| c.is_ascii_digit())
            && !self.word.dynamic
        {
            self.word = Word::new();
            self.in_word = false;
        } else {
            self.end_word();
        }
        let c = self.chars[self.i];
        let mut op = String::new();
        op.push(c);
        self.i += 1;
        if c == '<' {
            if self.peek(0) == Some('<') {
                self.i += 1;
                if self.peek(0) == Some('<') {
                    self.i += 1;
                    self.pending_redirect = Some("<<<".to_string());
                    return;
                }
                let strip_tabs = if self.peek(0) == Some('-') {
                    self.i += 1;
                    true
                } else {
                    false
                };
                // Heredoc delimiter: next word, quotes removed.
                while matches!(self.peek(0), Some(' ' | '\t')) {
                    self.i += 1;
                }
                let mut delim = String::new();
                while let Some(ch) = self.peek(0) {
                    match ch {
                        ' ' | '\t' | '\n' | ';' | '|' | '&' | '<' | '>' => break,
                        '\'' | '"' => {
                            self.i += 1;
                            while let Some(q) = self.peek(0) {
                                self.i += 1;
                                if q == ch {
                                    break;
                                }
                                delim.push(q);
                            }
                        }
                        '\\' => {
                            if let Some(n) = self.peek(1) {
                                delim.push(n);
                            }
                            self.i += 2;
                        }
                        _ => {
                            delim.push(ch);
                            self.i += 1;
                        }
                    }
                }
                if delim.is_empty() {
                    self.unresolved("heredoc without delimiter");
                }
                self.pending_heredocs.push((delim, strip_tabs));
                self.cur.redirects.push(Redirect {
                    op: "<<".to_string(),
                    target: Word::literal(HEREDOC_PENDING),
                });
                return;
            }
            if self.peek(0) == Some('>') {
                op.push('>');
                self.i += 1;
            } else if self.peek(0) == Some('&') {
                op.push('&');
                self.i += 1;
            }
        } else {
            match self.peek(0) {
                Some('>') => {
                    op.push('>');
                    self.i += 1;
                }
                Some('|') => {
                    op.push('|');
                    self.i += 1;
                }
                Some('&') => {
                    op.push('&');
                    self.i += 1;
                }
                _ => {}
            }
        }
        self.pending_redirect = Some(op);
    }

    /// Skip heredoc bodies that begin at `self.i` (just after a newline).
    fn consume_heredoc_bodies(&mut self) {
        let pending = std::mem::take(&mut self.pending_heredocs);
        for (delim, strip_tabs) in pending {
            let mut body = String::new();
            let mut found = false;
            while self.i < self.chars.len() {
                let line_start = self.i;
                while self.i < self.chars.len() && self.chars[self.i] != '\n' {
                    self.i += 1;
                }
                let line: String = self.chars[line_start..self.i].iter().collect();
                if self.i < self.chars.len() {
                    self.i += 1; // newline
                }
                let cmp = if strip_tabs {
                    line.trim_start_matches('\t')
                } else {
                    line.as_str()
                };
                if cmp == delim {
                    found = true;
                    break;
                }
                body.push_str(cmp);
                body.push('\n');
            }
            self.attach_heredoc_body(body);
            if !found {
                // Unterminated heredoc swallows the rest of the input as data.
                break;
            }
        }
    }

    /// Replace the earliest pending heredoc marker with its body.
    fn attach_heredoc_body(&mut self, body: String) {
        let marker = self
            .out
            .commands
            .iter_mut()
            .flat_map(|c| c.redirects.iter_mut())
            .find(|r| r.op == "<<" && r.target.text == HEREDOC_PENDING);
        if let Some(r) = marker {
            r.target = Word::literal(&body);
        }
    }

    fn end_word(&mut self) {
        if !self.in_word {
            return;
        }
        let word = std::mem::replace(&mut self.word, Word::new());
        self.in_word = false;
        if let Some(op) = self.pending_redirect.take() {
            self.cur.redirects.push(Redirect { op, target: word });
            return;
        }
        if self.at_command_start && is_assignment(&word) {
            // `NAME=value` prefix: environment for the command, not argv.
            return;
        }
        if word.is_empty() {
            return;
        }
        self.at_command_start = false;
        self.cur.words.push(word);
    }

    fn end_command(&mut self, new_pipeline: bool) {
        if let Some(op) = self.pending_redirect.take() {
            // Dangling redirect with no target — syntax error in bash; record.
            self.out
                .unresolved
                .push(format!("redirect `{op}` without target"));
        }
        let end = self.byte_offset(self.i);
        let start = self.byte_offset(self.cmd_start).min(end);
        let mut cmd = std::mem::take(&mut self.cur);
        if !cmd.words.is_empty() || !cmd.redirects.is_empty() {
            cmd.site = self.src[start..end]
                .trim_matches(|c: char| c.is_whitespace() || ";|&(){}".contains(c))
                .to_string();
            cmd.pipeline = self.pipeline;
            cmd.pipe_index = self.pipe_index;
            // `function name { … }` — keyword form of a definition.
            if cmd.words.len() >= 2 && cmd.words[0].text == "function" && !cmd.words[0].dynamic {
                cmd.function_def = true;
                cmd.words = vec![cmd.words[1].clone()];
            }
            if let Some(first) = cmd.words.first()
                && !cmd.function_def
            {
                if first.dynamic {
                    self.out
                        .unresolved
                        .push(format!("dynamic command name in `{}`", cmd.site));
                } else if first.glob {
                    self.out.glob_commands.push(cmd.site.clone());
                }
            }
            for variant in self.brace_expand_command(cmd) {
                self.out.commands.push(variant);
            }
            self.pipe_index += 1;
        }
        if new_pipeline {
            self.pipeline = *self.next_pipeline;
            *self.next_pipeline += 1;
            self.pipe_index = 0;
        }
        self.cmd_start = self.i;
        self.at_command_start = true;
    }

    /// Expand unquoted `{a,b}` groups in argv and redirect targets into the
    /// cartesian product of simple commands bash would run. Oversized
    /// products are reported as unresolved rather than truncated.
    fn brace_expand_command(&mut self, cmd: SimpleCommand) -> Vec<SimpleCommand> {
        let mut variants = vec![cmd];
        let n_words = variants[0].words.len();
        let n_redirects = variants[0].redirects.len();
        for slot in 0..n_words + n_redirects {
            let word_of = |c: &SimpleCommand| -> Word {
                if slot < n_words {
                    c.words[slot].clone()
                } else {
                    c.redirects[slot - n_words].target.clone()
                }
            };
            let Some(alts) = word_of(&variants[0]).brace_alternatives(MAX_BRACE_EXPANSIONS) else {
                continue;
            };
            let mut next = Vec::with_capacity(variants.len() * alts.len());
            for v in &variants {
                for alt in &alts {
                    let mut nv = v.clone();
                    let target = if slot < n_words {
                        &mut nv.words[slot]
                    } else {
                        &mut nv.redirects[slot - n_words].target
                    };
                    let keep = Word {
                        text: alt.clone(),
                        literal: vec![true; alt.chars().count()],
                        ..target.clone()
                    };
                    *target = keep;
                    next.push(nv);
                }
            }
            if next.len() > MAX_BRACE_EXPANSIONS {
                self.out.unresolved.push(format!(
                    "brace expansion too large in `{}`",
                    variants[0].site
                ));
                return variants;
            }
            variants = next;
        }
        variants
    }
}

fn is_assignment(word: &Word) -> bool {
    let Some(eq) = word.text.find('=') else {
        return false;
    };
    let name = &word.text[..eq];
    let name = name.strip_suffix('+').unwrap_or(name);
    !name.is_empty()
        && name
            .chars()
            .enumerate()
            .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
}
