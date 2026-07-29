use std::fs;
use std::path::{Path, PathBuf};

pub const PHASE_0_DOCS: &[&str] = &[
    "docs/prd/prd-harness-architecture-hardening.md",
    "docs/architecture-design-records/README.md",
    "docs/docs-tool-embeds/uds-protocol.md",
    "docs/architecture/harness-architecture-map.md",
    "docs/architecture/protocol-capability-matrix.md",
    "docs/docs-tool-embeds/contributor-cookbooks.md",
];

pub const PHASE_0_ADRS: &[&str] = &[
    "docs/architecture-design-records/adr-0012-explicit-agent-turn-state-machine.md",
    "docs/architecture-design-records/adr-0013-uds-command-family-router.md",
    "docs/architecture-design-records/adr-0014-context-management-is-a-first-class-application-subsystem.md",
    "docs/architecture-design-records/adr-0015-subagent-lifecycle-state-machine.md",
    "docs/architecture-design-records/adr-0016-typed-identifiers-for-protocol-and-session-boundaries.md",
    "docs/architecture-design-records/adr-0017-protocol-evolution-matrix.md",
    "docs/architecture-design-records/adr-0018-contributor-change-cookbooks.md",
    "docs/architecture-design-records/adr-0019-role-segregated-domain-ports.md",
];

pub fn phase_0_hardening_docs() -> impl Iterator<Item = &'static str> {
    PHASE_0_DOCS.iter().chain(PHASE_0_ADRS.iter()).copied()
}

#[derive(Debug, Default)]
pub struct LinkCheckReport {
    pub checked: Vec<String>,
    pub missing: Vec<String>,
}

impl LinkCheckReport {
    pub fn is_clean(&self) -> bool {
        self.missing.is_empty()
    }
}

pub fn check_phase_0_hardening_links(repo: &Path) -> LinkCheckReport {
    let mut report = LinkCheckReport::default();

    for path in phase_0_hardening_docs() {
        let full_path = repo.join(path);
        if !full_path.exists() {
            report.missing.push(format!("missing document: {path}"));
            continue;
        }
        report.checked.push(path.to_string());

        let Ok(content) = fs::read_to_string(&full_path) else {
            report.missing.push(format!("unreadable document: {path}"));
            continue;
        };
        let Some(parent) = full_path.parent() else {
            report
                .missing
                .push(format!("document has no parent: {path}"));
            continue;
        };
        for target in markdown_link_targets(&content) {
            if is_external_or_anchor(&target) {
                continue;
            }
            let resolved = normalize_link_target(parent, &target);
            report.checked.push(format!("{path} -> {target}"));
            if !resolved.exists() {
                report.missing.push(format!("{path} -> {target}"));
            }
        }
    }

    report
}

fn markdown_link_targets(content: &str) -> Vec<String> {
    let mut targets = Vec::new();
    for candidate in content.split("](").skip(1) {
        if let Some(target) = candidate.split(')').next() {
            targets.push(target.split('#').next().unwrap_or(target).to_string());
        }
    }
    targets
}

fn is_external_or_anchor(target: &str) -> bool {
    target.is_empty()
        || target.starts_with('#')
        || target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("mailto:")
}

fn normalize_link_target(parent: &Path, target: &str) -> PathBuf {
    let mut path = PathBuf::from(parent);
    for component in Path::new(target).components() {
        match component {
            std::path::Component::ParentDir => {
                path.pop();
            }
            std::path::Component::CurDir => {}
            other => path.push(other.as_os_str()),
        }
    }
    path
}
