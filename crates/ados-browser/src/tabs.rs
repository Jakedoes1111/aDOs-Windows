//! The Browser's page tabs: order, selection, the tab each popup came from, closed tabs to
//! reopen. Pure bookkeeping; `tab_strip.rs` draws it and `driver.rs` keeps the engine in step.

#[derive(Clone, Debug)]
pub struct PageTab {
    pub id: u64,
    pub url: String,
    pub title: String,
    /// The tab whose page opened this one (a popup): selected again when this one closes itself.
    pub opener: Option<u64>,
    /// Opened in the background while another module had the keyboard, not looked at yet.
    pub unseen: bool,
    /// Back/Forward position, kept while another tab is shown.
    pub nav: crate::nav::Nav,
}
impl PageTab {
    fn new(id: u64, url: String) -> Self {
        Self { id, url, title: String::new(), opener: None, unseen: false, nav: Default::default() }
    }
}

pub struct Tabs {
    pub list: Vec<PageTab>,
    pub selected: usize,
    /// Closed tabs, the latest last (Ctrl+Shift+T): address and place in the strip.
    closed: Vec<(String, usize)>,
    next_id: u64,
}

const CLOSED_KEPT: usize = 20;

impl Tabs {
    /// Saved addresses and selection; never empty (`fallback` when nothing was saved).
    pub fn restore(urls: &[String], selected: usize, fallback: &str) -> Self {
        let mut tabs = Self { list: vec![], selected: 0, closed: vec![], next_id: 1 };
        for url in urls.iter().filter(|u| !u.trim().is_empty()) {
            let id = tabs.fresh_id();
            tabs.list.push(PageTab::new(id, url.clone()));
        }
        if tabs.list.is_empty() {
            let id = tabs.fresh_id();
            tabs.list.push(PageTab::new(id, fallback.into()));
        }
        tabs.selected = selected.min(tabs.list.len() - 1);
        tabs
    }
    fn fresh_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    pub fn current(&self) -> &PageTab { &self.list[self.selected] }
    pub fn current_mut(&mut self) -> &mut PageTab { &mut self.list[self.selected] }
    pub fn index_of(&self, id: u64) -> Option<usize> { self.list.iter().position(|t| t.id == id) }
    pub fn get_mut(&mut self, id: u64) -> Option<&mut PageTab> { self.list.iter_mut().find(|t| t.id == id) }
    pub fn urls(&self) -> Vec<String> { self.list.iter().map(|t| t.url.clone()).collect() }

