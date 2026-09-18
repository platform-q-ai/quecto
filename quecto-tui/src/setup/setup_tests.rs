use super::*;

#[test]
fn bare_setup_is_the_all_areas_walkthrough() {
    assert_eq!(
        SetupCommand::parse(""),
        SetupCommand::Walkthrough(SetupArea::All)
    );
    assert_eq!(
        SetupCommand::parse("   "),
        SetupCommand::Walkthrough(SetupArea::All)
    );
}

#[test]
fn area_variants_parse_and_container_aliases_podman() {
    assert_eq!(
        SetupCommand::parse("admission"),
        SetupCommand::Walkthrough(SetupArea::Admission)
    );
    assert_eq!(
        SetupCommand::parse("podman"),
        SetupCommand::Walkthrough(SetupArea::Container)
    );
    assert_eq!(
        SetupCommand::parse(" container "),
        SetupCommand::Walkthrough(SetupArea::Container)
    );
    assert_eq!(
        SetupCommand::parse("auth"),
        SetupCommand::Walkthrough(SetupArea::Auth)
    );
}

#[test]
fn model_variant_takes_exactly_one_plain_id() {
    assert_eq!(
        SetupCommand::parse("model openai-api/gpt-5.5"),
        SetupCommand::Walkthrough(SetupArea::Model("openai-api/gpt-5.5".into()))
    );
    assert_eq!(
        SetupCommand::parse("model anthropic/claude-opus-5:latest_v1.2"),
        SetupCommand::Walkthrough(SetupArea::Model(
            "anthropic/claude-opus-5:latest_v1.2".into()
        ))
    );
    for bad in [
        "model",
        "model a b",
        "model \"x\"",
        "model a`b",
        "model a'b",
        "model a\\b",
        "model x$(y)",
        "model p/x$(y)",
        "model no-slash",
        "model --show-secrets",
        "model /m",
        "model p/",
        "model a/b/c",
        "model p/m;rm",
        "model p/m\u{e9}",
    ] {
        assert_eq!(SetupCommand::parse(bad), SetupCommand::Usage, "{bad:?}");
    }
}

#[test]
fn unknown_or_trailing_arguments_are_usage() {
    for bad in [
        "frobnicate",
        "Admission",
        "admission now",
        "auth me",
        "podman x",
    ] {
        assert_eq!(SetupCommand::parse(bad), SetupCommand::Usage, "{bad:?}");
    }
    for variant in ["model <model-id>", "admission", "podman", "auth"] {
        assert!(SETUP_USAGE.contains(variant));
    }
}

#[test]
fn all_areas_prompt_matches_the_epic_wording_and_names_setup_page() {
    let prompt = setup_walkthrough_prompt(&SetupArea::All);
    assert!(prompt.starts_with("Set up quecto for this folder/machine. First read the docs page `setup` (`docs {\"name\":\"setup\"}`)"));
    for needle in [
        "`quecto auth status`",
        "`quecto status`",
        "the default model",
        "the admission broker",
        "container config",
        "report the current state in one line",
        "ASK before writing or installing anything",
        "Never print secrets",
        "how to roll each change back",
    ] {
        assert!(prompt.contains(needle), "{needle}");
    }
}

#[test]
fn each_area_prompt_names_its_runbook_page_and_goal() {
    let cases: [(SetupArea, &str, &[&str]); 4] = [
        (
            SetupArea::Model("prov/m-1".into()),
            "docs {\"name\":\"models\"}",
            &[
                "`prov/m-1`",
                "agents.defaults.model",
                "quecto config unset agents.defaults.model",
            ],
        ),
        (
            SetupArea::Admission,
            "docs {\"name\":\"admission-broker\"}",
            &[
                "quecto admission-broker status",
                "install-service",
                "--global admission",
            ],
        ),
        (
            SetupArea::Container,
            "docs {\"name\":\"container-runtime\"}",
            &[
                "quecto container init",
                "quecto container doctor",
                "image-build line it prints",
            ],
        ),
        (
            SetupArea::Auth,
            "docs {\"name\":\"models\"}",
            &[
                "quecto auth status",
                "quecto auth login --provider",
                "--token",
                "quecto auth logout",
            ],
        ),
    ];
    for (area, page, needles) in cases {
        let prompt = setup_walkthrough_prompt(&area);
        assert!(prompt.contains(page), "{area:?} should name {page}");
        assert!(
            prompt.contains("docs {\"name\":\"setup\"}"),
            "{area:?} points at the setup row"
        );
        for needle in needles {
            assert!(
                prompt.contains(needle),
                "{area:?} should contain {needle:?}:\n{prompt}"
            );
        }
    }
}

#[test]
fn prompt_prose_never_names_a_container_runtime() {
    // The `setup` page names podman; the prompt only points at the page and
    // the runbook's own output, so the TUI text stays runtime-neutral.
    for area in [SetupArea::All, SetupArea::Container] {
        let prompt = setup_walkthrough_prompt(&area).to_lowercase();
        for word in ["podman", "docker"] {
            assert!(!prompt.contains(word), "{area:?} names {word}:\n{prompt}");
        }
    }
    assert!(SETUP_FROM_MASTER.starts_with(
        "Run /setup from the master session: select it (Esc from the sub-agent), then /setup again"
    ));
}

#[test]
fn every_prompt_carries_the_affirmative_safety_rules() {
    for area in [
        SetupArea::All,
        SetupArea::Model("x/y".into()),
        SetupArea::Admission,
        SetupArea::Container,
        SetupArea::Auth,
    ] {
        let prompt = setup_walkthrough_prompt(&area);
        for needle in [
            "ASK before writing or installing anything",
            "wait for my answer",
            "never pass `--show-secrets`",
            "never `cat` `credentials.json`",
            "`quecto auth login` always with `--token`",
            "`quecto admission-broker install-service --dry-run` first",
            "never run `quecto admission-broker run` from a tool call",
            "One key per `quecto config set`",
            "say which file changed",
        ] {
            assert!(prompt.contains(needle), "{area:?} must say {needle:?}");
        }
        assert!(
            !prompt.contains("--show-secrets`)"),
            "no positive --show-secrets instruction"
        );
        assert!(prompt.ends_with('.'), "prompt ends cleanly for {area:?}");
        assert!(!prompt.contains('\t'));
    }
}
