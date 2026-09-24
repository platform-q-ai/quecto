use super::*;
use std::path::PathBuf;

fn files(paths: &[&str]) -> Vec<ListedFile> {
    paths
        .iter()
        .map(|p| ListedFile {
            path: (*p).into(),
            count: None,
        })
        .collect()
}

#[test]
fn files_mode_reads_null_separated_paths_whole() {
    assert_eq!(
        parse_listing("/ws/a:b.rs\0/ws/c d.py\0", OutputMode::Files),
        files(&["/ws/a:b.rs", "/ws/c d.py"])
    );
    assert!(parse_listing("", OutputMode::Files).is_empty());
}

#[test]
fn count_mode_reads_path_then_count_and_skips_what_does_not_parse() {
    assert_eq!(
        parse_listing(
            "/ws/a:b.rs\x002\n/ws/c.py\x0011\nno-null-here 3\n/ws/d\0x\n",
            OutputMode::Count
        ),
        vec![
            ListedFile {
                path: "/ws/a:b.rs".into(),
                count: Some(2)
            },
            ListedFile {
                path: "/ws/c.py".into(),
                count: Some(11)
            },
        ]
    );
}

fn format(listing: Vec<ListedFile>, limit: usize, max_output_bytes: usize) -> String {
    format_listing(
        listing,
        &ListingFormat {
            workspace: &PathBuf::from("/ws"),
            sandbox: &Sandbox::new(None),
            limit,
            max_output_bytes,
        },
    )
}

#[test]
fn files_are_listed_in_path_order_relative_to_the_workspace() {
    assert_eq!(
        format(files(&["/ws/z.rs", "/ws/a.rs", "/else/b.rs"]), 10, 1024),
        "/else/b.rs\na.rs\nz.rs"
    );
    assert_eq!(format(vec![], 10, 1024), "No matches found");
}

#[test]
fn counts_are_listed_busiest_first_then_by_path() {
    let listing = vec![
        ListedFile {
            path: "/ws/b.rs".into(),
            count: Some(1),
        },
        ListedFile {
            path: "/ws/c.rs".into(),
            count: Some(9),
        },
        ListedFile {
            path: "/ws/a.rs".into(),
            count: Some(1),
        },
    ];
    assert_eq!(format(listing, 10, 1024), "c.rs: 9\na.rs: 1\nb.rs: 1");
}

#[test]
fn a_listing_is_capped_by_the_limit_and_the_output_budget() {
    let many = files(&["/ws/a", "/ws/b", "/ws/c"]);
    assert_eq!(
        format(many.clone(), 2, 1024),
        "a\nb\n\n[2 of 3 files shown. Use limit=4 for more, or refine pattern]"
    );
    let capped = format(many, 10, 3);
    assert!(capped.starts_with("a\n\n["), "{capped}");
    assert!(capped.contains("limit reached"), "{capped}");
}

#[test]
fn a_record_cut_off_by_the_output_cap_is_dropped() {
    assert_eq!(
        parse_listing("/ws/a.rs\0/ws/b.r", OutputMode::Files),
        files(&["/ws/a.rs"])
    );
    assert_eq!(
        parse_listing("/ws/a.rs\x0012\n/ws/b.rs\x001", OutputMode::Count),
        vec![ListedFile {
            path: "/ws/a.rs".into(),
            count: Some(12)
        }],
        "a count without its newline may be short"
    );
}

#[test]
fn a_count_record_keeps_a_path_holding_a_newline_whole() {
    assert_eq!(
        parse_listing("/ws/nl\nname.txt\x001\n", OutputMode::Count),
        vec![ListedFile {
            path: "/ws/nl\nname.txt".into(),
            count: Some(1)
        }]
    );
}

#[test]
fn a_search_of_dot_shows_paths_without_the_dot() {
    assert_eq!(
        format(files(&["/ws/./src/a.rs", "./b.rs"]), 10, 1024),
        "b.rs\nsrc/a.rs"
    );
}
