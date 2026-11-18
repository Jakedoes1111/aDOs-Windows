//! What a browser comment is, and the one structured text block the assistant receives.
//! Pure: parsed from `comment.js` output, unit tested without a page.
use serde::Deserialize;
use std::path::PathBuf;

/// The page around a target, as `comment.js` reports it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Picked {
    pub title: String,
    pub url: String,
    /// CSS px to device px.
    pub dpr: f64,
    /// Viewport height in CSS px.
    pub h: f64,
    pub target: Option<Target>,
    /// Page text, only for "Send page".
    pub text: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Target {
    #[serde(rename_all = "camelCase")]
    Element {
        selector: String,
        tag: String,
        #[serde(default)]
        role: String,
        #[serde(default)]
        label: String,
        #[serde(default)]
        text: String,
        #[serde(default)]
        text_cut: bool,
        #[serde(default)]
        html: String,
        #[serde(default)]
        html_cut: bool,
        viewport: [f64; 4],
        page: [f64; 4],
    },
    Text {
        text: String,
        #[serde(default)]
        before: String,
        #[serde(default)]
        after: String,
        #[serde(default)]
        selector: String,
        viewport: [f64; 4],
        page: [f64; 4],
    },
}

impl Target {
    pub fn viewport(&self) -> [f64; 4] {
        match self { Self::Element { viewport, .. } | Self::Text { viewport, .. } => *viewport }
    }
    pub fn page(&self) -> [f64; 4] {
        match self { Self::Element { page, .. } | Self::Text { page, .. } => *page }
    }
}

impl Picked {
    pub fn parse(text: &str) -> Option<Self> {
        if text.trim().is_empty() { return None; }
        serde_json::from_str(text).ok()
    }
    /// What the card says it is commenting on, in a few words.
    pub fn describe(&self) -> String {
        match &self.target {
            None => format!("Page · {}", short(if self.title.is_empty() { &self.url } else { &self.title }, 60)),
            Some(Target::Text { text, .. }) => format!("Selected text · “{}”", short(text, 60)),
            Some(Target::Element { tag, label, text, .. }) => {
                let words = if !label.is_empty() { label } else { text };
                if words.trim().is_empty() { format!("<{tag}>") } else { format!("<{tag}> · “{}”", short(words, 60)) }
            }
        }
    }
}

/// One comment the owner wrote, numbered on its page.
#[derive(Clone, Debug, PartialEq)]
pub struct Comment {
    pub number: usize,
    pub picked: Picked,
    pub comment: String,
    /// The target's cropped screenshot, when the page window could be read.
    pub image: Option<PathBuf>,
}

/// Page text sent with "Send page" is capped here.
const PAGE_TEXT_CAP: usize = 8000;

/// The block the agent gets: one section per comment, every fact on its own labelled line.
pub fn block(comments: &[Comment]) -> String {
    let mut out = String::new();
    let count = comments.len();
    out.push_str(&if count == 1 {
        "The owner commented on a web page in the aDOs Browser.".to_string()
    } else {
        format!("The owner left {count} comments on web pages in the aDOs Browser.")
    });
    out.push_str(" Each target is described exactly below.\n");
    for c in comments {
        out.push_str(&format!("\n## Comment {}\n", c.number));
        line(&mut out, "Page title", &c.picked.title);
        line(&mut out, "URL", &c.picked.url);
        let said = c.comment.trim();
        out.push_str(&format!("Comment: {}\n", if said.is_empty() { "(no words; see the target)" } else { said }));
        match &c.picked.target {
            None => {
                out.push_str("Target: the whole page\n");
                if let Some(text) = c.picked.text.as_deref().filter(|t| !t.trim().is_empty()) {
                    let (text, cut) = cap(text, PAGE_TEXT_CAP);
                    out.push_str(&format!("Page text{}:\n", if cut { " (first 8000 characters)" } else { "" }));
                    out.push_str(&fence("text", text));
                }
            }
            Some(Target::Text { text, before, after, selector, viewport, page }) => {
                out.push_str("Target: selected text\n");
                if !selector.is_empty() { out.push_str(&format!("Inside element: `{selector}`\n")); }
                out.push_str("Selected text (exact):\n");
                out.push_str(&fence("text", text));
                out.push_str("Text just before it:\n");
                out.push_str(&fence("text", before));
                out.push_str("Text just after it:\n");
                out.push_str(&fence("text", after));
                rects(&mut out, page, viewport);
            }
            Some(Target::Element { selector, tag, role, label, text, text_cut, html, html_cut, viewport, page }) => {
                out.push_str("Target: element\n");
                out.push_str(&format!("CSS selector (unique): `{selector}`\n"));
                out.push_str(&format!("Tag: {tag}\n"));
                line(&mut out, "Role", role);
                line(&mut out, "Label", label);
                rects(&mut out, page, viewport);
                if !text.trim().is_empty() {
                    out.push_str(&format!("Text{}:\n", if *text_cut { " (first 1000 characters)" } else { "" }));
                    out.push_str(&fence("text", text));
                }
                out.push_str(&format!("Outer HTML{}:\n", if *html_cut { " (first 4000 characters)" } else { "" }));
                out.push_str(&fence("html", html));
            }
        }
        if let Some(image) = &c.image {
            out.push_str(&format!("Screenshot of the target (PNG): {}\n", image.display()));
        }
    }
    out
}

