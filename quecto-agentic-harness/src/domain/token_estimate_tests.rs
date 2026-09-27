use super::*;

/// `seq a b | tr "\n" " "`: the #2212 QA content.
fn seq(from: u32, to: u32) -> String {
    (from..=to).map(|n| format!("{n} ")).collect()
}

#[test]
fn prose_is_four_characters_per_token_as_before() {
    assert_eq!(estimate_tokens(&"x".repeat(400)), 100);
    assert_eq!(estimate_tokens("ab"), 1);
    let prose = "The quick brown fox jumps over the lazy dog. ".repeat(40);
    assert_eq!(estimate_tokens(&prose), prose.len().div_ceil(4));
}

#[test]
fn non_ascii_is_one_token_per_character_as_before() {
    assert_eq!(estimate_tokens(&"中".repeat(100)), 100);
    assert_eq!(estimate_tokens("日本 abcd"), 2 + 2);
}

#[test]
fn empty_text_is_zero_tokens() {
    assert_eq!(estimate_tokens(""), 0);
}

/// #2212 evidence: `seq` output grew the provider's count at about 2.0x
/// the ASCII/4 figure, so the estimate counts it at 2 characters a token.
#[test]
fn seq_output_counts_at_two_characters_per_token() {
    let digits = seq(90_000, 100_000);
    let ascii_quarter = digits.len().div_ceil(4);
    let estimate = estimate_tokens(&digits);
    assert_eq!(estimate, digits.len().div_ceil(2));
    assert!((1_990..=2_000).contains(&(estimate * 1_000 / ascii_quarter)));
    // Newline-separated, as `seq` prints it without `tr`.
    let lines: String = (1..=500).map(|n| format!("{n}\n")).collect();
    assert_eq!(estimate_tokens(&lines), lines.len().div_ceil(2));
}

#[test]
fn prose_with_a_few_numbers_stays_close_to_the_prose_rate() {
    let text = "In 2024 the team shipped 3 releases and fixed many bugs. ".repeat(20);
    let ascii_quarter = text.len().div_ceil(4);
    let estimate = estimate_tokens(&text);
    assert!(estimate > ascii_quarter);
    assert!(
        estimate * 100 <= ascii_quarter * 115,
        "mostly prose: {estimate} vs {ascii_quarter}"
    );
}

#[test]
fn words_carrying_digits_are_dense_and_letter_words_are_prose() {
    // Hex digests, UUIDs, versions: a word with a digit is dense.
    assert_eq!(estimate_tokens("9f86d081884c7d65"), 8);
    assert_eq!(estimate_tokens("550e8400-e29b-41d4"), 9);
    // A pure-letter word is prose even when it spells hex.
    assert_eq!(estimate_tokens("deadbeef"), 2);
    // The separator after a dense word is dense; after a prose word, prose.
    assert_eq!(estimate_tokens("12 "), 2);
    assert_eq!(estimate_tokens("ab "), 1);
}

/// #2212 review 2: only the first separator after a dense word is dense;
/// padding and indentation after it (`cat -n`, aligned columns) are prose.
#[test]
fn only_the_first_separator_after_a_dense_word_is_dense() {
    // "12" + "\t" dense (2 tokens); "    " + "fn" prose (6 chars, 2 tokens).
    assert_eq!(estimate_tokens("12\t    fn"), 2 + 2);
    // `cat -n` lines: the number and its tab are dense, the code prose.
    let cat_n: String = (1..=200)
        .map(|n| format!("{n:6}\tlet value = compute(input);\n"))
        .collect();
    let flat = estimate_opaque_tokens(&cat_n);
    assert!(
        estimate_tokens(&cat_n) * 100 <= flat * 115,
        "{} vs {flat}",
        estimate_tokens(&cat_n)
    );
}

#[test]
fn a_dense_run_ends_at_the_next_prose_word() {
    // "1," "2," "3," are 6 dense chars (3 tokens); the spaces and "done"
    // are 7 prose chars (2 tokens).
    assert_eq!(estimate_tokens("1, 2, 3, done"), 3 + 2);
    // A non-ASCII character ends the dense run too: the spaces after it are
    // prose ("12" dense 1, "€" 1, "   abcd" prose 2).
    assert_eq!(estimate_tokens("12€   abcd"), 1 + 1 + 2);
}

#[test]
fn opaque_payloads_keep_the_plain_ascii_rate() {
    // Base64 image data is priced per image by providers, not per character;
    // the dense rate would double an already large over-estimate.
    let base64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk".repeat(10);
    assert_eq!(estimate_opaque_tokens(&base64), base64.len().div_ceil(4));
    assert_eq!(estimate_opaque_tokens("中中ab"), 3);
}

#[test]
fn the_estimate_never_falls_below_the_plain_ascii_rate() {
    for text in [
        seq(1, 3_000),
        "a1 b2 c3 d4".repeat(50),
        "x".repeat(33),
        "12€x9 ,, 7".to_string(),
    ] {
        assert!(
            estimate_tokens(&text) >= estimate_opaque_tokens(&text),
            "{text}"
        );
    }
}

