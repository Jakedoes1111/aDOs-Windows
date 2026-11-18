//! Preview-only Markdown hard-break conversion.
//!
//! The editor keeps soft line breaks as typed. The preview should show each
//! line of a prose paragraph on its own line, so this helper inserts Markdown
//! hard breaks (two trailing spaces) at soft line endings. It uses the same
//! `markdown` parser as the rest of the UI to find paragraph and inline-code
//! byte ranges, so code, lists, tables and headings are left alone.

use markdown::{mdast::Node, ParseOptions};

pub(crate) fn hard_breaks(text: &str) -> String {
    if text.is_empty() || (!text.contains('\n') && !text.contains('\r')) {
        return text.to_owned();
    }

    let Ok(root) = markdown::to_mdast(text, &ParseOptions::gfm()) else {
        return text.to_owned();
    };

    let mut paragraphs = Vec::new();
    let mut inline_code = Vec::new();
    collect_ranges(&root, false, &mut paragraphs, &mut inline_code);
    if paragraphs.is_empty() {
        return text.to_owned();
    }

    paragraphs.sort_unstable();
    inline_code.sort_unstable();

    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len() + paragraphs.len() * 2);
    let mut cursor = 0;

    for (start, end) in paragraphs {
        if start < cursor
            || start >= end
            || end > text.len()
            || !text.is_char_boundary(start)
            || !text.is_char_boundary(end)
        {
            continue;
        }

        out.push_str(&text[cursor..start]);
        let mut i = start;
        while i < end {
            match bytes[i] {
                b'\r' | b'\n' => {
                    let newline_start = i;
                    let newline_end =
                        if bytes[i] == b'\r' && i + 1 < end && bytes[i + 1] == b'\n' {
                            i + 2
                        } else {
                            i + 1
                        };

                    if !inside_inline_code(newline_start, &inline_code) {
                        add_break_if_soft(bytes, start, newline_start, &mut out);
                    }
                    out.push_str(&text[newline_start..newline_end]);
                    i = newline_end;
                }
                _ => {
                    let next = next_line_ending(bytes, i, end);
                    out.push_str(&text[i..next]);
                    i = next;
                }
            }
        }
        cursor = end;
    }

    out.push_str(&text[cursor..]);
    out
}

fn collect_ranges(
    node: &Node,
    in_list: bool,
    paragraphs: &mut Vec<(usize, usize)>,
    inline_code: &mut Vec<(usize, usize)>,
) {
    let in_list = in_list || matches!(node, Node::List(_) | Node::ListItem(_));

    match node {
        Node::Paragraph(paragraph) if !in_list => {
            if let Some(position) = &paragraph.position {
                paragraphs.push((position.start.offset, position.end.offset));
            }
        }
        Node::InlineCode(code) => {
            if let Some(position) = &code.position {
                inline_code.push((position.start.offset, position.end.offset));
            }
        }
        _ => {}
    }

    if let Some(children) = node.children() {
        for child in children {
            collect_ranges(child, in_list, paragraphs, inline_code);
        }
    }
}

fn inside_inline_code(offset: usize, ranges: &[(usize, usize)]) -> bool {
    ranges
        .iter()
        .any(|&(start, end)| start <= offset && offset < end)
}

fn next_line_ending(bytes: &[u8], from: usize, end: usize) -> usize {
    let mut i = from;
    while i < end && bytes[i] != b'\r' && bytes[i] != b'\n' {
        i += 1;
    }
    i
}

fn add_break_if_soft(bytes: &[u8], start: usize, newline_start: usize, out: &mut String) {
    if newline_start <= start {
        return;
    }

    let mut spaces = 0;
    let mut i = newline_start;
    while i > start && bytes[i - 1] == b' ' {
        spaces += 1;
        i -= 1;
    }

    if spaces >= 2 {
        return;
    }
    if spaces == 1 {
        out.push(' ');
        return;
    }

    let mut backslashes = 0;
    let mut i = newline_start;
    while i > start && bytes[i - 1] == b'\\' {
        backslashes += 1;
        i -= 1;
    }
    if backslashes % 2 == 1 {
        return;
    }

    out.push_str("  ");
}

#[cfg(test)]
mod tests {
    use super::hard_breaks;

    #[test]
    fn ordinary_paragraph_lines_get_hard_breaks() {
        assert_eq!(hard_breaks("one\ntwo\nthree"), "one  \ntwo  \nthree");
    }

    #[test]
    fn existing_hard_breaks_are_preserved() {
        assert_eq!(hard_breaks("one  \ntwo\\\nthree"), "one  \ntwo\\\nthree");
        assert_eq!(hard_breaks("one  \r\ntwo"), "one  \r\ntwo");
    }

    #[test]
    fn a_single_trailing_space_becomes_a_hard_break() {
        assert_eq!(hard_breaks("one \ntwo"), "one  \ntwo");
    }

    #[test]
    fn list_continuation_paragraphs_are_untouched() {
        let text = "- item\n  continuation\n- next";
        assert_eq!(hard_breaks(text), text);

        let text = "- first\n\n  second paragraph\n  still second";
        assert_eq!(hard_breaks(text), text);
    }

    #[test]
    fn a_paragraph_after_a_list_is_still_converted() {
        assert_eq!(
            hard_breaks("- item\n\nprose one\nprose two"),
            "- item\n\nprose one  \nprose two"
        );
    }

    #[test]
    fn fenced_and_indented_code_are_untouched() {
        let fenced = "```\na\nb\n```";
        assert_eq!(hard_breaks(fenced), fenced);

        let indented = "    a\n    b";
        assert_eq!(hard_breaks(indented), indented);
    }

    #[test]
    fn headings_and_setext_headings_are_untouched() {
        assert_eq!(
            hard_breaks("# heading\n\nline one\nline two"),
            "# heading\n\nline one  \nline two"
        );
        assert_eq!(
            hard_breaks("Title\n=====\n\nline one\nline two"),
            "Title\n=====\n\nline one  \nline two"
        );
    }

    #[test]
    fn tables_are_untouched() {
        let table = "| a | b |\n| - | - |\n| c | d |";
        assert_eq!(hard_breaks(table), table);
    }

    #[test]
    fn a_table_followed_by_prose_only_changes_the_prose() {
        assert_eq!(
            hard_breaks("| a | b |\n| - | - |\n| c | d |\n\nprose one\nprose two"),
            "| a | b |\n| - | - |\n| c | d |\n\nprose one  \nprose two"
        );
    }

    #[test]
    fn newlines_inside_inline_code_are_untouched() {
        assert_eq!(
            hard_breaks("before `code\nstill code` after\nnext"),
            "before `code\nstill code` after  \nnext"
        );
        assert_eq!(
            hard_breaks("before `code\r\nstill code` after\r\nnext"),
            "before `code\r\nstill code` after  \r\nnext"
        );
    }

    #[test]
    fn unicode_and_crlf_are_preserved() {
        assert_eq!(
            hard_breaks("héllo\r\nwörld\r\nnext"),
            "héllo  \r\nwörld  \r\nnext"
        );
        assert_eq!(hard_breaks("a😀\nb"), "a😀  \nb");
    }

    #[test]
    fn blank_lines_are_preserved() {
        assert_eq!(
            hard_breaks("one\ntwo\n\nthree\nfour"),
            "one  \ntwo\n\nthree  \nfour"
        );
    }

    #[test]
    fn text_without_line_endings_is_unchanged() {
        assert_eq!(hard_breaks("one line"), "one line");
        assert_eq!(hard_breaks(""), "");
    }
}