fn line(out: &mut String, name: &str, value: &str) {
    if !value.trim().is_empty() { out.push_str(&format!("{name}: {}\n", value.trim())); }
}

fn rects(out: &mut String, page: &[f64; 4], viewport: &[f64; 4]) {
    let r = |v: &[f64; 4]| format!("x={} y={} width={} height={}", v[0].round(), v[1].round(), v[2].round(), v[3].round());
    out.push_str(&format!("Rectangle in page px: {}\n", r(page)));
    out.push_str(&format!("Rectangle in viewport px: {}\n", r(viewport)));
}

/// A fenced block whose fence is longer than any backtick run inside it.
fn fence(lang: &str, body: &str) -> String {
    let mut run = 0;
    let mut longest = 0;
    for ch in body.chars() {
        if ch == '`' { run += 1; longest = longest.max(run); } else { run = 0; }
    }
    let ticks = "`".repeat(longest.max(2) + 1);
    format!("{ticks}{lang}\n{}\n{ticks}\n", body.trim_end_matches('\n'))
}

fn cap(text: &str, max: usize) -> (&str, bool) {
    match text.char_indices().nth(max) {
        Some((i, _)) => (&text[..i], true),
        None => (text, false),
    }
}

fn short(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let (head, cut) = cap(&flat, max);
    if cut { format!("{head}…") } else { head.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ELEMENT: &str = r##"{"title":"Fixture","url":"http://127.0.0.1:8765/comment.html","dpr":1,"w":800,"h":600,
        "target":{"kind":"element","selector":"#signup > button:nth-of-type(1)","tag":"button","role":"button",
        "label":"","text":"Sign up","textCut":false,"html":"<button>Sign up</button>","htmlCut":false,
        "viewport":[10,20,80,30],"page":[10,520,80,30]}}"##;
    const TEXT: &str = r#"{"title":"Fixture","url":"http://x/","dpr":2,
        "target":{"kind":"text","text":"quick brown","before":"The ","after":" fox","selector":"p",
        "viewport":[1,2,3,4],"page":[1,102,3,4]}}"#;

    #[test]
    fn parses_both_targets() {
        let el = Picked::parse(ELEMENT).expect("element");
        assert!(matches!(el.target, Some(Target::Element { ref tag, .. }) if tag == "button"));
        assert_eq!(el.describe(), "<button> · “Sign up”");
        let text = Picked::parse(TEXT).expect("text");
        assert_eq!(text.target.as_ref().map(Target::page), Some([1., 102., 3., 4.]));
        assert_eq!(text.describe(), "Selected text · “quick brown”");
        assert_eq!(Picked::parse(""), None);
        assert_eq!(Picked::parse("not json"), None);
    }

    #[test]
    fn element_block_has_every_fact() {
        let c = Comment { number: 1, picked: Picked::parse(ELEMENT).unwrap(), comment: " Make it blue ".into(),
            image: Some(PathBuf::from("/h/cache/comments/1.png")) };
        let b = block(&[c]);
        for want in [
            "The owner commented on a web page", "## Comment 1", "Page title: Fixture",
            "URL: http://127.0.0.1:8765/comment.html", "Comment: Make it blue",
            "CSS selector (unique): `#signup > button:nth-of-type(1)`", "Tag: button", "Role: button",
            "Rectangle in page px: x=10 y=520 width=80 height=30", "Rectangle in viewport px: x=10 y=20 width=80 height=30",
            "```text\nSign up\n```", "```html\n<button>Sign up</button>\n```",
            "Screenshot of the target (PNG): /h/cache/comments/1.png",
        ] {
            assert!(b.contains(want), "missing {want:?} in\n{b}");
        }
        assert!(!b.contains("Label:"), "empty label is left out");
    }

    #[test]
    fn text_block_and_batch() {
        let one = Comment { number: 1, picked: Picked::parse(TEXT).unwrap(), comment: "typo?".into(), image: None };
        let page = Comment { number: 2, picked: Picked { title: "P".into(), url: "u".into(), text: Some("body".into()), ..Default::default() },
            comment: String::new(), image: None };
        let b = block(&[one, page]);
        assert!(b.starts_with("The owner left 2 comments"));
        assert!(b.contains("Target: selected text\nInside element: `p`\nSelected text (exact):\n```text\nquick brown\n```"));
        assert!(b.contains("Text just before it:\n```text\nThe\n```") || b.contains("Text just before it:\n```text\nThe \n```"));
        assert!(b.contains("Text just after it:\n```text\n fox\n```"));
        assert!(b.contains("## Comment 2\nPage title: P\nURL: u\nComment: (no words; see the target)\nTarget: the whole page\nPage text:\n```text\nbody\n```"));
        assert!(!b.contains("Screenshot"));
    }

    #[test]
    fn fence_outgrows_backticks_and_caps_count_chars() {
        assert_eq!(fence("html", "a ```` b"), "`````html\na ```` b\n`````\n");
        assert_eq!(cap("ééé", 2), ("éé", true));
        assert_eq!(short("a  b\nc", 10), "a b c");
        assert_eq!(short("abcdef", 3), "abc…");
    }
}
