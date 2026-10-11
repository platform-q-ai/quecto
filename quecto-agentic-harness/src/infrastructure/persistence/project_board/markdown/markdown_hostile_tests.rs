//! Hostile documents: each refused when written, with its reason named.
use super::*;

type Named = fn(&MarkdownError) -> bool;

#[test]
fn hostile_documents_are_refused_when_written() {
    let with = |change: fn(&mut Document)| {
        let mut document = example();
        change(&mut document);
        render(&document, &SHAPE)
    };
    use MarkdownError as E;
    #[rustfmt::skip]
    let cases: Vec<(&str, Result<String, E>, Named)> = vec![
        ("a newline in a cell", with(|d| d.fields[1].1 = "re\nview".into()), |e| matches!(e, E::Cell { .. })),
        ("an empty cell", with(|d| d.fields[1].1 = String::new()), |e| matches!(e, E::Cell { .. })),
        ("a padded cell", with(|d| d.fields[1].1 = " review".into()), |e| matches!(e, E::Cell { .. })),
        ("HTML in a title", with(|d| d.title = "a <script>".into()), |e| matches!(e, E::Html { line: 2 })),
        ("bidi in a cell", with(|d| d.fields[1].1 = "re\u{2067}view".into()), |e| matches!(e, E::Bidi { .. })),
        ("empty prose", with(|d| d.sections[0].body = Body::Prose(String::new())), |e| matches!(e, E::EmptyProse { .. })),
        ("two blank lines", with(|d| d.sections[0].body = Body::Prose("a\n\n\nb".into())), |e| matches!(e, E::Structure { .. })),
        ("a row of one cell", with(|d| d.sections[1].body = Body::Table(vec![vec!["x".into()]])), |e| matches!(e, E::Cell { .. })),
        ("a field no task has", with(|d| d.fields[0].0 = "Owner".into()), |e| matches!(e, E::UnknownField { .. })),
        ("a prose section as a table", with(|d| d.sections[0].body = Body::Table(vec![])), |e| matches!(e, E::UnknownSection { .. })),
        ("a two-line title", with(|d| d.title = "a\nb".into()), |e| matches!(e, E::Title)),
    ];
    for (name, outcome, named) in cases {
        assert!(outcome.as_ref().is_err_and(named), "{name}: {outcome:?}");
    }
    // A joiner between emoji is how a family is written: it is kept.
    let family = with(|d| d.title = "👨\u{200d}👩\u{200d}👧".into());
    assert!(family.is_ok(), "{family:?}");
}
