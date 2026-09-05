// Execution-aware shell command parsing for the dangerous-command denylist (#1620).
//
// This is deliberately NOT a full bash parser. It recovers the structure the
// denylist needs — which words sit in an executable position, which are literal
// arguments, which are redirect targets, which bytes are heredoc data — and it
// reports any syntax it cannot follow (dynamic command names, unbalanced
// quoting) so the caller can fall back to a conservative scan instead of
// silently letting the command through.

pub(crate) use super::ansi_c::expand_escape_sequence;
pub(crate) use super::shell_ast::{
    ParseBudget, Parsed, Redirect, SimpleCommand, Word, basename_lower,
};

/// Programs whose output on a pipe or substitution is treated as a remote fetch.
pub(crate) const FETCH_PROGRAMS: &[&str] = &["curl", "wget", "fetch", "aria2c"];

#[path = "shell_parse_redirects.rs"]
mod redirects;

const MAX_RECURSION_DEPTH: usize = 8;

/// Placeholder text for a heredoc whose body has not been read yet.
const HEREDOC_PENDING: &str = "\u{0}heredoc-pending";

/// Upper bound on simple commands produced by brace expansion of one command.
const MAX_BRACE_EXPANSIONS: usize = 64;

/// Parse a shell command line into its simple commands.
///
/// `budget` is shared across every nested parse of one command (substitutions,
/// `bash -c` scripts) so pipeline ids stay unique and total work stays bounded.
pub(crate) fn parse(command: &str, budget: &mut ParseBudget, depth: usize) -> Parsed {
    let mut out = Parsed::default();
    parse_into(command, &mut out, budget, depth);
    out
}

struct Parser<'a> {
    chars: Vec<char>,
    /// Byte offset of each char index in `src` (len + 1 entries).
    byte_at: Vec<usize>,
    src: &'a str,
    i: usize,
    out: &'a mut Parsed,
    budget: &'a mut ParseBudget,
    depth: usize,
    /// Enclosing `(`/`{` groups: the pipeline id and stage index to restore
    /// when the group closes.
    group_stack: Vec<(usize, usize)>,

    // Current simple-command state.
    cur: SimpleCommand,
    word: Word,
    in_word: bool,
    cmd_start: usize,
    /// A redirect operator was just read; the next word is its target.
    pending_redirect: Option<String>,
    /// Heredoc delimiters awaiting their body (processed at next newline):
    /// `(delimiter, strip_leading_tabs, delimiter_was_quoted)`. Each
    /// declaration also leaves a `HEREDOC_PENDING` marker redirect on its
    /// command so the body can be attached once it has been read.
    pending_heredocs: Vec<(String, bool, bool)>,
    pipeline: usize,
    pipe_index: usize,
    /// The next word may still be a `NAME=value` assignment.
    at_command_start: bool,
}

fn parse_into(command: &str, out: &mut Parsed, budget: &mut ParseBudget, depth: usize) {
    if depth > MAX_RECURSION_DEPTH {
        out.unresolved
            .push("nesting deeper than supported".to_string());
        return;
    }
    Parser::new(command, out, budget, depth).run();
}

