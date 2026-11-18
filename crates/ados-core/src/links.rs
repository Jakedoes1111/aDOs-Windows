//! One path for "open this URL in the aDOs Browser". The shell installs the opener
//! (it knows the layouts); every module just calls `open_url`.
use gpui_kit::component::WindowExt;
use gpui_kit::{App, Global, Window};
use std::ops::Range;
use std::rc::Rc;

type Opener = Rc<dyn Fn(&str, &mut Window, &mut App)>;
#[derive(Default)]
struct LinkOpener(Option<Opener>);
impl Global for LinkOpener {}

/// The shell calls this once with the function that shows the Browser and loads the text.
pub fn set_opener(opener: impl Fn(&str, &mut Window, &mut App) + 'static, cx: &mut App) {
    cx.set_global(LinkOpener(Some(Rc::new(opener))));
}

/// Open a URL (or search words, as typed into the address bar) in the aDOs Browser.
pub fn open_url(text: &str, window: &mut Window, cx: &mut App) {
    let text = text.trim();
    if text.is_empty() {
        window.push_notification("There is no link to open", cx);
        return;
    }
    let opener = cx.try_global::<LinkOpener>().and_then(|o| o.0.clone());
    match opener {
        Some(open) => open(text, window, cx),
        None => window.push_notification("The Browser is not available in this build", cx),
    }
}

/// The one explicit way out: hand a web address to the system browser.
pub fn open_in_system_browser(url: &str, window: &mut Window, cx: &mut App) {
    let url = url.trim();
    if url.starts_with("http://") || url.starts_with("https://") {
        cx.open_url(url);
    } else {
        window.push_notification("Only web addresses open in the system browser", cx);
    }
}

fn is_url_char(c: char) -> bool {
    !c.is_whitespace() && !matches!(c, '<' | '>' | '"' | '`' | '\u{0}'..='\u{1f}')
}

/// Byte ranges of the `http(s)://` URLs in `text`. Trailing punctuation and an unmatched
/// closing bracket (as in "(see https://x.org)") are not part of the URL.
pub fn find_urls(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(rel) = text[from..].find("http") {
        let start = from + rel;
        let rest = &text[start..];
        let scheme = if rest.starts_with("https://") { 8 } else if rest.starts_with("http://") { 7 } else { 0 };
        let word_start = text[..start].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        if scheme == 0 || !word_start {
            from = start + 4;
            continue;
        }
        let mut end = start + rest.char_indices().find(|(_, c)| !is_url_char(*c)).map_or(rest.len(), |(i, _)| i);
        loop {
            let url = &text[start..end];
            let Some(last) = url.chars().next_back() else { break };
            let unmatched = |open: char| last == match open { '(' => ')', '[' => ']', _ => '}' }
                && url.matches(open).count() < url.matches(last).count();
            if matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '*')
                || unmatched('(') || unmatched('[') || unmatched('{')
            {
                end -= last.len_utf8();
            } else {
                break;
            }
        }
        if end - start > scheme {
            out.push(start..end);
        }
        from = end.max(start + 4);
    }
    out
}

/// The URL at byte offset `at` (the cursor), else the first URL on the line.
pub fn url_at<'a>(line: &'a str, at: usize) -> Option<&'a str> {
    let urls = find_urls(line);
    urls.iter().find(|r| r.contains(&at) || r.end == at).or(urls.first()).map(|r| &line[r.clone()])
}

