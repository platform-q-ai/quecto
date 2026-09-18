//! `agent_cmd get_container_configs` and the spawn description's roster
//! line (#2024 S4c): the container configs the launching agent can name,
//! as launch policy would honour them. The port reports the effective set
//! of the agent's checkout (global file plus its trusted overlay); this
//! query owns the rules an agent relies on: the `container: true` default
//! comes first and is the ONLY entry marked default; while the checkout's
//! overlay is withheld (untrusted, refused or unparseable and able to have
//! changed the set) no entry is marked default — an implicit selection is
//! refused until `quecto config trust`, and the diagnostics say so; an
//! entry a launch would refuse (missing or unsafe argv) is never marked
//! default and carries its problem, and more than one labelled default
//! marks none (a launch would refuse the implicit selection) — each with
//! a diagnostic line; the rest follow by name so the listing is stable
//! between calls.

use std::sync::Arc;

use crate::application::environments::dto::ContainerConfigInventory;
use crate::application::environments::ports::ContainerConfigRoster;

pub struct ListContainerConfigs {
    roster: Arc<dyn ContainerConfigRoster>,
}

impl ListContainerConfigs {
    pub fn new(roster: Arc<dyn ContainerConfigRoster>) -> Self {
        Self { roster }
    }

    /// The roster's revision token: a presenter caches its rendering
    /// against it (see the port).
    pub fn revision(&self) -> String {
        self.roster.revision()
    }

    pub fn execute(&self) -> Result<ContainerConfigInventory, String> {
        let report = self.roster.roster()?;
        let mut configs = report.configs;
        let mut diagnostics = report.diagnostics;
        // Counted as configured, before any rule clears a flag: launch
        // policy refuses the implicit selection whenever the configured
        // set labels more than one default, broken entries included.
        let mut defaults: Vec<String> = configs
            .iter()
            .filter(|entry| entry.default)
            .map(|entry| entry.name.clone())
            .collect();
        defaults.sort_unstable();
        if report.overlay_withheld {
            for entry in &mut configs {
                entry.default = false;
            }
        }
        for entry in &mut configs {
            if let Some(problem) = &entry.problem {
                diagnostics.push(format!(
                    "container config '{}' cannot launch as configured: {problem}",
                    entry.name
                ));
                entry.default = false;
            }
        }
        if defaults.len() > 1 {
            diagnostics.push(format!(
                "multiple container configs are labeled \"default\": true ({}); container: true is refused until exactly one is",
                defaults.join(", ")
            ));
            for entry in &mut configs {
                entry.default = false;
            }
        }
        configs.sort_by(|a, b| b.default.cmp(&a.default).then_with(|| a.name.cmp(&b.name)));
        Ok(ContainerConfigInventory {
            configs,
            overlay_withheld: report.overlay_withheld,
            diagnostics,
        })
    }
}

impl std::fmt::Debug for ListContainerConfigs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListContainerConfigs")
            .finish_non_exhaustive()
    }
}