impl<'a> Parser<'a> {
    fn new(
        command: &'a str,
        out: &'a mut Parsed,
        budget: &'a mut ParseBudget,
        depth: usize,
    ) -> Self {
        let chars: Vec<char> = command.chars().collect();
        let mut byte_at = Vec::with_capacity(chars.len() + 1);
        let mut acc = 0;
        for c in &chars {
            byte_at.push(acc);
            acc += c.len_utf8();
        }
        byte_at.push(acc);
        let pipeline = budget.next_pipeline;
        budget.next_pipeline += 1;
        Parser {
            chars,
            byte_at,
            src: command,
            i: 0,
            out,
            budget,
            depth,
            group_stack: Vec::new(),
            cur: SimpleCommand::default(),
            word: Word::new(),
            in_word: false,
            cmd_start: 0,
            pending_redirect: None,
            pending_heredocs: Vec::new(),
            pipeline,
            pipe_index: 0,
            at_command_start: true,
        }
    }

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
                '\\' => match self.peek(1) {
                    Some('\n') => self.i += 2, // line continuation, not a word
                    Some(n) => {
                        self.in_word = true;
                        self.word.push(n, true);
                        self.i += 2;
                    }
                    None => {
                        self.in_word = true;
                        self.i += 1;
                    }
                },
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
                    self.open_group();
                    self.i += 1;
                }
                ')' => {
                    self.end_word();
                    self.close_group();
                    self.i += 1;
                }
                '{' | '}' if !self.in_word => {
                    // Brace group boundary when standalone.
                    match self.peek(1) {
                        Some(n) if n.is_whitespace() || n == ';' || n == '&' || n == '|' => {
                            if c == '{' {
                                self.open_group();
                            } else {
                                self.close_group();
                            }
                            self.i += 1;
                        }
                        None => {
                            if c == '{' {
                                self.open_group();
                            } else {
                                self.close_group();
                            }
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

    /// `(` or standalone `{`: the group is one stage of the current pipeline;
    /// commands inside get their own pipeline but remember the outer one.
    fn open_group(&mut self) {
        self.end_command(false);
        self.group_stack.push((self.pipeline, self.pipe_index));
        self.pipeline = self.budget.next_pipeline;
        self.budget.next_pipeline += 1;
        self.pipe_index = 0;
    }

    /// `)` or standalone `}`: back to the enclosing pipeline, one stage on.
    fn close_group(&mut self) {
        self.end_command(false);
        if let Some((pipeline, stage)) = self.group_stack.pop() {
            self.pipeline = pipeline;
            self.pipe_index = stage + 1;
        }
    }

    /// Lex `text` the way bash treats an unquoted heredoc body or the inside
    /// of `${…}` / `$((…))`: quotes are literal but `$(…)`, backticks and
    /// nested `${…}` still expand. Nested commands found are appended to the
    /// output; the current word inherits `dynamic` / `fetch_subst`.
    fn scan_expansions(&mut self, text: &str) {
        let (dynamic, fetch) = {
            let mut sub = Parser::new(text, self.out, self.budget, self.depth + 1);
            if sub.depth > MAX_RECURSION_DEPTH {
                sub.unresolved("nesting deeper than supported");
                (true, false)
            } else {
                sub.in_word = true;
                sub.double_quoted_inner(false);
                (sub.word.dynamic, sub.word.fetch_subst)
            }
        };
        if dynamic {
            self.word.mark_dynamic();
        }
        if fetch {
            self.word.fetch_subst = true;
        }
    }

    /// Body of a `"..."` string; `self.i` is just past the opening quote.
    fn double_quoted(&mut self) {
        self.double_quoted_inner(true);
    }

    /// Double-quote lexing. With `terminate` false a `"` is an ordinary
    /// character and the scan runs to the end of the input (heredoc bodies,
    /// `${…}` interiors).
    fn double_quoted_inner(&mut self, terminate: bool) {
        let mut closed = !terminate;
        while self.i < self.chars.len() {
            match self.chars[self.i] {
                '"' if terminate => {
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
                    // Arithmetic `$(( … ))`: operands may still contain
                    // command substitutions.
                    self.i += 1;
                    let inner = self.balanced_parens();
                    if self.peek(0) == Some(')') {
                        self.i += 1;
                    }
                    self.word.mark_dynamic();
                    self.scan_expansions(&inner);
                    return;
                }
                let inner = self.balanced_parens();
                self.substitution(&inner);
            }
            Some('{') => {
                self.i += 2;
                let start = self.i;
                let mut depth = 1;
                while self.i < self.chars.len() && depth > 0 {
                    match self.chars[self.i] {
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        _ => {}
                    }
                    self.i += 1;
                }
                let inner: String = if depth > 0 {
                    self.unresolved("unterminated ${...}");
                    self.chars[start..].iter().collect()
                } else {
                    self.chars[start..self.i - 1].iter().collect()
                };
                self.word.mark_dynamic();
                // `${x:-$(reboot)}`: defaults and subscripts are expanded.
                self.scan_expansions(&inner);
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
                self.word.mark_dynamic();
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
        self.word.mark_dynamic();
        let before = self.out.commands.len();
        parse_into(inner, self.out, self.budget, self.depth + 1);
        if self.out.commands[before..].iter().any(|c| {
            c.program()
                .map(|p| FETCH_PROGRAMS.contains(&p.as_str()))
                .unwrap_or(false)
        }) {
            self.word.fetch_subst = true;
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
            cmd.enclosing = self.group_stack.iter().map(|(p, _)| *p).collect();
            let variants = self.brace_expand_command(cmd);
            if variants.len() > self.budget.remaining {
                self.out.unresolved.push("work budget exceeded".to_string());
                self.budget.remaining = 0;
            } else {
                self.budget.remaining -= variants.len();
                self.out.commands.extend(variants);
            }
            self.pipe_index += 1;
        }
        if new_pipeline {
            self.pipeline = self.budget.next_pipeline;
            self.budget.next_pipeline += 1;
            self.pipe_index = 0;
        }
        self.cmd_start = self.i;
        self.at_command_start = true;
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
