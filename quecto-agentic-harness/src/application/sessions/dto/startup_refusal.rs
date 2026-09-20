//! Command-line wording for refusal to start a saved session in this folder.
use super::resume_saved_session::ResumeDisposition;
use crate::domain::session_open_command::cd_there_command;
use crate::domain::session_path_text::display_path;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupRefusal {
    pub key: String,
    pub disposition: ResumeDisposition,
    pub execution_dir: Option<PathBuf>,
}

impl std::fmt::Display for StartupRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "session '{}' cannot start here: {}",
            self.key, self.disposition
        )?;
        let Some(dir) = &self.execution_dir else {
            return f.write_str(". The saved transcript was not changed.");
        };
        write!(f, ". Open quecto there: {}", display_path(dir))?;
        // The reader ran this command themselves: they repeat it from there.
        if let Some(cd) = cd_there_command(dir) {
            write!(f, "\n{cd}\nthen run the same command again")?;
        }
        Ok(())
    }
}
