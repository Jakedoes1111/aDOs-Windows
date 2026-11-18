//! What agents can do with the Browser (the MCP tools `browser_*` call these): list tabs,
//! open a URL, read the page as text with refs, click, type, read the selection, screenshot.
//! Every page script goes through the one `CommandMsg::Eval` path, so it works on Servo
//! (WebDriver) and Chromium (CDP) alike; only the screenshot uses the engine's own capture.
use super::*;
use serde_json::{json, Value};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

/// Longest `browser_read` answer in characters; the rest is behind the `more` cursor.
pub const READ_CAP: usize = 6000;
/// Longest selection returned or published.
const SELECTION_CAP: usize = 20_000;

/// A tool call waiting for the page.
pub type Pending = Pin<Box<dyn Future<Output = Result<Value, String>>>>;

#[derive(Debug)]
pub enum Op {
    /// List the page tabs; `select` shows that tab first.
    Tabs { select: Option<usize> },
    Open { url: String, new_tab: bool },
    Read { selector: Option<String>, cursor: usize },
    Click { target: Target },
    Type { target: Target, text: String, submit: bool },
    Selection,
    Screenshot,
}

/// An element: a ref from `browser_read` (`e12`) or a CSS selector.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Ref(String),
    Selector(String),
}

/// `ref` wins over `selector` when both are given; one of them is needed.
pub fn target(reference: Option<&str>, selector: Option<&str>) -> Result<Target, String> {
    match (reference.map(str::trim).filter(|s| !s.is_empty()), selector.map(str::trim).filter(|s| !s.is_empty())) {
        (Some(r), _) => {
            let ok = r.strip_prefix('e').is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
            if ok { Ok(Target::Ref(r.to_string())) } else { Err(format!("\"{r}\" is not a ref. Refs look like e12 and come from browser_read.")) }
        }
        (None, Some(s)) => Ok(Target::Selector(s.to_string())),
        (None, None) => Err("Give a \"ref\" from browser_read or a CSS \"selector\".".into()),
    }
}

/// Only http(s) pages. `file:`, `data:`, `javascript:`, `about:` and the rest are refused.
/// The fixture host (`http://127.0.0.1:8765/`) is plain http, so it passes. A bare
/// `example.com` gets `https://`.
pub fn check_url(url: &str) -> Result<String, String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("The URL is empty.".into());
    }
    let scheme = url.split_once("://").map(|(s, _)| s.to_ascii_lowercase());
    match scheme.as_deref() {
        Some("http" | "https") => {}
        Some(other) => return Err(format!("Only http and https pages can be opened, not {other}:.")),
        None if url.contains(':') && !url.split(':').nth(1).is_some_and(|p| p.starts_with(|c: char| c.is_ascii_digit())) => {
            let s = url.split(':').next().unwrap_or("");
            return Err(format!("Only http and https pages can be opened, not {s}:."));
        }
        None => return Ok(format!("https://{url}")),
    }
    let host = url.split_once("://").map(|(_, rest)| rest).unwrap_or("");
    if host.is_empty() || host.starts_with('/') {
        return Err("The URL has no host.".into());
    }
    Ok(url.to_string())
}

/// One `browser_read` item as a line of text.
fn line(item: &Value) -> String {
    let s = |k: &str| item[k].as_str().unwrap_or("");
    let r = s("r");
    match s("k") {
        "a" => format!("[{r}] link \"{}\" -> {}", s("t"), s("h")),
        "button" => format!("[{r}] button \"{}\"", s("t")),
        "field" => {
            let mut out = format!("[{r}] field({}) \"{}\"", s("y"), s("t"));
            if !s("v").is_empty() {
                out.push_str(&format!(" value=\"{}\"", s("v")));
            }
            out
        }
        h => format!("{} {}{}", "#".repeat(h.trim_start_matches('h').parse().unwrap_or(1)), s("t"), if r.is_empty() { String::new() } else { format!(" [{r}]") }),
    }
}

/// The items from `cursor` as text, stopping before `cap` characters (always at least one
/// item). Returns the text and the cursor of the next item, if any are left.
pub fn shape(items: &[Value], cursor: usize, cap: usize) -> (String, Option<usize>) {
    let mut out = String::new();
    let mut next = cursor;
    while let Some(item) = items.get(next) {
        let l = line(item);
        if next > cursor && out.chars().count() + l.chars().count() + 1 > cap {
            break;
        }
        out.push_str(&l);
        out.push('\n');
        next += 1;
    }
    (out, (next < items.len()).then_some(next))
}