/// Pseudo-random bytes (a fixed LCG): the tests stay deterministic.
fn random_bytes(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed;
    (0..len)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as u8
        })
        .collect()
}

fn high_entropy(chars: usize) -> usize {
    (chars * HIGH_ENTROPY_TOKENS_PER_SEVEN_CHARS).div_ceil(7)
}

/// #2212 review 2: base64 tokenises at about 1.4 characters a token
/// (cl100k and o200k: an 8 KB blob measured 0.69 of its tokens at the
/// dense and prose rates).
#[test]
fn an_8_kb_base64_blob_counts_at_the_high_entropy_rate() {
    use base64::Engine;
    let blob = base64::engine::general_purpose::STANDARD.encode(random_bytes(6_000, 7));
    assert_eq!(blob.len(), 8_000);
    assert!(blob.contains('+') && blob.contains('/'));
    assert_eq!(estimate_tokens(&blob), high_entropy(blob.len()));
    // Wrapped at 76 columns, each line is its own run.
    let wrapped: String = blob
        .as_bytes()
        .chunks(76)
        .map(|line| format!("{}\n", std::str::from_utf8(line).unwrap()))
        .collect();
    for line in blob.as_bytes().chunks_exact(76) {
        assert_eq!(
            estimate_tokens(std::str::from_utf8(line).unwrap()),
            high_entropy(76)
        );
    }
    // The newlines are prose; a short last line may lack a digit.
    let lines = blob.len().div_ceil(76);
    let wrapped_estimate = estimate_tokens(&wrapped);
    let expected = high_entropy(blob.len()) + lines.div_ceil(4);
    assert!(
        wrapped_estimate.abs_diff(expected) <= 2,
        "{wrapped_estimate} vs {expected}"
    );
}

#[test]
fn a_jwt_counts_each_segment_at_the_high_entropy_rate() {
    use base64::Engine;
    let url = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let segments = [
        url.encode(br#"{"alg":"HS256","typ":"JWT"}"#),
        url.encode(random_bytes(180, 11)),
        url.encode(random_bytes(32, 13)),
    ];
    let jwt = segments.join(".");
    let segment_chars: usize = segments.iter().map(String::len).sum();
    // The two dots are prose (one token).
    assert_eq!(estimate_tokens(&jwt), high_entropy(segment_chars) + 1);
}

#[test]
fn long_runs_need_upper_lower_and_digits_to_be_high_entropy() {
    // A path joined by slashes and hyphens: no digit, prose.
    let path = "quecto-agentic-harness/README";
    assert_eq!(estimate_tokens(path), path.len().div_ceil(4));
    // A snake_case identifier with a digit but no upper case: dense words.
    // "agent_loop_turn_flow_" is 21 prose chars (6), "2" dense (1).
    assert_eq!(estimate_tokens("agent_loop_turn_flow_2"), 6 + 1);
    // Mixed case with digits under 16 characters: the dense rate.
    assert_eq!(estimate_tokens("Ab1Cd2Ef3"), 5);
    // Sixteen characters: high entropy.
    assert_eq!(estimate_tokens("Ab1Cd2Ef3Gh4Ij5K"), high_entropy(16));
}

/// #2212 PR review: paths and identifiers carry mixed case and digits and
/// are joined by `/` and `-`, but switch between upper case, lower case and
/// digits rarely; base64 switches on most characters. Measured against
/// cl100k: `src/Foo2Bar/README-v2/documentation.md` is 11 tokens.
#[test]
fn paths_and_identifiers_are_not_high_entropy() {
    for text in [
        "src/Foo2Bar/README-v2/documentation.md",
        "x86_64-unknown-linux-gnu/release/Build3Script",
        "README-v2/documentation",
        "SessionIdentityV2Parser",
        "quecto-agentic-harness/src/Application2/ContextManager",
    ] {
        let estimate = estimate_tokens(text);
        assert!(
            estimate < high_entropy(text.len()),
            "{text}: {estimate} is the high-entropy rate"
        );
        assert!(estimate <= text.len().div_ceil(2), "{text}: {estimate}");
    }
    // The path's words keep their own classes: "src/" prose, "Foo2Bar/"
    // dense, "README-" prose, "v2/" dense, "documentation.md" prose.
    assert_eq!(
        estimate_tokens("src/Foo2Bar/README-v2/documentation.md"),
        (4usize + 7 + 16).div_ceil(4) + (8usize + 3).div_ceil(2)
    );
}

#[test]
fn keys_and_tokens_that_switch_class_often_are_high_entropy() {
    for key in [
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9",
        "ghp_16C7e42F292c6912E7710c838347Ae178B4a",
        "Ab1Cd2Ef3Gh4Ij5K",
    ] {
        assert_eq!(estimate_tokens(key), high_entropy(key.len()), "{key}");
    }
}