    /// A new tab at `url` right after the selected one; returns its index (not selected yet).
    pub fn add(&mut self, url: &str) -> usize {
        let id = self.fresh_id();
        let at = self.selected + 1;
        self.list.insert(at, PageTab::new(id, url.into()));
        at
    }
    /// A tab a page opened (`id` from the engine side): placed after its opener's other popups.
    pub fn adopt(&mut self, id: u64, opener: Option<u64>, url: String, title: String) -> usize {
        let mut at = opener.and_then(|o| self.index_of(o)).map_or(self.list.len(), |i| i + 1);
        while at < self.list.len() && opener.is_some() && self.list[at].opener == opener { at += 1; }
        let mut tab = PageTab::new(id, url);
        tab.title = title;
        tab.opener = opener;
        self.list.insert(at, tab);
        if at <= self.selected && self.list.len() > 1 { self.selected += 1; }
        at
    }
    pub fn select(&mut self, index: usize) {
        if index < self.list.len() {
            self.selected = index;
            self.list[index].unseen = false;
        }
    }
    /// The index `delta` steps from the selected one, wrapping round.
    pub fn step(&self, delta: i32) -> usize {
        let n = self.list.len() as i32;
        (self.selected as i32 + delta).rem_euclid(n) as usize
    }
    /// Remove tab `id`. Returns the index to select next when it was the selected one: its
    /// opener if still open, else the tab to its right, else the one to its left. `remember`
    /// keeps the address for Ctrl+Shift+T (not for a popup that closed itself).
    pub fn remove(&mut self, id: u64, remember: bool) -> Option<usize> {
        let index = self.index_of(id)?;
        let tab = self.list.remove(index);
        if remember && !crate::focus::is_blank(&tab.url) {
            self.closed.push((tab.url.clone(), index));
            if self.closed.len() > CLOSED_KEPT { self.closed.remove(0); }
        }
        if self.list.is_empty() {
            self.selected = 0;
            return None;
        }
        if index == self.selected {
            let next = tab.opener.and_then(|o| self.index_of(o)).unwrap_or(index.min(self.list.len() - 1));
            self.selected = next;
            Some(next)
        } else {
            if index < self.selected { self.selected -= 1; }
            None
        }
    }
    /// The last closed tab: its address and the index it had (clamped to the strip).
    pub fn reopen(&mut self) -> Option<(String, usize)> {
        let (url, index) = self.closed.pop()?;
        Some((url, index.min(self.list.len())))
    }
    /// A new tab at `url` at `index`; returns its index (not selected yet).
    pub fn insert(&mut self, index: usize, url: &str) -> usize {
        let id = self.fresh_id();
        let at = index.min(self.list.len());
        self.list.insert(at, PageTab::new(id, url.into()));
        if at <= self.selected && self.list.len() > 1 { self.selected += 1; }
        at
    }
    /// Put tabs `ids` side by side in that order, where the first of them was (a tile group
    /// reads as one block in the strip). The selected tab stays selected.
    pub fn gather(&mut self, ids: &[u64]) {
        let selected = self.current().id;
        let Some(at) = ids.iter().filter_map(|id| self.index_of(*id)).min() else { return };
        let before = self.list[..at].len();
        let mut moved: Vec<PageTab> = vec![];
        for id in ids {
            if let Some(i) = self.index_of(*id) { moved.push(self.list.remove(i)); }
        }
        let at = before.min(self.list.len());
        for (k, tab) in moved.into_iter().enumerate() { self.list.insert(at + k, tab); }
        self.selected = self.index_of(selected).unwrap_or(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn urls(t: &Tabs) -> Vec<&str> { t.list.iter().map(|t| t.url.as_str()).collect() }

    #[test]
    fn restore_never_empty_and_clamps() {
        let t = Tabs::restore(&[], 4, "start");
        assert_eq!(urls(&t), ["start"]);
        assert_eq!(t.selected, 0);
        let t = Tabs::restore(&["a".into(), "b".into()], 9, "start");
        assert_eq!(t.selected, 1);
    }
    #[test]
    fn add_goes_right_of_selected() {
        let mut t = Tabs::restore(&["a".into(), "b".into()], 0, "s");
        let i = t.add("c");
        assert_eq!(i, 1);
        assert_eq!(urls(&t), ["a", "c", "b"]);
    }
    #[test]
    fn popup_closing_selects_opener_and_is_not_reopened() {
        let mut t = Tabs::restore(&["a".into(), "b".into()], 0, "s");
        let opener = t.current().id;
        let i = t.adopt(1 << 40, Some(opener), "popup".into(), String::new());
        assert_eq!(i, 1);
        t.select(i);
        assert_eq!(t.remove(1 << 40, false), Some(0));
        assert_eq!(t.current().id, opener);
        assert_eq!(t.reopen(), None);
    }
    #[test]
    fn reopen_returns_to_its_place() {
        let mut t = Tabs::restore(&["a".into(), "b".into(), "c".into(), "d".into()], 3, "s");
        let a = t.list[0].id;
        t.remove(a, true);
        assert_eq!(t.current().url, "d");
        let (url, at) = t.reopen().unwrap();
        let i = t.insert(at, &url);
        assert_eq!(i, 0);
        assert_eq!(urls(&t), ["a", "b", "c", "d"]);
        assert_eq!(t.current().url, "d");
    }
    #[test]
    fn background_adopt_keeps_selection() {
        let mut t = Tabs::restore(&["a".into(), "b".into()], 1, "s");
        let b = t.current().id;
        let first = t.list[0].id;
        t.adopt(1 << 40, Some(first), "p".into(), String::new());
        assert_eq!(t.current().id, b);
        assert_eq!(urls(&t), ["a", "p", "b"]);
    }
    #[test]
    fn gather_makes_a_block() {
        let mut t = Tabs::restore(&["a".into(), "b".into(), "c".into(), "d".into()], 3, "s");
        let (b, d) = (t.list[1].id, t.list[3].id);
        t.gather(&[b, d]);
        assert_eq!(urls(&t), ["a", "b", "d", "c"]);
        assert_eq!(t.current().url, "d");
    }
    #[test]
    fn close_and_reopen_and_cycle() {
        let mut t = Tabs::restore(&["a".into(), "b".into(), "c".into()], 1, "s");
        let b = t.current().id;
        assert_eq!(t.remove(b, true), Some(1));
        assert_eq!(t.current().url, "c");
        assert_eq!(t.reopen(), Some(("b".to_string(), 1)));
        assert_eq!(t.step(1), 0);
        assert_eq!(t.step(-1), 0);
        let a = t.list[0].id;
        assert_eq!(t.remove(a, true), None);
        assert_eq!(t.current().url, "c");
    }
}