const COMMON: &str = "var A=window.__adosAgent||(window.__adosAgent={n:0,els:{},ids:new WeakMap()});\
function ref(el){var r=A.ids.get(el);if(!r){r='e'+(++A.n);A.ids.set(el,r);A.els[r]=el;}return r;}\
function txt(el){return (el.innerText||el.textContent||'').replace(/\\s+/g,' ').trim().slice(0,200);}\
function find(t){if(t.ref){var e=A.els[t.ref];\
if(!e)return {error:'Ref '+t.ref+' is unknown: the page changed or was reloaded. Run browser_read again.'};\
if(!e.isConnected)return {error:'Ref '+t.ref+' is stale: that element is no longer on the page. Run browser_read again.'};return {el:e};}\
var el;try{el=document.querySelector(t.sel);}catch(x){return {error:'\"'+t.sel+'\" is not a valid CSS selector.'};}\
return el?{el:el}:{error:'No element matches \"'+t.sel+'\".'};}";

fn wrap(body: &str) -> String {
    format!("(function(){{try{{{COMMON}{body}}}catch(x){{return JSON.stringify({{error:String(x)}});}}}})()")
}

fn target_json(t: &Target) -> Value {
    match t {
        Target::Ref(r) => json!({ "ref": r }),
        Target::Selector(s) => json!({ "sel": s }),
    }
}

pub fn read_script(selector: Option<&str>) -> String {
    let sel = serde_json::to_string(&selector).unwrap_or_else(|_| "null".into());
    wrap(&format!(
        "var S={sel};var root;try{{root=S?document.querySelector(S):document.body;}}catch(x){{return JSON.stringify({{error:'\"'+S+'\" is not a valid CSS selector.'}});}}\
if(!root)return JSON.stringify({{error:'No element matches \"'+S+'\".'}});\
var q='h1,h2,h3,h4,h5,h6,a[href],input,textarea,select,button,[role=button]';\
var nodes=Array.prototype.slice.call(root.querySelectorAll(q));if(root.matches&&root.matches(q))nodes.unshift(root);\
var items=[];for(var i=0;i<nodes.length&&items.length<3000;i++){{var el=nodes[i];\
if(!el.getClientRects().length||getComputedStyle(el).visibility==='hidden')continue;\
var tag=el.tagName.toLowerCase();var y=(el.getAttribute('type')||'text').toLowerCase();\
if(/^h[1-6]$/.test(tag)){{var t=txt(el);if(t)items.push({{k:tag,t:t,r:ref(el)}});}}\
else if(tag==='a')items.push({{k:'a',t:txt(el)||el.getAttribute('aria-label')||'',h:el.href,r:ref(el)}});\
else if(tag==='button'||el.getAttribute('role')==='button'||(tag==='input'&&/^(submit|button|reset|image)$/.test(y)))items.push({{k:'button',t:txt(el)||el.value||el.getAttribute('aria-label')||'',r:ref(el)}});\
else if(tag==='input'&&y==='hidden')continue;\
else{{var lab=(el.labels&&el.labels[0])?txt(el.labels[0]):'';\
items.push({{k:'field',y:tag==='select'?'select':tag==='textarea'?'textarea':y,t:lab||el.getAttribute('aria-label')||el.placeholder||el.name||el.id||'',v:y==='password'?'':String(el.value||'').slice(0,200),r:ref(el)}});}}}}\
return JSON.stringify({{title:document.title,url:location.href,items:items}});"
    ))
}

pub fn click_script(t: &Target) -> String {
    wrap(&format!(
        "var f=find({});if(f.error)return JSON.stringify(f);var el=f.el;\
try{{el.scrollIntoView({{block:'center'}});}}catch(x){{}}el.click();\
return JSON.stringify({{clicked:ref(el),text:txt(el),url:location.href}});",
        target_json(t)
    ))
}

pub fn type_script(t: &Target, text: &str, submit: bool) -> String {
    let text = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into());
    wrap(&format!(
        "var f=find({});if(f.error)return JSON.stringify(f);var el=f.el;var tag=el.tagName.toLowerCase();\
if(tag!=='input'&&tag!=='textarea')return JSON.stringify({{error:'That element is a '+tag+', not a text field.'}});\
try{{el.scrollIntoView({{block:'center'}});}}catch(x){{}}el.focus();\
var d=Object.getOwnPropertyDescriptor(tag==='textarea'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype,'value');\
if(d&&d.set)d.set.call(el,{text});else el.value={text};\
el.dispatchEvent(new Event('input',{{bubbles:true}}));el.dispatchEvent(new Event('change',{{bubbles:true}}));\
var sent=false;if({submit}){{var ev=function(n){{return new KeyboardEvent(n,{{key:'Enter',code:'Enter',keyCode:13,which:13,bubbles:true,cancelable:true}});}};\
var go=el.dispatchEvent(ev('keydown'));el.dispatchEvent(ev('keypress'));el.dispatchEvent(ev('keyup'));\
if(go&&el.form){{if(el.form.requestSubmit)el.form.requestSubmit();else el.form.submit();}}sent=true;}}\
return JSON.stringify({{typed:ref(el),value:tag==='input'&&el.type==='password'?'':el.value,submitted:sent,url:location.href}});",
        target_json(t),
        submit = submit,
    ))
}

