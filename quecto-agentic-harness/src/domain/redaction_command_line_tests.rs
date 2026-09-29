//! #2304 review: the credentials a command line carries — a Bash tool
//! call's summary in a claude member's event log, a quecto `bash` audit
//! record — are redacted in their command-line shapes, keeping the flag or
//! the name they follow; the flags and names that only look alike survive.

use super::*;

#[test]
fn each_command_line_credential_shape_is_redacted_keeping_its_label() {
    for (text, expected) in [
        (
            "curl -u alice:s3cret https://api.example",
            "curl -u [REDACTED] https://api.example",
        ),
        (
            "curl --user alice:s3cret https://api.example",
            "curl --user [REDACTED] https://api.example",
        ),
        (
            "curl --user=alice:s3cret https://api.example",
            "curl --user=[REDACTED] https://api.example",
        ),
        (
            "curl -ualice:s3cret https://api.example",
            "curl -u[REDACTED] https://api.example",
        ),
        (
            "curl -u 'alice:s3cret' https://api.example",
            "curl -u '[REDACTED]' https://api.example",
        ),
        (
            "curl -H 'Authorization: Basic YWxpY2U6czNjcmV0' https://api.example",
            "curl -H 'Authorization: Basic [REDACTED]' https://api.example",
        ),
        (
            "curl -H \"proxy-authorization: basic YWxpY2U6czNjcmV0\"",
            "curl -H \"proxy-authorization: basic [REDACTED]\"",
        ),
        (
            "psql --password hunter2 db",
            "psql --password [REDACTED] db",
        ),
        ("tool --passwd hunter2", "tool --passwd [REDACTED]"),
        (
            "openssl enc -pass pass:hunter2 -in a",
            "openssl enc -pass [REDACTED] -in a",
        ),
        (
            "mysql -h db -u root -phunter2 shop",
            "mysql -h db -u root -p[REDACTED] shop",
        ),
        ("mysqldump -proot shop", "mysqldump -p[REDACTED] shop"),
        (
            "cd /w && mariadb -uroot -pS3cr3t! -e 'select 1'",
            "cd /w && mariadb -uroot -p[REDACTED] -e 'select 1'",
        ),
        (
            "sshpass -p hunter2 ssh host",
            "sshpass -p [REDACTED] ssh host",
        ),
        (
            "export STRIPE_KEY=rk_live_0123456789",
            "export STRIPE_KEY=[REDACTED]",
        ),
        (
            "AWS_ACCESS_KEY_ID=ASIA0123456789 aws s3 ls",
            "AWS_ACCESS_KEY_ID=[REDACTED] aws s3 ls",
        ),
        ("export DB_PASS=hunter2", "export DB_PASS=[REDACTED]"),
        (
            "git clone https://oauth2:glpat-0123456789abcdefghij@gitlab.example/x",
            "git clone https://oauth2:[REDACTED]@gitlab.example/x",
        ),
        (
            "aws configure set aws_secret_access_key wJalrXUtnFEMI/K7MDENG",
            "aws configure set aws_secret_access_key [REDACTED]",
        ),
        (
            "aws_secret_access_key = wJalrXUtnFEMI/K7MDENG",
            "aws_secret_access_key = [REDACTED]",
        ),
    ] {
        assert_eq!(redact_secrets(text), expected, "{text:?}");
    }
}

/// The named shapes redacted before, still redacted whatever the pass
/// order: the secret never survives.
#[test]
fn a_named_secret_assignment_is_redacted() {
    for (text, secret) in [
        ("export GITHUB_TOKEN=ghx0123456789", "ghx0123456789"),
        ("export APP_SECRET=abcdefabcdef", "abcdefabcdef"),
        ("export DB_PASSWORD=hunter2", "hunter2"),
        ("mysql --password=hunter2 shop", "hunter2"),
        (
            "AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG",
            "wJalrXUtnFEMI",
        ),
    ] {
        let redacted = redact_secrets(text);
        assert!(!redacted.contains(secret), "{text:?} gave {redacted:?}");
        assert!(redacted.contains("[REDACTED]"), "{redacted:?}");
    }
}

/// The flags and names the shapes above could be mistaken for: a `-u`
/// without a password (or a `uid:gid`), a `-p` that is a port, a flag or
/// a directory mode, an ordinary environment assignment, the word
/// "basic".
#[test]
fn look_alike_flags_and_names_survive() {
    for text in [
        "sort -u names.txt",
        "ps -u root",
        "docker run -u 1000:1000 image",
        "mysql -h db -P 3306 -u root shop",
        "mysql -p shop",
        "psql -p 5432 shop",
        "ssh -p 2222 host",
        "nc -l -p 8080",
        "mkdir -p /w/a/b",
        "cp -p a b",
        "git log -p -- src",
        "export PATH=/usr/bin:/bin",
        "export EDITOR=vim",
        "primary_key=id",
        "tool --passes=3",
        "a basic usage of curl",
        "Authorization: Basic",
        "glpat-short",
    ] {
        assert_eq!(redact_secrets(text), text, "{text:?}");
    }
}

/// A redacted text redacts to itself: a summary cut after redaction, or
/// redacted twice on its way to the log, stays the same.
#[test]
fn redaction_is_idempotent_over_the_command_line_shapes() {
    for text in [
        "curl -u alice:s3cret -H 'Authorization: Basic YWxpY2U6czNjcmV0' x",
        "mysql -phunter2 && export STRIPE_KEY=rk_live_0123 && sshpass -p pw ssh h",
        "aws_secret_access_key wJalrXUtnFEMI --password hunter2",
    ] {
        let once = redact_secrets(text);
        assert_eq!(redact_secrets(&once), once, "{text:?}");
    }
}
