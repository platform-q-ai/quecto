use crate::infrastructure::auth::credential_store::CredentialStore;

pub(crate) fn cmd_auth_logout(
    base: &std::path::Path,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let mut provider: Option<String> = None;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--provider" => {
                if i + 1 < args.len() {
                    provider = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    stderr.push_str("auth logout: --provider requires a value\n");
                    return 1;
                }
            }
            other if other.starts_with("--") => {
                stderr.push_str(&format!("auth logout: unknown flag '{}'\n", other));
                return 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    let Some(provider) = provider else {
        stderr.push_str("auth logout: --provider is required\n");
        return 1;
    };

    // Logout is deliberately a revocation operation, not a store-selection
    // operation. During migration the same provider can exist in both files;
    // removing only the first match leaves a usable secret behind. Inspect each
    // primary file and remove every matching entry, while avoiding creation of
    // an empty dedicated file for a provider that was never migrated.
    let oauth_store = CredentialStore::oauth(base);
    let legacy_store = CredentialStore::new(base);
    let oauth_file_present = match std::fs::symlink_metadata(oauth_store.path()) {
        Ok(metadata) if metadata.is_file() => true,
        Ok(_) => {
            stderr.push_str("auth logout: failed to remove credential: OAuth path is not a regular file\n");
            return 1;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            stderr.push_str(&format!("auth logout: failed to remove credential: {}\n", error));
            return 1;
        }
    };
    let oauth_present = if oauth_file_present {
        match oauth_store.primary_exists(&provider) {
            Ok(value) => value,
            Err(e) => {
                stderr.push_str(&format!("auth logout: failed to remove credential: {}\n", e));
                return 1;
            }
        }
    } else {
        false
    };
    let legacy_file_present = match std::fs::symlink_metadata(legacy_store.path()) {
        Ok(metadata) if metadata.is_file() => true,
        Ok(_) => {
            stderr.push_str("auth logout: failed to remove credential: credential path is not a regular file\n");
            return 1;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            stderr.push_str(&format!("auth logout: failed to remove credential: {}\n", error));
            return 1;
        }
    };
    let legacy_present = if legacy_file_present {
        match legacy_store.primary_exists(&provider) {
            Ok(value) => value,
            Err(e) => {
                stderr.push_str(&format!("auth logout: failed to remove credential: {}\n", e));
                return 1;
            }
        }
    } else {
        false
    };
    let mut removed = false;
    if oauth_present && oauth_file_present {
        match oauth_store.remove(&provider) {
            Ok(was_removed) => removed |= was_removed,
            Err(e) => {
                stderr.push_str(&format!("auth logout: failed to remove OAuth credential: {}\n", e));
                return 1;
            }
        }
    }
    if legacy_present {
        match legacy_store.remove(&provider) {
            Ok(was_removed) => removed |= was_removed,
            Err(e) => {
                stderr.push_str(&format!("auth logout: failed to remove credential: {}\n", e));
                return 1;
            }
        }
    }
    if removed {
        stdout.push_str(&format!("Credential removed for {}\n", provider));
    } else {
        stdout.push_str(&format!("no credential found for {}\n", provider));
    }
    0
}