pub fn selection_script() -> String {
    wrap(&format!(
        "var s=getSelection();var t=s?String(s):'';\
if(!t.trim())return JSON.stringify({{text:'',url:location.href,title:document.title}});\
var n=s.anchorNode;var el=n&&(n.nodeType===1?n:n.parentElement);var head='';\
if(el){{var hs=document.querySelectorAll('h1,h2,h3,h4,h5,h6');for(var i=0;i<hs.length;i++){{var h=hs[i];\
if(h===el||h.contains(el)||(h.compareDocumentPosition(el)&Node.DOCUMENT_POSITION_FOLLOWING))head=txt(h);}}}}\
return JSON.stringify({{text:t.slice(0,{SELECTION_CAP}),heading:head,url:location.href,title:document.title}});"
    ))
}

/// The `text` field of the selection script's answer (empty on any failure).
pub(crate) fn selection_text(answer: &str) -> String {
    parse(answer).ok().and_then(|v| v["text"].as_str().map(str::to_string)).unwrap_or_default()
}

/// The page's answer as JSON, or the page's own error sentence.
fn parse(text: &str) -> Result<Value, String> {
    let v: Value = serde_json::from_str(text).map_err(|_| "The page gave an answer aDOs could not read.".to_string())?;
    match v["error"].as_str() {
        Some(e) => Err(e.to_string()),
        None => Ok(v),
    }
}

/// `ADOS_HOME/cache/browser-<unix seconds>.png`, decoded from the engine's base64.
fn save_png(data: &str, dir: PathBuf, stamp: u64) -> Result<PathBuf, String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.trim())
        .map_err(|_| "The engine's screenshot could not be decoded.".to_string())?;
    if !bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Err("The engine did not return a PNG.".into());
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}.", dir.display()))?;
    let path = dir.join(format!("browser-{stamp}.png"));
    std::fs::write(&path, bytes).map_err(|e| format!("Could not save the screenshot: {e}."))?;
    Ok(path)
}

/// The Browser panel, if one exists.
pub fn panel(cx: &App) -> Option<Entity<BrowserPanel>> {
    cx.try_global::<Target0>()?.0.as_ref().and_then(WeakEntity::upgrade)
}
type Target0 = super::Target;

/// Is the Browser in the current layout and on screen?
pub fn in_view(cx: &App) -> bool {
    panel(cx).is_some_and(|p| p.read(cx).active)
}

impl BrowserPanel {
    pub(crate) fn run(&self, script: String) -> Result<async_channel::Receiver<Result<String, String>>, String> {
        let runtime = self.runtime.as_ref().ok_or_else(|| self.error.clone().unwrap_or_else(|| "The Browser has not started yet.".into()))?;
        let (tx, rx) = async_channel::bounded(1);
        runtime.command(CommandMsg::Eval(script, tx));
        Ok(rx)
    }
    /// Look at the owner's selection a moment after they press in the page, and once more
    /// later (a drag ends after the press). One-shot tasks, no loop.
    pub(super) fn refresh_selection_soon(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for ms in [400u64, 1600] {
            let generation = self.generation;
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(ms)).await;
                let rx = this.update(cx, |this, _| (this.generation == generation).then(|| this.run(selection_script()).ok()).flatten()).ok().flatten();
                let Some(rx) = rx else { return };
                let Ok(Ok(text)) = rx.recv().await else { return };
                let selected = parse(&text).ok().and_then(|v| v["text"].as_str().map(str::to_string)).unwrap_or_default();
                let _ = this.update(cx, |this, cx| this.set_selection(selected, cx));
            })
            .detach();
        }
    }
    /// Put the selection into the Browser's context snapshot.
    pub(super) fn set_selection(&mut self, selected: String, cx: &mut Context<Self>) {
        if self.selection == selected {
            return;
        }
        self.selection = selected.clone();
        if let Some(mut snapshot) = ContextHub::get(ids::BROWSER, cx) {
            snapshot.selection = (!selected.is_empty()).then(|| selected.into());
            ContextHub::publish(snapshot, cx);
        }
    }
}