// Markdown for page -> note / task.
/// `[title](url)`; brackets in the title are escaped, an empty title shows the address.
pub fn md_link(title: &str, url: &str) -> String {
    let t = title.trim().replace('\n', " ");
    let t = if t.is_empty() { url.to_string() } else { t.replace('[', "\\[").replace(']', "\\]") };
    format!("[{t}]({url})")
}
fn quote(selection: &str) -> String {
    selection.trim().lines().map(|l| format!("> {l}").trim_end().to_string()).collect::<Vec<_>>().join("\n")
}
/// New note: title, URL, then the selection (when there is one) as a quote.
pub fn note_text(title: &str, url: &str, selection: &str) -> String {
    let heading = if title.trim().is_empty() { url } else { title.trim() };
    let mut text = format!("# {}\n\n{url}\n", heading.replace('\n', " "));
    if !selection.trim().is_empty() {
        text.push_str(&format!("\n{}\n", quote(selection)));
    }
    text
}
/// Block appended to the current note: a link line, then the selection as a quote.
pub fn append_text(title: &str, url: &str, selection: &str) -> String {
    let mut text = md_link(title, url);
    if !selection.trim().is_empty() {
        text.push_str(&format!("\n\n{}", quote(selection)));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    fn urls(t: &str) -> Vec<&str> { find_urls(t).into_iter().map(|r| &t[r]).collect() }
    #[test]
    fn finds_and_trims() {
        assert_eq!(urls("see https://x.org/a?b=1."), ["https://x.org/a?b=1"]);
        assert_eq!(urls("(see https://x.org)"), ["https://x.org"]);
        assert_eq!(urls("https://en.wikipedia.org/wiki/Rust_(language)"), ["https://en.wikipedia.org/wiki/Rust_(language)"]);
        assert_eq!(urls("a http://a.b, and <https://c.d/e>"), ["http://a.b", "https://c.d/e"]);
        assert_eq!(urls("[t](https://x.org/p)"), ["https://x.org/p"]);
        assert!(urls("nohttp://x https:// shttp://y").is_empty());
    }
    #[test]
    fn url_at_cursor() {
        let t = "a https://one.org b https://two.org";
        assert_eq!(url_at(t, 25), Some("https://two.org"));
        assert_eq!(url_at(t, 0), Some("https://one.org"));
        assert_eq!(url_at("none", 1), None);
    }
}

/// Plain text whose web addresses are links: blue and clickable in place, plus one
/// keyboard- and hint-reachable button per address under the text.
pub fn linked_text(id: &str, text: &str, cx: &App) -> gpui_kit::AnyElement {
    use gpui_kit::component::{button::{Button, ButtonVariants}, v_flex, ActiveTheme, Sizable};
    use gpui_kit::assets::IconName;
    use gpui_kit::prelude::*;
    use gpui_kit::*;
    let text = text.trim().to_string();
    let ranges = find_urls(&text);
    if ranges.is_empty() {
        return div().whitespace_normal().child(text).into_any_element();
    }
    let blue = rgb(crate::brand::BLUE);
    let style = HighlightStyle {
        color: Some(blue.into()),
        underline: Some(UnderlineStyle { thickness: px(1.), color: Some(blue.into()), wavy: false }),
        ..Default::default()
    };
    let urls: Vec<String> = ranges.iter().map(|r| text[r.clone()].to_string()).collect();
    let styled = StyledText::new(text.clone()).with_highlights(ranges.iter().map(|r| (r.clone(), style)));
    let clicked = urls.clone();
    let inline = InteractiveText::new(SharedString::from(format!("{id}-text")), styled)
        .on_click(ranges, move |i, window, cx| open_url(&clicked[i], window, cx));
    let muted = cx.theme().muted_foreground;
    v_flex().gap_1().child(div().whitespace_normal().child(inline))
        .children(urls.into_iter().enumerate().map(|(i, url)| {
            let target = url.clone();
            crate::hints::marked(Button::new(SharedString::from(format!("{id}-link-{i}")))
                .icon(IconName::ExternalLink).ghost().small().h(px(32.)).text_color(muted)
                .label(if url.chars().count() > 60 { format!("{}…", url.chars().take(59).collect::<String>()) } else { url })
                .tooltip("Open in the Browser")
                .on_click(move |_, window, cx| open_url(&target, window, cx)))
        }))
        .into_any_element()
}

#[cfg(test)]
mod page_md_tests {
    use super::*;
    #[test]
    fn markdown_pieces() {
        assert_eq!(md_link("A [b]", "https://x.org"), "[A \\[b\\]](https://x.org)");
        assert_eq!(md_link("", "https://x.org"), "[https://x.org](https://x.org)");
        assert_eq!(note_text("T", "https://x.org", ""), "# T\n\nhttps://x.org\n");
        assert_eq!(note_text("T", "https://x.org", "a\nb"), "# T\n\nhttps://x.org\n\n> a\n> b\n");
        assert_eq!(append_text("T", "https://x.org", " q "), "[T](https://x.org)\n\n> q");
        assert_eq!(append_text("T", "https://x.org", ""), "[T](https://x.org)");
    }
}
