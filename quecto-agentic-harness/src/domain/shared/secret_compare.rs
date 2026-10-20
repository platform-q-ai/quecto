//! Comparing secrets (#1910 review): one owner for the constant-time
//! comparison every secret check uses (the admission authority's capability
//! tokens, its broker's owner token, an OAuth callback's state).

/// Whether `a` equals `b`, in time that depends only on their lengths, so a
/// secret cannot be guessed byte by byte from response timing.
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = a.len() ^ b.len();
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= usize::from(x ^ y);
    }
    diff == 0
}

#[cfg(test)]
#[path = "secret_compare_tests.rs"]
mod tests;
