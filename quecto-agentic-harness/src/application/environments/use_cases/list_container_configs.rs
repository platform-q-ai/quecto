//! `agent_cmd get_container_configs` and the spawn description's roster
//! line (#2024 S4c): the container configs the launching agent can name,
//! as launch policy would honour them. The port reports the effective set
//! of the agent's checkout (global file plus its trusted overlay); this
//! query owns the rules an agent relies on: the `container: true` default
//! comes first and is the ONLY entry marked default; while the checkout's
//! overlay is withheld (untrusted, refused or unparseable and able to have
//! changed the set) no entry is marked default — an implicit selection is
//! refused until `quecto config trust`, and the diagnostics say so; the
//! rest follow by name so the listing is stable between calls.

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

    pub fn execute(&self) -> Result<ContainerConfigInventory, String> {
        let report = self.roster.roster()?;
        let mut configs = report.configs;
        if report.overlay_withheld {
            for entry in &mut configs {
                entry.default = false;
            }
        }
        // Exactly one default is the configuration capability's rule for a
        // non-empty applied set; the query still orders defensively so a
        // bypass cannot present two "defaults" ahead of the rest.
        configs.sort_by(|a, b| b.default.cmp(&a.default).then_with(|| a.name.cmp(&b.name)));
        Ok(ContainerConfigInventory {
            configs,
            overlay_withheld: report.overlay_withheld,
            diagnostics: report.diagnostics,
        })
    }
}

impl std::fmt::Debug for ListContainerConfigs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListContainerConfigs")
            .finish_non_exhaustive()
    }
}
