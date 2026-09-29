//! #2304 review round 5: the leaks and over-redactions the fifth review
//! found, each with the exact output redaction gives it. The whole set
//! redacts to itself a second time.

use super::*;

/// `(text, expected)`: every round-five string, and what it redacts to.
pub(super) const ROUND_FIVE: &[(&str, &str)] = &[
    // M2: a bare value stopping at a mid-word quote no longer leaks the
    // quoted secret after it; a wrapper before the quote is kept.
    (
        r#"Settings { password: Some("S3CRsome") }"#,
        r#"Settings { password: Some("[REDACTED]") }"#,
    ),
    (
        r#"Config { token: Some(Secret("S3CRsecret")) }"#,
        r#"Config { token: Some(Secret("[REDACTED]")) }"#,
    ),
    (
        r#"Err(Auth { api_key: Ok("S3CRok") })"#,
        r#"Err(Auth { api_key: Ok("[REDACTED]") })"#,
    ),
    (
        "export PASSWORD=$'S3CRansi'",
        "export PASSWORD=$'[REDACTED]'",
    ),
    (r#"api_key=b"S3CRbytes""#, r#"api_key=b"[REDACTED]""#),
    ("token=u'S3CRunicode'", "token=u'[REDACTED]'"),
    (r#"password=abc"S3CRtail""#, "password=[REDACTED]"),
    (r#"token = ["S3CRarr"]"#, r#"token = ["[REDACTED]"]"#),
    (
        r#"let c = Creds { password: Secret::new("S3CRnew") };"#,
        r#"let c = Creds { password: Secret::new("[REDACTED]") };"#,
    ),
    // A quote closing the string the label sits in is kept.
    (
        r#"sh -c "echo token=abc" && ls"#,
        r#"sh -c "echo token=[REDACTED]" && ls"#,
    ),
    (
        r#"x="a token=abc",y="z""#,
        r#"x="a token=[REDACTED]",y="z""#,
    ),
    // M3: a URL's userinfo that is a bare token, not a plain username.
    (
        "git clone https://S3CRbareToken123456@github.com/org/repo",
        "git clone https://[REDACTED]@github.com/org/repo",
    ),
    (
        r#"git config --global url."https://S3CRtoken123@github.com/".insteadOf "https://github.com/""#,
        r#"git config --global url."https://[REDACTED]@github.com/".insteadOf "https://github.com/""#,
    ),
    (
        "SENTRY_DSN=https://S3CRdsnkey@o1.ingest.sentry.io/1",
        "SENTRY_DSN=https://[REDACTED]@o1.ingest.sentry.io/1",
    ),
    (
        "git clone ssh://git@example.test/r.git",
        "git clone ssh://git@example.test/r.git",
    ),
    ("open https://user@host/x", "open https://user@host/x"),
    (
        "ftp://anonymous@ftp.example/pub",
        "ftp://anonymous@ftp.example/pub",
    ),
    // M3: a URL password holding `/`, `#`, `?` or `"`, to the last `@`
    // before the host.
    (
        "psql postgres://app:pa/ss#w?rd@db/shop",
        "psql postgres://app:[REDACTED]@db/shop",
    ),
    (r#"mysql://root:pa"ss@db/x"#, "mysql://root:[REDACTED]@db/x"),
    (
        "https://u:p@host/x?email=a@b",
        "https://u:[REDACTED]@host/x?email=a@b",
    ),
    // …while a port, and an `@` in a path, stay.
    (
        "http://localhost:8080/@user/x",
        "http://localhost:8080/@user/x",
    ),
    (
        "npm view https://registry.npmjs.org/@scope/pkg",
        "npm view https://registry.npmjs.org/@scope/pkg",
    ),
    (r#"["http://h:80","a@b"]"#, r#"["http://h:80","a@b"]"#),
    // Lows: labels.
    (
        "Server=db;Uid=app;Pwd=S3CRpwd;Database=shop",
        "Server=db;Uid=app;Pwd=[REDACTED];Database=shop",
    ),
    (
        "Server=db;User Id=app;Password=S3CRconn;",
        "Server=db;User Id=app;Password=[REDACTED];",
    ),
    (
        "DefaultEndpointsProtocol=https;AccountName=acct;AccountKey=S3CRacct+/key==;EndpointSuffix=core.windows.net",
        "DefaultEndpointsProtocol=https;AccountName=acct;AccountKey=[REDACTED];EndpointSuffix=core.windows.net",
    ),
    (
        r#"conn "Password=S3CRq;Server=db""#,
        r#"conn "Password=[REDACTED];Server=db""#,
    ),
    ("user passwd=S3CRpasswd", "user passwd=[REDACTED]"),
    (
        "key passphrase: S3CRphrase ok",
        "key passphrase: [REDACTED] ok",
    ),
    (
        r#"{"privateKey": "S3CRpriv", "accessKey": "S3CRacc", "secretKey": "S3CRsec"}"#,
        r#"{"privateKey": "[REDACTED]", "accessKey": "[REDACTED]", "secretKey": "[REDACTED]"}"#,
    ),
    ("private_key=S3CRpk", "private_key=[REDACTED]"),
    (
        "<password>S3CRxml</password>",
        "<password>[REDACTED]</password>",
    ),
    (
        "<db_password>S3CRxml2</db_password>",
        "<db_password>[REDACTED]</db_password>",
    ),
    ("<password></password>", "<password></password>"),
    ("DJANGO_SECRET_KEY=s3cr", "DJANGO_SECRET_KEY=[REDACTED]"),
    ("SECRET_KEY_BASE=s3cr", "SECRET_KEY_BASE=[REDACTED]"),
    // …while a working directory and a path stay.
    ("PWD=/home/u OLDPWD=/tmp", "PWD=/home/u OLDPWD=/tmp"),
    (
        "cat: /etc/passwd: Permission denied",
        "cat: /etc/passwd: Permission denied",
    ),
    // Lows: flags.
    (
        "twine upload -u __token__ -p S3CRtwine dist/*",
        "twine upload -u __token__ -p [REDACTED] dist/*",
    ),
    ("zip -P S3CRzip out.zip f", "zip -P [REDACTED] out.zip f"),
    ("7z a -pS3CR7z out.7z f", "7z a -p[REDACTED] out.7z f"),
    ("7z x -p out.7z", "7z x -p out.7z"),
    (
        "sqlcmd -S db -U sa -P S3CRsql",
        "sqlcmd -S db -U sa -P [REDACTED]",
    ),
    (
        "openssl pkcs12 -in a.p12 -passin pass:S3CRin -passout pass:S3CRout",
        "openssl pkcs12 -in a.p12 -passin [REDACTED] -passout [REDACTED]",
    ),
    (
        "gpg --batch --passphrase S3CRgpg -d f",
        "gpg --batch --passphrase [REDACTED] -d f",
    ),
    (
        "gpg --batch --passphrase-file p.txt -d f",
        "gpg --batch --passphrase-file p.txt -d f",
    ),
    (
        "ssh-keygen -t ed25519 -N S3CRkeygen -f k",
        "ssh-keygen -t ed25519 -N [REDACTED] -f k",
    ),
    ("ssh-keygen -N '' -f k", "ssh-keygen -N '' -f k"),
    (
        "az storage blob upload --account-key S3CRazkey -f x",
        "az storage blob upload --account-key [REDACTED] -f x",
    ),
    (
        "skopeo copy --src-creds u:S3CRsrc --dest-creds u:S3CRdst a b",
        "skopeo copy --src-creds [REDACTED] --dest-creds [REDACTED] a b",
    ),
    (
        "skopeo inspect --creds=u:S3CRcreds x",
        "skopeo inspect --creds=[REDACTED] x",
    ),
    (
        "doctl auth init -t S3CRdoctl",
        "doctl auth init -t [REDACTED]",
    ),
    // Lows: prefixed tokens.
    (
        "echo github_pat_11ABCDEFG0123456789_abcdefghijklmnopqrstuvwxyz",
        "echo [REDACTED]",
    ),
    (
        "echo hf_abcdefghijklmnopqrstuvwxyzABCDEF",
        "echo [REDACTED]",
    ),
    (
        "echo npm_abcdefghijklmnopqrstuvwxyz0123456789",
        "echo [REDACTED]",
    ),
    (
        "echo pypi-AgEIcHlwaS5vcmcCJGFiY2RlZjAxMjM0NTY3ODk",
        "echo [REDACTED]",
    ),
    (
        "echo AGE-SECRET-KEY-1QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ",
        "echo [REDACTED]",
    ),
    (
        "echo eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJl done",
        "echo [REDACTED] done",
    ),
    (
        "-----BEGIN RSA PRIVATE KEY-----\nMIIEowS3CR\nIBAAKS3CR\n-----END RSA PRIVATE KEY-----",
        "-----BEGIN RSA PRIVATE KEY-----\n[REDACTED]\n-----END RSA PRIVATE KEY-----",
    ),
    (
        r#"{"k":"-----BEGIN PRIVATE KEY-----\nMIIES3CR\n-----END PRIVATE KEY-----\n"}"#,
        r#"{"k":"-----BEGIN PRIVATE KEY-----[REDACTED]-----END PRIVATE KEY-----\n"}"#,
    ),
    (
        "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaS3CR",
        "-----BEGIN OPENSSH PRIVATE KEY-----\n[REDACTED]",
    ),
    (
        "https://x/cb?authorization=Bearer%20S3CRpct&x=1",
        "https://x/cb?authorization=[REDACTED]&x=1",
    ),
    // Lows: positional logins.
    (
        "cargo login cioS3CR0123456789abcdef",
        "cargo login [REDACTED]",
    ),
    (
        "cargo login --registry r cioS3CR0123456789abcdeg",
        "cargo login --registry r [REDACTED]",
    ),
    (
        "vault login hvs.S3CR0123456789abcdef",
        "vault login [REDACTED]",
    ),
    ("run cargo login to publish", "run cargo login to publish"),
    (
        "vault login -method=userpass username=me",
        "vault login -method=userpass username=me",
    ),
    (
        "SSHPASS=S3CRsshpass sshpass -e ssh h",
        "SSHPASS=[REDACTED] sshpass -e ssh h",
    ),
    (
        "set -x GITHUB_TOKEN S3CRfish",
        "set -x GITHUB_TOKEN [REDACTED]",
    ),
    ("set -gx API_KEY S3CRfish2", "set -gx API_KEY [REDACTED]"),
    ("set -x PATH /usr/bin", "set -x PATH /usr/bin"),
    // Over-redaction: code, form fields, flags kept.
    (
        "let token = lexer.next_token()?;",
        "let token = lexer.next_token()?;",
    ),
    (
        "    api_key: Option<Secret>,",
        "    api_key: Option<Secret>,",
    ),
    (
        "pub struct C { pub api_key: Option<Secret>, n: u8 }",
        "pub struct C { pub api_key: Option<Secret>, n: u8 }",
    ),
    ("    password: Secret,", "    password: Secret,"),
    (
        r#"curl -d "client_secret=S3CR&grant_type=client_credentials" x"#,
        r#"curl -d "client_secret=[REDACTED]&grant_type=client_credentials" x"#,
    ),
    (
        "curl --oauth2-bearer S3CRtok x",
        "curl --oauth2-bearer [REDACTED] x",
    ),
    ("mysql -p=S3CR shop", "mysql -p=[REDACTED] shop"),
];

#[test]
fn every_round_five_string_redacts_to_its_expected_output() {
    let failures: Vec<String> = ROUND_FIVE
        .iter()
        .filter_map(|(text, expected)| {
            let redacted = redact_secrets(text);
            (redacted != *expected)
                .then(|| format!("{text:?}\n   gave {redacted:?}\n   want {expected:?}"))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} of {} wrong:\n{}",
        failures.len(),
        ROUND_FIVE.len(),
        failures.join("\n")
    );
}

#[test]
fn redaction_is_idempotent_over_the_round_five_strings() {
    for (text, _) in ROUND_FIVE {
        let once = redact_secrets(text);
        assert_eq!(redact_secrets(&once), once, "{text:?}");
    }
}

/// No round-five secret (each spelt with `S3CR`) survives redaction.
#[test]
fn no_round_five_secret_survives() {
    for (text, _) in ROUND_FIVE {
        let redacted = redact_secrets(text);
        assert!(!redacted.contains("S3CR"), "{text:?} gave {redacted:?}");
    }
}

/// The round-five gaps and false positives kept on purpose, each pinned
/// so a change to it is a decision.
#[test]
fn the_round_five_trade_offs_are_pinned() {
    for (text, expected) in [
        // `Bearer` takes the word after it, prose or not.
        ("Bearer authentication failed", "[REDACTED] failed"),
        // A type-like CamelCase value before `,` reads as code.
        ("{ password: Hunter, x }", "{ password: Hunter, x }"),
        // A call after a spaced label reads as code.
        ("password = compute(x)", "password = compute(x)"),
        // A field access that is not a call is a value.
        ("let t = token = self.token;", "let t = token = [REDACTED];"),
        // A token in prose, with no label, flag or prefix, is not found.
        (
            "the key is Zq8vB2mNx4LpW7rT9yK3",
            "the key is Zq8vB2mNx4LpW7rT9yK3",
        ),
        // A lower-case userinfo with no digits reads as a username.
        ("https://deploybot@host/x", "https://deploybot@host/x"),
    ] {
        assert_eq!(redact_secrets(text), expected, "{text:?}");
    }
}