/// Start an operation. The synchronous part (reaching the panel, sending the command) runs
/// now on the GPUI thread; the answer comes from the returned future.
pub fn start(op: Op, window: &mut Window, cx: &mut App) -> Result<Pending, String> {
    let panel = panel(cx).ok_or_else(|| "The Browser module is not open. Open it from the rail or a layout that has it.".to_string())?;
    match op {
        Op::Tabs { select } => {
            if let Some(index) = select {
                let count = panel.read(cx).tabs.list.len();
                if index >= count { return Err(format!("There is no tab {index}; the Browser has {count}.")); }
                panel.update(cx, |p, cx| p.select_tab(index, window, cx));
            }
            let p = panel.read(cx);
            let tabs: Vec<Value> = p.tabs.list.iter().enumerate().map(|(index, t)| {
                json!({ "index": index, "title": t.title, "url": t.url, "active": index == p.tabs.selected })
            }).collect();
            let active = tabs[p.tabs.selected].clone();
            let out = json!({ "tabs": tabs, "active": active, "engine": p.state.engine.label(), "in_view": p.active });
            Ok(Box::pin(async move { Ok(out) }))
        }
        Op::Open { url, new_tab } => {
            let url = check_url(&url)?;
            let in_view = panel.read(cx).active;
            panel.update(cx, |p, cx| {
                if new_tab {
                    p.open_tab(url.clone(), window, cx);
                } else {
                    p.state.url = url.clone();
                    p.tabs.current_mut().url = url.clone();
                    p.persist_tabs();
                    p.persist();
                    p.error = None;
                    p.send(CommandMsg::Navigate(url.clone()));
                }
                cx.notify();
            });
            let out = json!({ "opened": url, "in_view": in_view, "new_tab": new_tab, "note": "" });
            Ok(Box::pin(async move { Ok(out) }))
        }
        Op::Read { selector, cursor } => {
            let rx = panel.read(cx).run(read_script(selector.as_deref()))?;
            Ok(Box::pin(async move {
                let v = parse(&rx.recv().await.map_err(|_| "The page did not answer.".to_string())??)?;
                let items = v["items"].as_array().cloned().unwrap_or_default();
                if cursor > items.len() {
                    return Err(format!("The cursor {cursor} is past the end; the page has {} items.", items.len()));
                }
                let (text, more) = shape(&items, cursor, READ_CAP);
                Ok(json!({ "title": v["title"], "url": v["url"], "total": items.len(), "text": text, "more": more }))
            }))
        }
        Op::Click { target } => {
            let rx = panel.read(cx).run(click_script(&target))?;
            Ok(Box::pin(async move { parse(&rx.recv().await.map_err(|_| "The page did not answer.".to_string())??) }))
        }
        Op::Type { target, text, submit } => {
            let rx = panel.read(cx).run(type_script(&target, &text, submit))?;
            Ok(Box::pin(async move { parse(&rx.recv().await.map_err(|_| "The page did not answer.".to_string())??) }))
        }
        Op::Selection => {
            let rx = panel.read(cx).run(selection_script())?;
            Ok(Box::pin(async move {
                let v = parse(&rx.recv().await.map_err(|_| "The page did not answer.".to_string())??)?;
                if v["text"].as_str().unwrap_or("").is_empty() {
                    return Ok(json!({ "text": "", "url": v["url"], "note": "Nothing is selected in the page." }));
                }
                Ok(v)
            }))
        }
        Op::Screenshot => {
            let p = panel.read(cx);
            let runtime = p.runtime.clone().ok_or_else(|| "The Browser has not started yet.".to_string())?;
            let (tx, rx) = async_channel::bounded(1);
            runtime.command(CommandMsg::Screenshot(tx));
            let dir = ados_core::paths::home().join("cache");
            let engine = p.state.engine.label();
            Ok(Box::pin(async move {
                let data = rx
                    .recv()
                    .await
                    .map_err(|_| "The page did not answer.".to_string())?
                    .map_err(|e| format!("{engine} could not take a screenshot: {e}."))?;
                let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
                let path = save_png(&data, dir, stamp)?;
                Ok(json!({ "path": path.to_string_lossy(), "engine": engine }))
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{check_url, click_script, line, parse, read_script, save_png, shape, target, type_script, Target};
    use serde_json::{json, Value};

    #[test]
    fn urls_are_http_only() {
        assert_eq!(check_url("https://example.com/a"), Ok("https://example.com/a".into()));
        assert_eq!(check_url("http://127.0.0.1:8765/index.html"), Ok("http://127.0.0.1:8765/index.html".into()));
        assert_eq!(check_url("example.com"), Ok("https://example.com".into()));
        assert_eq!(check_url("localhost:3000"), Ok("https://localhost:3000".into()));
        assert!(check_url("file:///etc/passwd").unwrap_err().contains("file:"));
        assert!(check_url("javascript:alert(1)").is_err());
        assert!(check_url("data:text/html,hi").is_err());
        assert!(check_url("about:blank").is_err());
        assert!(check_url("ftp://x.org").is_err());
        assert!(check_url("  ").is_err());
        assert!(check_url("https://").is_err());
    }

    #[test]
    fn targets_resolve() {
        assert_eq!(target(Some("e12"), None), Ok(Target::Ref("e12".into())));
        assert_eq!(target(Some("e12"), Some("#x")), Ok(Target::Ref("e12".into())));
        assert_eq!(target(None, Some("#q")), Ok(Target::Selector("#q".into())));
        assert!(target(Some("12"), None).is_err());
        assert!(target(Some("e"), None).is_err());
        assert!(target(None, None).is_err());
        assert!(target(Some(" "), Some(" ")).is_err());
    }

    fn items() -> Vec<Value> {
        vec![
            json!({"k":"h1","t":"Title","r":"e1"}),
            json!({"k":"a","t":"Next","h":"http://x/second.html","r":"e2"}),
            json!({"k":"field","y":"text","t":"Search field","v":"","r":"e3"}),
            json!({"k":"field","y":"text","t":"Name","v":"Jo","r":"e4"}),
            json!({"k":"button","t":"Submit","r":"e5"}),
        ]
    }

    #[test]
    fn text_shapes_each_kind() {
        let (text, more) = shape(&items(), 0, 6000);
        assert_eq!(more, None);
        assert_eq!(
            text,
            "# Title [e1]\n[e2] link \"Next\" -> http://x/second.html\n[e3] field(text) \"Search field\"\n[e4] field(text) \"Name\" value=\"Jo\"\n[e5] button \"Submit\"\n"
        );
        assert!(line(&json!({"k":"h3","t":"Deep","r":"e9"})).starts_with("### Deep"));
    }

    #[test]
    fn text_is_capped_with_a_cursor() {
        let (first, more) = shape(&items(), 0, 60);
        assert_eq!(more, Some(2));
        assert_eq!(first.lines().count(), 2);
        let (rest, more) = shape(&items(), more.unwrap_or(0), 6000);
        assert_eq!(more, None);
        assert_eq!(rest.lines().count(), 3);
        // A single item longer than the cap is still returned.
        let (one, more) = shape(&items(), 0, 1);
        assert_eq!(one.lines().count(), 1);
        assert_eq!(more, Some(1));
        assert_eq!(shape(&[], 0, 10), (String::new(), None));
    }

    #[test]
    fn scripts_embed_arguments_safely() {
        let s = type_script(&Target::Selector("input[name=\"q\"]".into()), "he said \"hi\"\n</script>", true);
        assert!(s.contains(r#""sel":"input[name=\"q\"]""#));
        assert!(s.contains(r#""he said \"hi\"\n</script>""#));
        assert!(s.contains("if(true)"));
        assert!(read_script(Some("a[href]")).contains("var S=\"a[href]\";"));
        assert!(read_script(None).contains("var S=null;"));
        assert!(click_script(&Target::Ref("e3".into())).contains(r#""ref":"e3""#));
    }

    #[test]
    fn page_errors_become_sentences() {
        assert_eq!(parse(r#"{"error":"Ref e3 is stale"}"#), Err("Ref e3 is stale".into()));
        assert!(parse("not json").is_err());
        assert!(parse(r#"{"clicked":"e1"}"#).is_ok());
    }

    #[test]
    fn screenshots_are_saved_as_png() {
        let dir = std::env::temp_dir().join(format!("ados-shot-test-{}", std::process::id()));
        // 1x1 PNG.
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
        let path = save_png(png, dir.clone(), 7).expect("saved");
        assert!(path.ends_with("browser-7.png"));
        assert!(std::fs::read(&path).expect("read").starts_with(&[0x89, b'P']));
        assert!(save_png("aGVsbG8=", dir.clone(), 8).is_err());
        assert!(save_png("!!!", dir.clone(), 9).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
