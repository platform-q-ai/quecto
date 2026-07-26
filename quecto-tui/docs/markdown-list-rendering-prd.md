# PRD: Fix Markdown List Rendering in quecto-tui

## Problem

`quecto-tui`'s Markdown renderer incorrectly renders **all** lists as numbered lists. Unordered bullet lists render with `1.`, `2.`, `3.` instead of `•`.

### Example

Input:

```markdown
- item one
- item two
- item three
```

Actual output:

```text
1. item one
2. item two
3. item three
```

Expected output:

```text
• item one
• item two
• item three
```

Nested lists are also affected: a nested unordered list under an unordered parent renders as a nested numbered list, and mixed ordered/unordered nesting loses the distinction entirely.

## Root cause

In `src/components/markdown.rs`, the renderer tracks list state with:

```rust
let mut ordered_list_index: Vec<u64> = Vec::new();
```

On `Tag::List(start)`, it pushes:

```rust
ordered_list_index.push(start.unwrap_or(1));
```

For unordered lists, pulldown-cmark emits `Tag::List(None)`. The `unwrap_or(1)` turns every unordered list into a pseudo-ordered list starting at 1. The `Tag::Item` branch then sees a number on the stack and emits `<n>. ` instead of a bullet.

## Goals

1. Distinguish ordered and unordered lists at every nesting level.
2. Render unordered lists with a bullet (`•`).
3. Render ordered lists with the correct numeric index.
4. Support mixed ordered/unordered nesting correctly.
5. Wrap long list item text with proper hanging indent so continuation lines align under the item text, not the bullet.

## Non-goals

- Do not change heading, code block, table, blockquote, or inline styling behavior.
- Do not change the parser or the set of supported Markdown constructs.
- Do not add new list marker styles (e.g. roman numerals, letters) beyond bullets and numbers.

## Proposed implementation

### 1. Track list type per level

Change the stack from `Vec<u64>` to `Vec<Option<u64>>`:

```rust
let mut ordered_list_index: Vec<Option<u64>> = Vec::new();
```

On `Tag::List(start)`, push `start` directly:

```rust
Tag::List(start) => {
    flush_line(&mut current_line, &mut lines);
    list_depth += 1;
    ordered_list_index.push(start);
}
```

On `TagEnd::List(_)`:

```rust
TagEnd::List(_) => {
    list_depth = list_depth.saturating_sub(1);
    ordered_list_index.pop();
    if list_depth == 0 {
        flush_line(&mut current_line, &mut lines);
        lines.push(String::new());
    }
}
```

### 2. Choose marker based on list type

In the `Tag::Item` branch:

```rust
Tag::Item => {
    flush_line(&mut current_line, &mut lines);
    let indent = "  ".repeat(list_depth.saturating_sub(1));
    let bullet = match ordered_list_index.last_mut() {
        Some(Some(idx)) => {
            let num = *idx;
            *idx += 1;
            format!("{}{}. ", indent, num)
        }
        _ => {
            format!("{}{} ", indent, theme::accent("•"))
        }
    };
    current_line.push_str(&bullet);
}
```

### 3. Wrap list item text with hanging indent

When a list item line exceeds `content_width`, wrap it so continuation lines start at the same column as the item text after the marker.

Define a helper that knows the current list marker width:

```text
• item one that is long
  and continues here
1. ordered item that is long
   and continues here
```

Implementation approach:

- After `render_markdown` produces raw lines, or during line wrapping, detect lines that begin with a list marker.
- Compute the visual width of the marker prefix (including indent and trailing space).
- Use that width as the hanging indent when calling `wrap_text` for list item lines.

A simple way is to post-process lines after the main render loop:

```rust
for line in &lines {
    if line.is_empty() {
        result.push(String::new());
    } else {
        let (prefix, rest) = split_list_marker(line);
        let prefix_width = visible_width(&prefix);
        let available = content_width.saturating_sub(prefix_width);
        if available > 0 && visible_width(rest) > available {
            let wrapped = wrap_text(rest, available);
            result.push(format!("{}{}{}", pad, prefix, wrapped[0]));
            for w in &wrapped[1..] {
                result.push(format!("{}{}{}", pad, " ".repeat(prefix_width), w));
            }
        } else {
            result.push(format!("{}{}", pad, line));
        }
    }
}
```

`split_list_marker` should recognize:

- `• ` (bullet)
- `N. ` (ordered marker)
- Leading spaces before the marker

Alternatively, track the per-item prefix width during rendering and pass it through the line data. Post-processing is simpler and keeps the existing render loop mostly intact.

### 4. Tests to add

In `src/components/markdown_tests.rs`:

- `unordered_list_uses_bullets`: assert unordered list renders `•` markers.
- `ordered_list_uses_numbers`: assert ordered list renders `1.`, `2.`, etc.
- `nested_unordered_list_uses_bullets`: assert nested unordered list under unordered parent uses bullets at both levels.
- `mixed_ordered_unordered_nesting`: assert `1. a / • b / • c / 2. d` shape.
- `unordered_list_long_item_wraps_with_hanging_indent`: assert continuation lines align under the item text.
- `ordered_list_long_item_wraps_with_hanging_indent`: assert continuation lines align under the item text, accounting for the wider `10. ` marker.

## Acceptance criteria

1. Unordered lists render with `•` bullets.
2. Ordered lists render with correct numbers.
3. Mixed and nested ordered/unordered lists preserve marker type per level.
4. Long list item text wraps with a hanging indent matching the marker width.
5. Existing Markdown tests continue to pass.
6. New tests cover the bullet/number distinction and wrapping behavior.
