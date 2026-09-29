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

/// #2304 review round 2 (M2): a quoted credential is redacted whole, to
/// its closing quote, spaces included (or to the text's end when the
/// quote never closes), keeping its quotes.
#[test]
fn a_quoted_credential_is_redacted_to_its_closing_quote() {
    for (text, expected) in [
        (
            "psql --password \"hunter2\" db",
            "psql --password \"[REDACTED]\" db",
        ),
        (
            "psql --password 'x y' db",
            "psql --password '[REDACTED]' db",
        ),
        (
            "tool --passwd=\"a b\" -v",
            "tool --passwd=\"[REDACTED]\" -v",
        ),
        (
            "mysql -u root -p\"x y\" shop",
            "mysql -u root -p\"[REDACTED]\" shop",
        ),
        (
            "sshpass -p 'hunter 2' ssh host",
            "sshpass -p '[REDACTED]' ssh host",
        ),
        (
            "export STRIPE_KEY=\"sk_live_0123456789\"",
            "export STRIPE_KEY=\"[REDACTED]\"",
        ),
        ("export DB_PASS='hunter 2'", "export DB_PASS='[REDACTED]'"),
        (
            "curl -H 'Authorization: Basic \"YWxp Y2U6\"' x",
            "curl -H 'Authorization: Basic \"[REDACTED]\"' x",
        ),
        (
            "curl -u \"alice:pa ss\" https://api.example",
            "curl -u \"[REDACTED]\" https://api.example",
        ),
        (
            "curl --user='alice:pa ss' https://api.example",
            "curl --user='[REDACTED]' https://api.example",
        ),
        (
            "aws configure set aws_secret_access_key \"wJalrXUtnFEMI/K7MDENG\"",
            "aws configure set aws_secret_access_key \"[REDACTED]\"",
        ),
        ("psql --password \"hunter 2", "psql --password \"[REDACTED]"),
    ] {
        assert_eq!(redact_secrets(text), expected, "{text:?}");
    }
}

/// #2304 review round 2 (M2): a named assignment's quoted value leaves no
/// tail behind either.
#[test]
fn a_quoted_named_assignment_leaves_no_tail() {
    for (text, tail) in [
        ("mysql --password=\"hunter two\" shop", "two"),
        ("password: 'correct horse battery'", "battery"),
        ("export GITHUB_TOKEN=\"ghx 0123456789\"", "0123456789"),
    ] {
        let redacted = redact_secrets(text);
        assert!(!redacted.contains(tail), "{text:?} gave {redacted:?}");
        assert!(redacted.contains("[REDACTED]"), "{redacted:?}");
    }
}

/// #2304 review round 2 (L3): `-u`/`--user` (and httpie's `-a`/`--auth`)
/// is a credential only after an HTTP client, `curl`, `wget`, `http`,
/// `https`, `httpie` or `xh`, in the same command: for every other tool
/// `-u user:group` names a user.
#[test]
fn a_user_flag_is_a_credential_only_after_an_http_client() {
    for (text, expected) in [
        (
            "curl -s -X POST -u alice:s3cret https://x",
            "curl -s -X POST -u [REDACTED] https://x",
        ),
        ("wget --user alice:s3cret x", "wget --user [REDACTED] x"),
        ("http -a alice:s3cret GET x", "http -a [REDACTED] GET x"),
        (
            "/usr/bin/curl -u 1000:1000 x",
            "/usr/bin/curl -u [REDACTED] x",
        ),
    ] {
        assert_eq!(redact_secrets(text), expected, "{text:?}");
    }
    for text in [
        "docker run -u app:app image",
        "docker exec -u postgres:postgres db psql",
        "curl -s x | docker run -u app:app image",
        "chown -R app:app /srv && sudo -u app:app ls",
    ] {
        assert_eq!(redact_secrets(text), text, "{text:?}");
    }
}

/// #2304 review round 2 (L3): an upper-case `*_KEY`, `*_KEY_ID` or `*_AUTH`
/// name is a credential's only when its value looks like one: at least
/// 16 characters, or a key's prefix (`sk_`, `pk_`, `rk_`, `sk-`, `ghp_`,
/// `github_pat_`, `glpat-`, `xox`, `AKIA`, `ASIA`, `AIza` …).
#[test]
fn a_key_name_is_redacted_only_with_a_secret_looking_value() {
    for (text, expected) in [
        (
            "export SIGNING_KEY=0123456789abcdef",
            "export SIGNING_KEY=[REDACTED]",
        ),
        (
            "export STRIPE_KEY=pk_live_1",
            "export STRIPE_KEY=[REDACTED]",
        ),
        ("DEPLOY_AUTH=ghp_abc make", "DEPLOY_AUTH=[REDACTED] make"),
    ] {
        assert_eq!(redact_secrets(text), expected, "{text:?}");
    }
    for text in [
        "export SORT_KEY=name PRIMARY_KEY=id",
        "USE_AUTH=true make",
        "export PARTITION_KEY_ID=42",
        "test FOO_KEY == x",
        "test FOO_KEY == averyveryverylongvalue123",
        "FOO_PASS == hunter2",
    ] {
        assert_eq!(redact_secrets(text), text, "{text:?}");
    }
}

/// #2304 review round 2 (L3): `aws_secret_access_key` spaced from its
/// value is a credential only when the value looks like an AWS secret (16
/// or more of `A-Z a-z 0-9 / + =`): prose naming it survives.
#[test]
fn prose_naming_the_aws_secret_survives() {
    for text in [
        "echo \"aws_secret_access_key is not set\"",
        "the aws_secret_access_key field",
    ] {
        assert_eq!(redact_secrets(text), text, "{text:?}");
    }
}

/// The false positives kept on purpose: each shape redacts what follows
/// its label whatever it is, where a miss would leak a credential.
#[test]
fn the_deliberate_false_positives_are_pinned() {
    for (text, expected) in [
        // A long `*_KEY` value that is no secret.
        (
            "export CACHE_KEY=user-profile-cache-v2",
            "export CACHE_KEY=[REDACTED]",
        ),
        (
            "SSH_KEY=/home/u/.ssh/id_ed25519 ssh h",
            "SSH_KEY=[REDACTED] ssh h",
        ),
        // `*_PASS`, `*_PWD` … take any value.
        ("export SKIP_PASS=1", "export SKIP_PASS=[REDACTED]"),
        // A password flag followed by another flag.
        ("tool --password --verbose", "tool --password [REDACTED]"),
        // `Authorization: Basic` followed by any word.
        (
            "Authorization: Basic realm",
            "Authorization: Basic [REDACTED]",
        ),
        // `aws_secret_access_key` after `=` or `:` takes any value.
        (
            "aws_secret_access_key: unset",
            "aws_secret_access_key: [REDACTED]",
        ),
        // A `user:password` after an HTTP client's `-u`, even all digits.
        ("curl -u 1000:1000 x", "curl -u [REDACTED] x"),
    ] {
        assert_eq!(redact_secrets(text), expected, "{text:?}");
    }
}

/// The quoted and restricted shapes redact to themselves too.
#[test]
fn redaction_is_idempotent_over_the_quoted_shapes() {
    for text in [
        "psql --password \"hunter 2\" && curl -u 'a:b c' x",
        "export STRIPE_KEY=\"sk_live_0123\" DB_PASS='p w'",
        "mysql -p\"x y\" && sshpass -p 'a b' ssh h && aws_secret_access_key \"wJalrXUtnFEMI/K7MDENG\"",
        "psql --password \"unterminated",
    ] {
        let once = redact_secrets(text);
        assert_eq!(redact_secrets(&once), once, "{text:?}");
    }
}
