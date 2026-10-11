use super::*;

fn entry(raw: &[u8]) -> Entry {
    Entry {
        mode: REGULAR_FILE.into(),
        oid: "a".repeat(40),
        name: String::from_utf8_lossy(raw).into_owned(),
        raw: raw.to_vec(),
    }
}

/// Every name quecto keeps shows exactly as it is, whatever byte it
/// holds: tried for every byte, alone and after a letter.
#[test]
fn every_name_quecto_keeps_shows_as_it_is() {
    let mut kept = 0;
    for byte in 0..=u8::MAX {
        for raw in [vec![byte], vec![b'a', byte], vec![b'a', byte, b'z']] {
            let name = entry(&raw);
            if name.kept_name() {
                kept += 1;
                assert_eq!(shown_name(&raw).as_bytes(), raw.as_slice(), "{raw:?}");
            }
        }
    }
    assert!(kept > 62 * 3, "letters, digits and punctuation are kept");
}
