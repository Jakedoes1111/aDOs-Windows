//! Minimal CDP and WebDriver clients, deliberately independent of the UI toolkit.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{
    net::TcpStream,
    time::{Duration, Instant},
};
use tungstenite::{Message, WebSocket};

#[derive(Clone, Debug, Default)]
pub struct Page {
    pub title: String,
    pub url: String,
    pub text: String,
    pub loading: bool,
    /// `history.length` of the tab: entries behind and ahead, plus the current one.
    pub history: usize,
}
impl Page {
    fn from_value(v: &Value) -> Self {
        Self {
            title: v["title"].as_str().unwrap_or_default().into(),
            url: v["url"].as_str().unwrap_or_default().into(),
            text: v["text"]
                .as_str()
                .unwrap_or_default()
                .chars()
                .take(60_000)
                .collect(),
            loading: v["loading"].as_bool().unwrap_or(false),
            history: v["history"].as_u64().unwrap_or(1).max(1) as usize,
        }
    }
}
const SNAPSHOT: &str = "JSON.stringify({title:document.title,url:location.href,text:(document.body?.innerText||'').slice(0,120000),loading:document.readyState!=='complete',history:history.length})";

/// Reply channel for one script evaluation: the result as text (JSON for non-strings).
pub type Reply = async_channel::Sender<Result<String, String>>;
fn reply_text(v: &Value) -> String {
    v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())
}

