// Bash ANSI-C (`$'...'`) escape expansion shared by the parser and the
// legacy fallback scan.

/// Bash `$'...'` escape expansion for a single escape starting at `chars[i]`
/// (the backslash). Returns the produced character (if any) and how many
/// chars were consumed.
pub(crate) fn expand_escape_sequence(chars: &[char], i: usize) -> (Option<char>, usize) {
    let next = chars[i + 1];
    let hex_run = |start: usize, max: usize| -> (u32, usize) {
        let mut val = 0u32;
        let mut n = 0;
        while n < max && start + n < chars.len() {
            match chars[start + n].to_digit(16) {
                Some(d) => {
                    val = val * 16 + d;
                    n += 1;
                }
                None => break,
            }
        }
        (val, n)
    };
    match next {
        'x' => {
            let (val, n) = hex_run(i + 2, 2);
            if n == 0 {
                (Some('x'), 2)
            } else {
                (char::from_u32(val), 2 + n)
            }
        }
        'u' => {
            let (val, n) = hex_run(i + 2, 4);
            if n == 0 {
                (Some('u'), 2)
            } else {
                (char::from_u32(val), 2 + n)
            }
        }
        'U' => {
            let (val, n) = hex_run(i + 2, 8);
            if n == 0 {
                (Some('U'), 2)
            } else {
                (char::from_u32(val), 2 + n)
            }
        }
        '0'..='7' => {
            let mut val = 0u32;
            let mut n = 0;
            while n < 3 && i + 1 + n < chars.len() {
                match chars[i + 1 + n].to_digit(8) {
                    Some(d) => {
                        val = val * 8 + d;
                        n += 1;
                    }
                    None => break,
                }
            }
            (char::from_u32(val), 1 + n)
        }
        'n' => (Some('\n'), 2),
        't' => (Some('\t'), 2),
        'r' => (Some('\r'), 2),
        'a' => (Some('\u{7}'), 2),
        'b' => (Some('\u{8}'), 2),
        'e' | 'E' => (Some('\u{1b}'), 2),
        'f' => (Some('\u{c}'), 2),
        'v' => (Some('\u{b}'), 2),
        '\\' => (Some('\\'), 2),
        '\'' => (Some('\''), 2),
        '"' => (Some('"'), 2),
        other => (Some(other), 2),
    }
}
