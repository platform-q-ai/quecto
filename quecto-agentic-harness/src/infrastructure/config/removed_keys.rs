//! #2414: watermark is the only context mode. The keys of the pruning
//! rules it replaced, and the switch between the two, are refused at load,
//! naming the key: no silent ignore, no compatibility shim.

#[cfg(test)]
#[path = "removed_keys_tests.rs"]
mod tests;
