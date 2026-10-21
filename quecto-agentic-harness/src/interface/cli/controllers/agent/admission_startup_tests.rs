use super::*;
use crate::application::admission::dto::negotiation::NegotiationPlan;
use crate::application::admission::ports::{AuthorityLauncher, LaunchedAuthority};
use crate::application::configuration::dto::ConfigLayers;
use std::sync::{Arc, Mutex};

/// A launcher whose authority never answers and whose launches fail,
/// recording what it was asked to start.
#[derive(Default)]
struct Unstartable {
    launches: Mutex<Vec<AuthorityLaunch>>,
}

impl AuthorityLauncher for Unstartable {
    fn probe(&self, _directory: &Path) -> Result<(), String> {
        Err("connection refused".into())
    }

    fn launch(&self, launch: &AuthorityLaunch) -> Result<Box<dyn LaunchedAuthority>, String> {
        self.launches.lock().unwrap().push(launch.clone());
        Err("fake launch refused".into())
    }

    fn pause(&self, _attempt: u32) {}
}

fn use_case() -> NegotiateAuthority {
    NegotiateAuthority::new(Arc::new(Unstartable::default()))
}

const VALID: &str = r#"{"directory":"/tmp/quecto-authority","groups":{"g":{"capacity":1,"reserve":0,"min_interval_ms":1,"queue_capacity":1,"queue_timeout_ms":1,"attempt_timeout_ms":1,"fallback_base_ms":1,"max_cooldown_ms":1}},"aliases":{"a":"g"},"bindings":{"openai":"a"}}"#;
const INVALID: &str = r#"{"directory":"relative","groups":{"g":{"capacity":1,"reserve":0,"min_interval_ms":1,"queue_capacity":1,"queue_timeout_ms":1,"attempt_timeout_ms":1,"fallback_base_ms":1,"max_cooldown_ms":1}},"aliases":{"a":"g"},"bindings":{"openai":"a"}}"#;

fn config(section: Option<&str>) -> Config {
    let mut config = Config::default();
    config.admission = section.map(|s| serde_json::from_str(s).unwrap());
    config
}

#[test]
fn a_root_validates_its_section_and_a_child_inherits_whatever_it_says() {
    let use_case = use_case();
    let context = Path::new("/tmp/child.ctx");
    // A root: a valid section is the authority to join; an invalid one stops
    // startup here, naming the error.
    assert_eq!(
        plan(&config(Some(VALID)), None, &use_case).unwrap(),
        NegotiationPlan::Root {
            directory: "/tmp/quecto-authority".into()
        }
    );
    let error = plan(&config(Some(INVALID)), None, &use_case).unwrap_err();
    assert!(error.contains("absolute"), "{error}");
    assert_eq!(
        plan(&config(None), None, &use_case).unwrap(),
        NegotiationPlan::Disabled
    );
    // A child inherits unconditionally: its own `--config` may carry no
    // section, a valid one, or an *invalid* one — none of that is its
    // business (the section is global-only and the parent already composed).
    for section in [None, Some(VALID), Some(INVALID)] {
        assert_eq!(
            plan(&config(section), Some(context), &use_case).unwrap(),
            NegotiationPlan::Child {
                context: context.to_path_buf()
            },
            "section {section:?}"
        );
    }
}

fn layered(global: &str) -> ConfigSelection {
    ConfigSelection::Layered(ConfigLayers {
        global: PathBuf::from(global),
        overlay: None,
        overlay_origin: Default::default(),
        legacy_local: None,
    })
}

/// #2615: a broker started on demand serves the global config the root's
/// section came from, absolute, with the root's base directory.
#[test]
fn a_launch_serves_the_global_config_the_section_came_from() {
    let base = Path::new("/home/me/.quecto");
    let layered = layered("/home/me/.quecto/config.json");
    let launch = launch_request(
        Path::new("/home/me/.quecto/admission"),
        &ConfigOrigin {
            base_dir: base,
            selection: &layered,
        },
    );
    assert_eq!(
        launch,
        AuthorityLaunch {
            directory: "/home/me/.quecto/admission".into(),
            config: "/home/me/.quecto/config.json".into(),
            base_dir: base.into(),
        }
    );
    let explicit = ConfigSelection::Explicit("relative/config.json".into());
    let launch = launch_request(
        Path::new("/a"),
        &ConfigOrigin {
            base_dir: Path::new("rel-base"),
            selection: &explicit,
        },
    );
    assert!(launch.config.is_absolute(), "{}", launch.config.display());
    assert!(launch.config.ends_with("relative/config.json"));
    assert!(
        launch.base_dir.is_absolute(),
        "{}",
        launch.base_dir.display()
    );
}

/// #2615: a root whose broker does not answer has one started on demand;
/// when that cannot start, startup stops with the reason.
#[test]
fn a_root_starts_its_broker_on_demand_and_stops_when_it_cannot() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("authority");
    let section = VALID.replace("/tmp/quecto-authority", &directory.to_string_lossy());
    let launcher = Arc::new(Unstartable::default());
    let use_case = NegotiateAuthority::new(launcher.clone());
    let selection = layered("/g/config.json");
    let origin = ConfigOrigin {
        base_dir: temp.path(),
        selection: &selection,
    };
    let mut stderr = String::new();
    assert_eq!(
        negotiate(
            &config(Some(&section)),
            &origin,
            None,
            &use_case,
            &mut stderr
        ),
        None
    );
    assert!(
        stderr.starts_with("agent: admission negotiation failed: "),
        "{stderr}"
    );
    assert!(stderr.contains("fake launch refused"), "{stderr}");
    let launches = launcher.launches.lock().unwrap();
    assert_eq!(launches.len(), 1, "one launch, then the failure is final");
    assert_eq!(launches[0].directory, directory);
    // Disabled starts nothing.
    let mut stderr = String::new();
    assert_eq!(
        negotiate(&config(None), &origin, None, &use_case, &mut stderr),
        Some(Joined::Disabled)
    );
    assert_eq!(launches.len(), 1);
}
