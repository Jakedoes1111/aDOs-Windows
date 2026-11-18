//! Where the page sits in its history, so Back and Forward are only offered when they can work.
//! The engines expose `history.length` but not the position, so the position is tracked here.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Nav {
    pos: usize,
    len: usize,
    url: String,
    /// A Back (-1) or Forward (1) sent and not yet seen in a snapshot.
    pending: i32,
    /// Entries before this one were there before the first page this tab showed (the engine's
    /// blank start page): Back does not go there.
    floor: usize,
}
impl Default for Nav {
    fn default() -> Self { Self { pos: 0, len: 1, url: String::new(), pending: 0, floor: 0 } }
}
impl Nav {
    pub fn url(&self) -> &str { &self.url }
    pub fn can_back(&self) -> bool { self.pos > self.floor }
    pub fn can_forward(&self) -> bool { self.pos + 1 < self.len }
    /// Accept a Back/Forward request. False means there is nowhere to go: send nothing.
    pub fn go(&mut self, delta: i32) -> bool {
        let ok = if delta < 0 { self.can_back() } else { self.can_forward() };
        if ok { self.pending = delta; }
        ok
    }
    /// A snapshot arrived: `url` and `history.length`.
    pub fn page(&mut self, url: &str, len: usize) {
        let len = len.max(1);
        // The same address loaded again (Enter in the address bar, a reload by script) still
        // adds an entry: a longer history with no Back/Forward pending ends at the new entry.
        if url == self.url && len > self.len && self.pending == 0 { self.pos = len - 1; }
        self.len = len;
        if url != self.url {
            if crate::focus::is_blank(&self.url) {
                // First page this tab shows: it is the newest entry (a restored or reopened tab
                // can have the engine's blank page before it, and nothing after it).
                self.pos = self.len - 1;
                self.floor = self.pos;
            } else if self.pending < 0 {
                self.pos = self.pos.saturating_sub(1);
            } else if self.pending > 0 {
                self.pos += 1;
            } else {
                // A new page (link, address, redirect): the newest entry.
                self.pos = self.len - 1;
            }
            self.pending = 0;
            self.url = url.to_string();
        }
        self.pos = self.pos.min(self.len - 1);
        self.floor = self.floor.min(self.pos);
    }
    /// The engine restarted: its history is empty again.
    pub fn reset(&mut self) { *self = Self::default(); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fresh_session_has_nowhere_to_go() {
        let mut n = Nav::default();
        n.page("http://a/", 1);
        assert!(!n.can_back() && !n.can_forward());
        assert!(!n.go(-1) && !n.go(1));
    }
    #[test]
    fn back_forward_follow_the_position() {
        let mut n = Nav::default();
        n.page("http://a/", 1);
        n.page("http://b/", 2);
        n.page("http://c/", 3);
        assert!(n.can_back() && !n.can_forward());
        assert!(n.go(-1));
        n.page("http://b/", 3);
        assert!(n.can_back() && n.can_forward());
        assert!(n.go(-1));
        n.page("http://a/", 3);
        assert!(!n.can_back() && n.can_forward());
        // A new link from the middle drops the forward entries (the engine's length shrinks).
        n.page("http://d/", 2);
        assert!(n.can_back() && !n.can_forward());
    }
    #[test]
    fn restored_tab_behind_a_blank_entry_has_nowhere_to_go() {
        let mut n = Nav::default();
        n.page("http://a/", 2);
        assert!(!n.can_back() && !n.can_forward());
        n.page("http://b/", 3);
        assert!(n.can_back() && !n.can_forward());
        assert!(n.go(-1));
        n.page("http://a/", 3);
        assert!(!n.can_back() && n.can_forward());
    }
    #[test]
    fn new_tab_with_a_typed_address_has_no_back() {
        let mut n = Nav::default();
        n.page("about:blank", 1);
        n.page("http://a/", 2);
        assert!(!n.can_back() && !n.can_forward());
    }
    #[test]
    fn same_address_again_ends_at_the_new_entry() {
        let mut n = Nav::default();
        n.page("http://a/", 2);
        n.page("http://a/", 3);
        assert!(!n.can_forward());
        assert!(n.can_back());
    }
    #[test]
    fn redirect_does_not_overrun() {
        let mut n = Nav::default();
        n.page("http://a/", 1);
        n.page("http://a/redirected", 1);
        assert!(!n.can_back());
    }
}
