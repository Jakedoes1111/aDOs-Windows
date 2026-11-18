//! "Open this URL in the Browser": the panel's side of `ados_core::links`.
//! A link from another module opens in a new page tab, unless the shown tab is blank.
use super::*;

impl BrowserPanel {
    /// Load typed text (a URL, or words to search for) and give the page the keyboard.
    pub fn open_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let url = state::address(text, self.state.engine);
        self.active = true;
        if !focus::is_blank(&self.state.url) {
            self.open_tab(url, window, cx);
            self.focus_primary(window, cx);
            cx.notify();
            return;
        }
        self.state.url = url.clone();
        self.tabs.current_mut().url = url.clone();
        self.persist_tabs();
        self.persist();
        self.error = None;
        self.input.update(cx, |input, cx| input.set_value(url.clone(), window, cx));
        self.send(CommandMsg::Navigate(url));
        self.focus_primary(window, cx);
        cx.notify();
    }
    /// Reads the page selection through the shared `control.rs` script, then calls `done` with it
    /// (empty when nothing is selected or the page does not answer).
    pub fn with_selection(&mut self, window: &mut Window, cx: &mut Context<Self>, done: impl FnOnce(String, &mut Window, &mut App) + 'static) {
        let rx = self.run(control::selection_script()).ok();
        cx.spawn_in(window, async move |_, cx| {
            let text = match rx {
                Some(rx) => rx.recv().await.ok().and_then(Result::ok).unwrap_or_default(),
                None => String::new(),
            };
            let selection = control::selection_text(&text);
            let _ = cx.update(|window, cx| done(selection, window, cx));
        })
        .detach();
    }
    /// The address the page last reported (what "Page to note" links to).
    pub fn current_url(&self) -> &str { &self.state.url }
}
