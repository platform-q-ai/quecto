//! #2304 review round 4: a corpus of realistic strings — shell commands,
//! config lines, headers, JSON bodies, error bodies — each with the exact
//! output redaction gives it. The whole corpus redacts to itself a second
//! time, and redaction stays linear however adversarial the text.

use super::*;

/// `(text, expected)`: every realistic string, and what it redacts to.
pub(super) const CORPUS: &[(&str, &str)] = &[
    // M1: a password in a connection URL; the user and the host are kept.
    (
        "psql postgresql://app:hunter2@db:5432/shop",
        "psql postgresql://app:[REDACTED]@db:5432/shop",
    ),
    (
        "export DATABASE_URL=postgres://app:hunter2@db/shop",
        "export DATABASE_URL=postgres://app:[REDACTED]@db/shop",
    ),
    (
        "redis-cli -u redis://:hunter2@cache:6379",
        "redis-cli -u redis://:[REDACTED]@cache:6379",
    ),
    (
        r#"mongosh "mongodb+srv://app:hunter2@cluster0/db""#,
        r#"mongosh "mongodb+srv://app:[REDACTED]@cluster0/db""#,
    ),
    (
        "psql postgres://app:p%40ss%3Aw0rd@db/shop",
        "psql postgres://app:[REDACTED]@db/shop",
    ),
    ("https://u:p@ss@host/x", "https://u:[REDACTED]@host/x"),
    (
        "amqp://guest:guest@rabbit:5672/",
        "amqp://guest:[REDACTED]@rabbit:5672/",
    ),
    (
        r#"DATABASE_URL="postgres://app:hunter2@db/shop" ./run"#,
        r#"DATABASE_URL="postgres://app:[REDACTED]@db/shop" ./run"#,
    ),
    (
        "pip install -i https://user:pw123@pypi.example/simple pkg",
        "pip install -i https://user:[REDACTED]@pypi.example/simple pkg",
    ),
    (
        "git clone ssh://git@example.test:2222/r.git",
        "git clone ssh://git@example.test:2222/r.git",
    ),
    ("mysql://root:@db/shop", "mysql://root:@db/shop"),
    // M2: a secret after a spaced flag.
    (
        "cargo publish --token cio_0123456789abcdef",
        "cargo publish --token [REDACTED]",
    ),
    (
        "tool --api-key abcdef0123456789 run",
        "tool --api-key [REDACTED] run",
    ),
    ("cli --auth-token X", "cli --auth-token [REDACTED]"),
    ("vault write --secret X", "vault write --secret [REDACTED]"),
    (
        "az login --client-secret X --tenant t",
        "az login --client-secret [REDACTED] --tenant t",
    ),
    ("tool --APIKEY abcd", "tool --APIKEY [REDACTED]"),
    ("tool --access-key abc123", "tool --access-key [REDACTED]"),
    (
        "tool --token 'two words' -v",
        "tool --token '[REDACTED]' -v",
    ),
    (
        r#"curl -H "Authorization: token abc123def" https://api.example"#,
        r#"curl -H "Authorization: token [REDACTED]" https://api.example"#,
    ),
    ("tool --token --verbose", "tool --token --verbose"),
    ("tool --token-file ~/.tok", "tool --token-file ~/.tok"),
    ("git log --grep token", "git log --grep token"),
    // L1: a scheme word after a label takes its credential with it.
    (
        "headers token: Bearer eyJhbGciOiJIUzI1NiJ9.e30.x end",
        "headers token: [REDACTED] end",
    ),
    (
        "AUTH_TOKEN=Bearer abc123 make",
        "AUTH_TOKEN=[REDACTED] make",
    ),
    ("x token=Basic YWxpY2U6czNjcmV0 y", "x token=[REDACTED] y"),
    (
        "cfg api_key: token ghx0123 ok",
        "cfg api_key: [REDACTED] ok",
    ),
    // L2: a label starting a line, after `:`, takes the rest of the line.
    (
        "password: correct horse battery staple",
        "password: [REDACTED]",
    ),
    (
        "  db_password: two words # prod",
        "  db_password: [REDACTED]",
    ),
    ("- secret: a b c", "- secret: [REDACTED]"),
    (
        "  API_TOKEN: ${{ secrets.API_TOKEN }}",
        "  API_TOKEN: [REDACTED]",
    ),
    (
        "note the password: hunter2 is old",
        "note the password: [REDACTED] is old",
    ),
    (
        "PGPASSWORD=hunter2 psql db",
        "PGPASSWORD=[REDACTED] psql db",
    ),
    // L3: cookie headers and `gh secret set`'s body.
    (
        "curl -H 'Cookie: sid=abc; csrftoken=xyz' https://x",
        "curl -H 'Cookie: [REDACTED]' https://x",
    ),
    (
        "Set-Cookie: sid=abc123; Path=/; HttpOnly",
        "Set-Cookie: [REDACTED]",
    ),
    ("grep -rn 'Cookie:' src/", "grep -rn 'Cookie:' src/"),
    (
        r#"gh secret set API_TOKEN --body "s3cr3t value""#,
        r#"gh secret set API_TOKEN --body "[REDACTED]""#,
    ),
    (
        "gh secret set API_TOKEN -b s3cr3t",
        "gh secret set API_TOKEN -b [REDACTED]",
    ),
    (
        "gh secret set API_TOKEN -bs3cr3t --repo o/r",
        "gh secret set API_TOKEN -b[REDACTED] --repo o/r",
    ),
    (
        "gh secret set X --body=abc",
        "gh secret set X --body=[REDACTED]",
    ),
    ("gh secret list", "gh secret list"),
    // L4: no rule reaches across a newline.
    (
        "password:\n  file: /run/secrets/db",
        "password:\n  file: /run/secrets/db",
    ),
    ("token=\nnext", "token=\nnext"),
    ("password: \"abc\ndef\"", "password: \"[REDACTED]\ndef\""),
    // Nits: `login -p` only for a registry-style login.
    (
        "gh auth login -p https -h github.com",
        "gh auth login -p https -h github.com",
    ),
    (
        r#"git commit -m "fix login -p flag""#,
        r#"git commit -m "fix login -p flag""#,
    ),
    (
        "az acr login -n reg -u me -p hunter2",
        "az acr login -n reg -u me -p [REDACTED]",
    ),
    (
        "az acr login -n reg -p hunter2 -u me",
        "az acr login -n reg -p [REDACTED] -u me",
    ),
    (
        "sudo podman login -p hunter2 quay.io",
        "sudo podman login -p [REDACTED] quay.io",
    ),
    // Nits: a bare value keeps the structure after it.
    (
        "https://x/cb?access_token=abc123&state=x",
        "https://x/cb?access_token=[REDACTED]&state=x",
    ),
    (
        "helm install --set db.password=x,db.user=app",
        "helm install --set db.password=[REDACTED],db.user=app",
    ),
    (
        r#"{"api_key": null, "n": 1}"#,
        r#"{"api_key": [REDACTED], "n": 1}"#,
    ),
    (
        r#"{"error":"bad {\"token\":\"tok_abc123\"}"}"#,
        r#"{"error":"bad {\"token\":\"[REDACTED]\"}"}"#,
    ),
    (
        "git push https://x-access-token:ghs_abcdefghijklmnopqrstuvwxyz0123@github.com/o/r",
        "git push https://x-access-token:[REDACTED]@github.com/o/r",
    ),
    ("$(echo token=abc)", "$(echo token=[REDACTED])"),
    // Nits: JSON escapes and a value starting with a space.
    (
        r#"{"password":"hun\"ter2","user":"bob"}"#,
        r#"{"password":"[REDACTED]","user":"bob"}"#,
    ),
    (
        r#"{"password": " hunter2"}"#,
        r#"{"password": "[REDACTED]"}"#,
    ),
    (r#"{"password": ""}"#, r#"{"password": ""}"#),
    (r#"{"password": " "}"#, r#"{"password": " "}"#),
    (r#"rg '"token":' fixtures/"#, r#"rg '"token":' fixtures/"#),
    // A bare command-line value ends where the shell's word does, and a
    // value inside its label's quote skips an escaped quote.
    (
        "x; docker login -p pw; ssh -p 22 h",
        "x; docker login -p [REDACTED]; ssh -p 22 h",
    ),
    ("echo $(mysql -phunter2)", "echo $(mysql -p[REDACTED])"),
    (
        r#"curl -d "token=abc\"def" x"#,
        r#"curl -d "token=[REDACTED]" x"#,
    ),
    // Shapes redacted before, still redacted.
    (
        "export GITHUB_TOKEN=ghp_0123456789abcdef0123456789abcdef0123",
        "export GITHUB_TOKEN=[REDACTED]",
    ),
    (
        "curl -u alice:s3cret https://api.example",
        "curl -u [REDACTED] https://api.example",
    ),
    (
        "echo sk-proj-Ab3dEf6hIj9kLm2nOp5qRs8tUv1wXy4z",
        "echo [REDACTED]",
    ),
    ("Authorization: Bearer abc.def", "Authorization: [REDACTED]"),
    (
        "sshpass -p hunter2 ssh host",
        "sshpass -p [REDACTED] ssh host",
    ),
    (
        "mysql -uroot -phunter2 shop",
        "mysql -uroot -p[REDACTED] shop",
    ),
    (
        "kubectl create secret generic db --from-literal=password=hunter2",
        "kubectl create secret generic db --from-literal=password=[REDACTED]",
    ),
    (
        "vault kv put secret/db password=hunter2",
        "vault kv put secret/db password=[REDACTED]",
    ),
    // Text that only looks alike survives.
    ("ls -la /tmp", "ls -la /tmp"),
    ("ssh -p 2222 host", "ssh -p 2222 host"),
    ("mkdir -p /w/a", "mkdir -p /w/a"),
    ("the token expired", "the token expired"),
    (r#"echo "$API_TOKEN""#, r#"echo "$API_TOKEN""#),
];

#[test]
fn the_corpus_is_large_enough() {
    assert!(CORPUS.len() >= 60, "{} strings", CORPUS.len());
}

#[test]
fn every_corpus_string_redacts_to_its_expected_output() {
    let failures: Vec<String> = CORPUS
        .iter()
        .filter_map(|(text, expected)| {
            let redacted = redact_secrets(text);
            (redacted != *expected)
                .then(|| format!("{text:?}\n   gave {redacted:?}\n   want {expected:?}"))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Idempotence over the whole corpus: `redact(redact(x)) == redact(x)`.
#[test]
fn redaction_is_idempotent_over_the_corpus() {
    for (text, _) in CORPUS {
        let once = redact_secrets(text);
        assert_eq!(redact_secrets(&once), once, "{text:?}");
    }
}

/// Every rule is a `regex` pattern or a scanner that moves forward: a
/// long adversarial text of labels, flags, quotes and URLs redacts in
/// time linear in its length (a quadratic pass over 256 KiB would take
/// minutes, not the seconds allowed here even on a loaded debug build).
#[test]
fn redaction_stays_linear_over_adversarial_text() {
    for unit in [
        r#""password": " "#,
        r#"password:" "#,
        "'token=",
        "--token ",
        "curl -u a ",
        "login -u -p ",
        "x://a:b",
        "Cookie: \"",
        r#"{\"token\":\""#,
        "password: Bearer ",
    ] {
        let text = unit.repeat(256 * 1024 / unit.len());
        let started = std::time::Instant::now();
        let redacted = redact_secrets(&text);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(20),
            "{unit:?} took {:?}",
            started.elapsed()
        );
        assert_eq!(redact_secrets(&redacted), redacted, "{unit:?}");
    }
}
