// Redirection, heredoc and brace-expansion handling for the shell parser
// (#1620). Split from `shell_parse.rs` to keep both files reviewable; this is
// the same `Parser` type, so private state is shared.

use super::{HEREDOC_PENDING, MAX_BRACE_EXPANSIONS, Parser};
use super::{Redirect, SimpleCommand, Word};

impl Parser<'_> {
    /// Handle `<` or `>` at `self.i` (not process substitution).
    pub(super) fn redirect_operator(&mut self) {
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
                // Heredoc delimiter: next word, quotes removed. Any quoting
                // or escaping makes the body literal; otherwise bash expands
                // `$(…)`, backticks and `${…}` inside it.
                while matches!(self.peek(0), Some(' ' | '\t')) {
                    self.i += 1;
                }
                let mut delim = String::new();
                let mut quoted = false;
                while let Some(ch) = self.peek(0) {
                    match ch {
                        ' ' | '\t' | '\n' | ';' | '|' | '&' | '<' | '>' => break,
                        '\'' | '"' => {
                            quoted = true;
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
                            quoted = true;
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
                self.pending_heredocs.push((delim, strip_tabs, quoted));
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
    pub(super) fn consume_heredoc_bodies(&mut self) {
        let pending = std::mem::take(&mut self.pending_heredocs);
        for (delim, strip_tabs, quoted) in pending {
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
            if !quoted {
                self.scan_expansions(&body);
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

    /// Expand unquoted `{a,b}` groups in argv and redirect targets into the
    /// cartesian product of simple commands bash would run. Oversized
    /// products are reported as unresolved rather than truncated.
    pub(super) fn brace_expand_command(&mut self, cmd: SimpleCommand) -> Vec<SimpleCommand> {
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