pub struct Cdp {
    pub socket: WebSocket<TcpStream>,
    /// The engine target (one page) this connection drives.
    pub target: String,
    /// A page target was created or destroyed: the driver lists the targets again.
    pub targets_changed: bool,
    /// Messages read while `call` waited for its reply; `drain` handles them first.
    backlog: std::collections::VecDeque<Value>,
    seq: u64,
    evals: Vec<(u64, Reply)>,
    /// Pending `Page.captureScreenshot` calls (base64 PNG).
    shots: Vec<(u64, Reply)>,
    snapshots: Vec<u64>,
    history: Vec<(u64, i32)>,
    loading: bool,
    last_snapshot: Instant,
    snapshot_again: bool,
}
impl Cdp {
    /// Page targets (id, address) in the engine's order.
    pub fn targets(port: u16, agent: &ureq::Agent) -> Result<Vec<(String, String)>> {
        let targets: Value = agent
            .get(format!("http://127.0.0.1:{port}/json/list"))
            .call()?
            .body_mut()
            .read_json()?;
        Ok(targets.as_array().into_iter().flatten()
            .filter(|v| v["type"] == "page")
            .filter_map(|v| Some((v["id"].as_str()?.to_string(), v["url"].as_str().unwrap_or_default().to_string())))
            .collect())
    }
    /// Connect to the page target `target`, or to the first page when `None`.
    pub fn connect(port: u16, agent: &ureq::Agent, target: Option<&str>) -> Result<Self> {
        let targets: Value = agent
            .get(format!("http://127.0.0.1:{port}/json/list"))
            .call()?
            .body_mut()
            .read_json()?;
        let page = targets
            .as_array()
            .and_then(|a| a.iter().find(|v| v["type"] == "page" && target.is_none_or(|t| v["id"] == t)))
            .context("Browser has no debuggable page yet")?;
        let id = page["id"].as_str().unwrap_or_default().to_string();
        let ws = page["webSocketDebuggerUrl"].as_str().context("Browser has no debuggable page yet")?;
        let uri: tungstenite::http::Uri = ws.parse()?;
        let stream = TcpStream::connect(("127.0.0.1", port))?;
        stream.set_read_timeout(Some(Duration::from_secs(3)))?;
        stream.set_write_timeout(Some(Duration::from_secs(3)))?;
        let (socket, _) =
            tungstenite::client(uri, stream).map_err(|e| anyhow::anyhow!("CDP handshake: {e}"))?;
        socket.get_ref().set_nonblocking(true)?;
        let mut cdp = Self {
            socket,
            target: id,
            targets_changed: false,
            backlog: Default::default(),
            seq: 0,
            evals: vec![],
            shots: vec![],
            snapshots: vec![],
            history: vec![],
            loading: false,
            last_snapshot: Instant::now() - Duration::from_secs(3),
            snapshot_again: false,
        };
        cdp.send("Page.enable", json!({}))?;
        cdp.send("Runtime.enable", json!({}))?;
        cdp.send("Target.setDiscoverTargets", json!({"discover":true}))?;
        cdp.snapshot()?;
        Ok(cdp)
    }
    fn send(&mut self, method: &str, params: Value) -> Result<u64> {
        self.seq += 1;
        self.socket.send(Message::Text(
            json!({"id":self.seq,"method":method,"params":params})
                .to_string()
                .into(),
        ))?;
        Ok(self.seq)
    }
    /// Send `method` and wait (at most 3 s) for its result; other messages wait in the backlog.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.send(method, params)?;
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match self.socket.read() {
                Ok(Message::Text(text)) => {
                    let v: Value = serde_json::from_str(&text)?;
                    if v["id"].as_u64() == Some(id) {
                        if let Some(error) = v["error"]["message"].as_str() { bail!("{method}: {error}"); }
                        return Ok(v["result"].clone());
                    }
                    self.backlog.push_back(v);
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline { bail!("{method}: the engine did not answer"); }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
    /// The engine window that shows this page (several pages can share one).
    pub fn window_id(&mut self) -> Result<String> {
        let v = self.call("Browser.getWindowForTarget", json!({}))?;
        Ok(v["windowId"].as_i64().context("Chromium returned no window for the page")?.to_string())
    }
    /// The page's viewport height in device pixels.
    pub fn viewport_height(&mut self) -> Result<u32> {
        let v = self.call("Runtime.evaluate", json!({"expression":"innerHeight * devicePixelRatio","returnByValue":true}))?;
        let height = v["result"]["value"].as_f64().context("Chromium returned no viewport height")?;
        if !height.is_finite() || height < 1. { bail!("Chromium's viewport is not ready"); }
        Ok(height.round() as u32)
    }
    /// Bring this page to the front of its engine window.
    pub fn activate(&mut self) -> Result<()> {
        let target = self.target.clone();
        self.send("Target.activateTarget", json!({"targetId":target}))?;
        Ok(())
    }
    /// A new page in its own engine window; its target id.
    pub fn create(&mut self, url: &str) -> Result<String> {
        let v = self.call("Target.createTarget", json!({"url":url,"newWindow":true}))?;
        Ok(v["targetId"].as_str().context("Chromium returned no new page")?.to_string())
    }
    pub fn close_target(&mut self) -> Result<()> {
        let target = self.target.clone();
        self.send("Target.closeTarget", json!({"targetId":target}))?;
        Ok(())
    }
    /// Run `source` in every new document, and in the current one now.
    pub fn add_script(&mut self, source: &str) -> Result<()> {
        self.send("Page.addScriptToEvaluateOnNewDocument", json!({"source":source}))?;
        self.send("Runtime.evaluate", json!({"expression":source}))?;
        Ok(())
    }
    pub fn eval(&mut self, expression: &str, reply: Reply) -> Result<()> {
        let id = self.send("Runtime.evaluate", json!({"expression":expression, "returnByValue":true}))?;
        self.evals.push((id, reply));
        Ok(())
    }
    pub fn screenshot(&mut self, reply: Reply) -> Result<()> {
        let id = self.send("Page.captureScreenshot", json!({"format":"png"}))?;
        self.shots.push((id, reply));
        Ok(())
    }
    pub fn navigate(&mut self, url: &str) -> Result<()> {
        self.send("Page.navigate", json!({"url":url}))?;
        Ok(())
    }
    pub fn reload(&mut self) -> Result<()> {
        self.send("Page.reload", json!({}))?;
        Ok(())
    }
    pub fn history(&mut self, delta: i32) -> Result<()> {
        let id = self.send("Page.getNavigationHistory", json!({}))?;
        self.history.push((id, delta));
        Ok(())
    }
    pub fn snapshot(&mut self) -> Result<()> {
        if !self.snapshots.is_empty() {
            self.snapshot_again = true;
            return Ok(());
        }
        let id = self.send(
            "Runtime.evaluate",
            json!({"expression":SNAPSHOT, "returnByValue":true}),
        )?;
        self.snapshots.push(id);
        self.last_snapshot = Instant::now();
        Ok(())
    }
    pub fn loading_tick(&mut self) -> Result<()> {
        if self.loading && self.last_snapshot.elapsed() >= Duration::from_secs(3) {
            self.snapshot()?;
        }
        Ok(())
    }
    pub fn timeout(&self) -> Option<Duration> {
        (self.loading && self.snapshots.is_empty())
            .then(|| Duration::from_secs(3).saturating_sub(self.last_snapshot.elapsed()))
    }
    pub fn drain(&mut self) -> Result<Vec<Page>> {
        let mut pages = vec![];
        loop {
            let v: Value = match self.backlog.pop_front() {
                Some(v) => v,
                None => match self.socket.read() {
                    Ok(Message::Text(text)) => serde_json::from_str(&text)?,
                    Ok(_) => continue,
                    Err(tungstenite::Error::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => return Err(e.into()),
                },
            };
            let id = v["id"].as_u64().unwrap_or(0);
            if let Some(i) = self.evals.iter().position(|(p, _)| *p == id) {
                let (_, reply) = self.evals.remove(i);
                let result = match v["result"]["exceptionDetails"]["text"].as_str() {
                    Some(error) => Err(error.to_string()),
                    None => Ok(reply_text(&v["result"]["result"]["value"])),
                };
                let _ = reply.try_send(result);
            }
            if let Some(i) = self.shots.iter().position(|(p, _)| *p == id) {
                let (_, reply) = self.shots.remove(i);
                let _ = reply.try_send(match v["result"]["data"].as_str() {
                    Some(data) => Ok(data.to_string()),
                    None => Err(v["error"]["message"].as_str().unwrap_or("the engine returned no image").to_string()),
                });
            }
            if let Some(i) = self.snapshots.iter().position(|p| *p == id) {
                self.snapshots.remove(i);
                if let Some(text) = v["result"]["result"]["value"].as_str() {
                    if let Ok(page) = serde_json::from_str::<Value>(text) {
                        pages.push(Page::from_value(&page));
                    }
                }
            }
            if let Some(i) = self.history.iter().position(|(p, _)| *p == id) {
                let (_, delta) = self.history.remove(i);
                let index = v["result"]["currentIndex"].as_i64().unwrap_or(0) + i64::from(delta);
                if index >= 0 {
                    if let Some(entry) = v["result"]["entries"]
                        .get(index as usize)
                        .and_then(|e| e["id"].as_i64())
                    {
                        self.send("Page.navigateToHistoryEntry", json!({"entryId":entry}))?;
                    }
                }
            }
            match v["method"].as_str().unwrap_or_default() {
                "Page.frameStartedLoading" => {
                    self.loading = true;
                }
                "Page.frameNavigated"
                | "Page.navigatedWithinDocument"
                | "Page.domContentEventFired"
                | "Target.targetInfoChanged" => {
                    if !self.loading || self.last_snapshot.elapsed() >= Duration::from_secs(3) {
                        self.snapshot()?;
                    }
                }
                "Target.targetCreated" | "Target.targetDestroyed" => self.targets_changed = true,
                "Page.loadEventFired" | "Page.frameStoppedLoading" => {
                    self.loading = false;
                    self.snapshot()?;
                }
                _ => {}
            }
        }
        if self.snapshot_again && self.snapshots.is_empty() {
            self.snapshot_again = false;
            self.snapshot()?;
        }
        Ok(pages)
    }
}

pub struct WebDriver {
    agent: ureq::Agent,
    base: String,
    /// The engine tab the session drove first: aDOs's first page.
    pub home: String,
}
impl WebDriver {
    pub fn connect(port: u16, agent: &ureq::Agent, marker: &str) -> Result<Self> {
        let v: Value = agent
            .post(format!("http://127.0.0.1:{port}/session"))
            .send_json(json!({"capabilities":{"alwaysMatch":{"pageLoadStrategy":"none"}}}))?
            .body_mut()
            .read_json()?;
        let id = v["value"]["sessionId"]
            .as_str()
            .or_else(|| v["sessionId"].as_str())
            .context("Servo returned no WebDriver session")?;
        let mut client = Self {
            agent: agent.clone(),
            base: format!("http://127.0.0.1:{port}/session/{id}"),
            home: String::new(),
        };
        // Servo creates a SECOND native window for a WebDriver session. Mark that
        // document so X11 discovery cannot accidentally embed the launcher window.
        client.post(
            "/execute/sync",
            json!({"script":"document.title = arguments[0]", "args":[marker]}),
        )?;
        client.home = client.get("/window")?.as_str().context("Servo returned no window handle")?.to_string();
        Ok(client)
    }
    /// Every engine tab and window (a page's `window.open` and `target=_blank` included).
    pub fn handles(&self) -> Result<Vec<String>> {
        let handles = self.get("/window/handles")?;
        Ok(handles.as_array().map(|a| a.iter().filter_map(|h| h.as_str().map(str::to_string)).collect()).unwrap_or_default())
    }
    /// Make `handle` the one commands act on; servoshell also shows it in its native window.
    pub fn switch(&self, handle: &str) -> Result<()> {
        self.post("/window", json!({"handle":handle}))?;
        Ok(())
    }
    /// A new engine tab (not selected); its handle.
    pub fn new_tab(&self) -> Result<String> {
        let v = self.post("/window/new", json!({"type":"tab"}))?;
        v["handle"].as_str().map(str::to_string).context("Servo returned no new tab")
    }
    /// A new engine tab in a native window of its own (servoshell opens one); its handle.
    pub fn new_window(&self) -> Result<String> {
        let v = self.post("/window/new", json!({"type":"window"}))?;
        v["handle"].as_str().map(str::to_string).context("Servo returned no new window")
    }
    /// Close the current engine tab. Commands need a `switch` afterwards.
    pub fn close_current(&self) -> Result<()> {
        self.agent.delete(format!("{}/window", self.base)).call()?;
        Ok(())
    }
    /// Address and title of the current engine tab.
    pub fn location(&self) -> Result<(String, String)> {
        let url = self.get("/url")?.as_str().unwrap_or_default().to_string();
        let title = self.get("/title")?.as_str().unwrap_or_default().to_string();
        Ok((url, title))
    }
    pub fn viewport_height(&self) -> Result<u32> {
        let viewport = self.post("/execute/sync", json!({
            "script":"return window.innerHeight * window.devicePixelRatio", "args":[]
        }))?;
        let height = viewport.as_f64().context("Servo returned no viewport height")?;
        if !height.is_finite() || height < 1. { bail!("Servo's viewport is not ready"); }
        Ok(height.round() as u32)
    }
    fn get(&self, path: &str) -> Result<Value> {
        Ok(self
            .agent
            .get(format!("{}{path}", self.base))
            .call()?
            .body_mut()
            .read_json::<Value>()?["value"]
            .clone())
    }
    fn post(&self, path: &str, body: Value) -> Result<Value> {
        let v: Value = self
            .agent
            .post(format!("{}{path}", self.base))
            .send_json(body)?
            .body_mut()
            .read_json()?;
        if v["value"]["error"].is_string() {
            bail!("Servo: {}", v["value"]["message"]);
        }
        Ok(v["value"].clone())
    }
    /// The page as a base64 PNG.
    pub fn screenshot(&self) -> Result<String> {
        self.get("/screenshot")?.as_str().map(str::to_string).context("Servo returned no image")
    }
    pub fn eval(&self, expression: &str) -> Result<String> {
        let v = self.post("/execute/sync", json!({"script":format!("return {expression}"),"args":[]}))?;
        Ok(reply_text(&v))
    }
    pub fn navigate(&self, url: &str) -> Result<()> {
        self.post("/url", json!({"url":url}))?;
        Ok(())
    }
    pub fn reload(&self) -> Result<()> {
        self.post("/refresh", json!({}))?;
        Ok(())
    }
    pub fn history(&self, delta: i32) -> Result<()> {
        self.post(if delta < 0 { "/back" } else { "/forward" }, json!({}))?;
        Ok(())
    }
    pub fn snapshot(&self) -> Result<Page> {
        if let Ok(v) = self.post(
            "/execute/sync",
            json!({"script":format!("return {SNAPSHOT}"),"args":[]}),
        ) {
            if let Some(s) = v.as_str() {
                return Ok(Page::from_value(&serde_json::from_str::<Value>(s)?));
            }
        }
        let url = self.get("/url")?;
        let title = self.get("/title")?;
        let body = self.post("/element", json!({"using":"css selector","value":"body"}))?;
        let id = body["element-6066-11e4-a52e-4f735466cecf"]
            .as_str()
            .context("Servo found no page body")?;
        let text = self.get(&format!("/element/{id}/text"))?;
        Ok(Page::from_value(
            &json!({"title":title,"url":url,"text":text}),
        ))
    }
}

pub fn http_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(5)))
        .build()
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_limit_counts_characters() {
        let page = Page::from_value(
            &json!({"title":"Unicode", "url":"https://example.com", "text":"é".repeat(60_010), "loading":true}),
        );
        assert_eq!(page.text.chars().count(), 60_000);
        assert!(page.loading);
        assert_eq!(page.title, "Unicode");
    }
}
