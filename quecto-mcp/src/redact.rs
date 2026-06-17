pub fn redact(input: &str) -> String {
    let mut output = input
        .replace("Authorization", "<redacted-header>")
        .replace("authorization", "<redacted-header>");

    let mut search_from = 0;
    while let Some(relative_start) = output[search_from..].find("Bearer ") {
        let start = search_from + relative_start;
        let token_start = start + "Bearer ".len();
        let token_len = output[token_start..]
            .find(|c: char| c.is_whitespace() || c == '\"' || c == '\'' || c == '}' || c == ',')
            .unwrap_or_else(|| output[token_start..].len());
        output.replace_range(token_start..token_start + token_len, "<redacted>");
        search_from = token_start + "<redacted>".len();
        if search_from >= output.len() {
            break;
        }
    }

    output
}

pub(crate) fn redact_url(url: &str) -> String {
    url.split('@').next_back().unwrap_or(url).to_string()
}
