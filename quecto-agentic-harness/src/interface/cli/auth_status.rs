use crate::infrastructure::auth::credential_store::CredentialStore;

pub(crate) fn cmd_auth_status(base: &std::path::Path, stdout: &mut String) -> i32 {
    // Read both stores and let the narrow OAuth store win on duplicate provider
    // names. This preserves visibility of legacy API tokens while preventing a
    // migrated OAuth credential from being reported as an API token.
    let legacy_store = CredentialStore::new(base);
    let oauth_store = CredentialStore::oauth(base);
    let mut statuses = match legacy_store.status_summary() {
        Ok(value) => value,
        Err(e) => {
            stdout.push_str(&format!("failed to read credentials: {}\n", e));
            return 1;
        }
    };
    let oauth_statuses = match oauth_store.status_summary() {
        Ok(value) => value,
        Err(e) => {
            stdout.push_str(&format!("failed to read OAuth credentials: {}\n", e));
            return 1;
        }
    };
    for oauth_status in oauth_statuses {
        statuses.retain(|status| status.provider != oauth_status.provider);
        statuses.push(oauth_status);
    }
    if statuses.is_empty() {
        stdout.push_str("no credentials stored\n");
    } else {
        stdout.push_str("Credentials:\n");
        for s in &statuses {
            stdout.push_str(&format!("  {} ({}) — {}\n", s.provider, s.method, s.status));
        }
    }
    0
}
