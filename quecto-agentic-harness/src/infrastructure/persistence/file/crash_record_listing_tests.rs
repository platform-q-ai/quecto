use super::*;

fn filled(entries: usize) -> (tempfile::TempDir, RecordDir) {
    let base = tempfile::tempdir().unwrap();
    let dir = RecordDir::create(base.path()).unwrap();
    for n in 0..entries {
        std::fs::write(base.path().join("audit/crash").join(format!("p.{n}")), "").unwrap();
    }
    (base, dir)
}

/// #2192 review: a real listing whose `readdir` fails part-way — a null
/// entry with `errno` set — is reported incomplete, not taken for the
/// directory's end; the names read before the error are kept.
#[test]
fn a_listing_whose_readdir_fails_part_way_is_incomplete() {
    let (_base, dir) = filled(10);
    let keep = |name: &str| name.starts_with("p.");
    let fault = ReadFault {
        after: 5,
        errno: libc::EIO,
    };
    let failed = names_matching_faulted(&dir, &keep, 100, fault);
    assert!(!failed.complete, "{failed:?}");
    assert!(failed.names.len() <= 5, "{failed:?}");
    // The same listing without the fault reaches the end.
    let clean = names_matching_within(&dir, &keep, 100);
    assert_eq!((clean.names.len(), clean.complete), (10, true));
}

/// A failure at the very first read, and one just as the bound is
/// reached (the read that tells the end from more), are incomplete too.
#[test]
fn a_readdir_failure_at_either_edge_is_incomplete() {
    let (_base, dir) = filled(3);
    let keep = |_: &str| true;
    for fault in [
        ReadFault {
            after: 0,
            errno: libc::EIO,
        },
        // `.`, `..` and three files: the sixth read is the one past the bound.
        ReadFault {
            after: 5,
            errno: libc::ENOENT,
        },
    ] {
        let listed = names_matching_faulted(&dir, &keep, 5, fault);
        assert!(!listed.complete, "{fault:?}: {listed:?}");
    }
    let unfaulted = names_matching_faulted(
        &dir,
        &keep,
        5,
        ReadFault {
            after: usize::MAX,
            errno: libc::EIO,
        },
    );
    assert!(unfaulted.complete, "{unfaulted:?}");
}

#[test]
fn only_a_clean_end_is_the_end() {
    assert_eq!(listing_end(0), ListingEnd::End);
    assert_eq!(listing_end(libc::EIO), ListingEnd::Failed(libc::EIO));
}
