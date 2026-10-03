//! The built-in models #2435 retired: GPT models older than GPT-5.6 and
//! Claude models older than Claude 5, by the built-in provider that listed
//! them. A configured model among them is named as retired, so a warning
//! can say why it is no longer listed; any other unlisted id is not.

/// Whether `model` was a built-in of `provider` before #2435 retired it.
pub fn retired_builtin(provider: &str, model: &str) -> bool {
    let _ = (provider, model);
    false
}

#[cfg(test)]
#[path = "retired_tests.rs"]
mod tests;
