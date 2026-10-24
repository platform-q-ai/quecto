use super::*;

const ID: &str = "0123456789abcdef";

fn id() -> BoardId {
    BoardId::of_hex(ID).unwrap()
}

fn urls(list: &[&str]) -> Vec<String> {
    list.iter().map(|url| (*url).to_string()).collect()
}

/// The canonical file: `board_id` first, then each URL, one per line.
#[test]
fn a_moved_file_names_the_board_and_its_urls_one_per_line() {
    let moved = BoardMoved::new(
        id(),
        urls(&["https://board.example:7443", "http://100.64.0.2:7443/q"]),
    )
    .unwrap();
    let file = moved.file();
    assert_eq!(
        String::from_utf8(file.clone()).unwrap(),
        format!("board_id {ID}\nurl https://board.example:7443\nurl http://100.64.0.2:7443/q\n")
    );
    assert_eq!(BoardMoved::parse(&file), Ok(moved.clone()));
    assert_eq!(moved.board().as_str(), ID);
    assert_eq!(moved.urls().len(), 2);
}

/// The refusal names every URL and says the build reads the board only
/// through that server.
#[test]
fn a_moved_board_reads_as_a_refusal_naming_its_urls() {
    let moved = BoardMoved::new(id(), urls(&["https://a.example", "https://b.example"])).unwrap();
    assert_eq!(
        moved.to_string(),
        format!(
            "this board moved to a board server at https://a.example, https://b.example (board \
             id {ID}); this build reads it only through that server and never writes to its \
             `quecto/board` branch again: point this board at that server's URL to reach it"
        )
    );
}

/// Every MOVED file quecto would not write is refused: an allowlist of
/// keys, the board id's shape, http(s) URLs alone, and bounded counts.
#[test]
fn a_malformed_moved_file_is_refused() {
    let url = "url https://a.example\n";
    let board = format!("board_id {ID}\n");
    let many: String = (0..=MAX_MOVED_URLS)
        .map(|at| format!("url https://h{at}.example\n"))
        .collect();
    let long_url = format!("url https://a.example/{}\n", "p".repeat(MAX_URL_BYTES));
    let padded = format!("{board}{url}").into_bytes();
    let mut huge = padded.clone();
    huge.resize(MAX_MOVED_BYTES + 1, b'\n');
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", vec![]),
        ("no url", board.clone().into_bytes()),
        ("no board id", url.as_bytes().to_vec()),
        (
            "an unknown key",
            format!("{board}{url}mirror x\n").into_bytes(),
        ),
        ("a url first", format!("{url}{board}").into_bytes()),
        ("two board ids", format!("{board}{board}{url}").into_bytes()),
        (
            "upper-case id",
            format!("board_id {}\n{url}", ID.to_uppercase()).into_bytes(),
        ),
        ("short id", format!("board_id 0123\n{url}").into_bytes()),
        (
            "a path for an id",
            format!("board_id ../../etc\n{url}").into_bytes(),
        ),
        ("ftp", format!("{board}url ftp://a.example\n").into_bytes()),
        (
            "file",
            format!("{board}url file:///etc/passwd\n").into_bytes(),
        ),
        ("no host", format!("{board}url https://\n").into_bytes()),
        (
            "a hex IPv4 part",
            format!("{board}url http://0x7f.1\n").into_bytes(),
        ),
        (
            "a hex IPv4",
            format!("{board}url http://0x7f000001\n").into_bytes(),
        ),
        (
            "a hex octet",
            format!("{board}url http://1.0x2.3.4\n").into_bytes(),
        ),
        (
            "a number last",
            format!("{board}url http://foo.1\n").into_bytes(),
        ),
        (
            "userinfo",
            format!("{board}url https://u:p@a.example\n").into_bytes(),
        ),
        (
            "a space",
            format!("{board}url https://a.example/a b\n").into_bytes(),
        ),
        (
            "upper scheme",
            format!("{board}url HTTPS://a.example\n").into_bytes(),
        ),
        ("a repeated url", format!("{board}{url}{url}").into_bytes()),
        ("too many urls", format!("{board}{many}").into_bytes()),
        ("too long a url", format!("{board}{long_url}").into_bytes()),
        ("too many bytes", huge),
        (
            "no final newline",
            format!("{board}url https://a.example").into_bytes(),
        ),
        ("a blank line", format!("{board}\n{url}").into_bytes()),
        ("two spaces", format!("board_id  {ID}\n{url}").into_bytes()),
        (
            "a carriage return",
            format!("board_id {ID}\r\n{url}").into_bytes(),
        ),
        (
            "not utf-8",
            [board.as_bytes(), b"url https://\xff\n"].concat(),
        ),
    ];
    for (case, bytes) in cases {
        let parsed = BoardMoved::parse(&bytes);
        assert!(parsed.is_err(), "{case}: {parsed:?}");
    }
    // A host is DNS labels, a dotted IPv4 or a bracketed IPv6 address, and
    // a port is 1 to 65535.
    for bad in [
        "http://:",
        "http://[",
        "http://...",
        "http://-",
        "http://a:b:c",
        "http://a.example:99999",
        "http://a.example:0",
        "http://a.example:08",
        "http://a.example:",
        "http://-a.example",
        "http://a-.example",
        "http://a..example",
        "http://a.example.",
        "http://256.1.1.1",
        "http://1.2.3",
        "http://[::1",
        "http://[::1]x",
        "http://[nope]",
        "http://[::1]:65536",
        "http://[1:2:3:4:5:6:7:8:9]",
        "http://[1::2::3]",
        "http://[12345::1]",
        "http://[:::]",
        "http://01.2.3.4",
        "http://1.2.3.4.5",
        &format!("http://{}.example", "a".repeat(64)),
    ] {
        let refused = BoardMoved::new(id(), urls(&[bad]));
        assert!(refused.is_err(), "{bad}: {refused:?}");
    }
    for good in [
        "http://a",
        "https://board-1.example.com:65535/q",
        "http://100.64.0.2:1",
        "http://[::1]:7443",
        "https://[fd7a:115c:a1e0::1]",
        "http://[::]",
        "http://[1:2:3:4:5:6:7:8]",
        "http://0.0.0.0",
        &format!("http://{}.example", "a".repeat(63)),
    ] {
        let taken = BoardMoved::new(id(), urls(&[good]));
        assert!(taken.is_ok(), "{good}: {taken:?}");
    }
    assert!(BoardMoved::new(id(), vec![]).is_err());
    assert!(BoardMoved::new(id(), urls(&["ssh://a.example"])).is_err());
    // The bounds themselves are taken.
    let most: Vec<String> = (0..MAX_MOVED_URLS)
        .map(|at| format!("https://h{at}.example"))
        .collect();
    let moved = BoardMoved::new(id(), most).unwrap();
    assert!(moved.file().len() <= MAX_MOVED_BYTES);
    assert_eq!(BoardMoved::parse(&moved.file()), Ok(moved));
}
