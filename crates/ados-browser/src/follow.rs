//! Follower tabs on the engine side: a leader tile holds plain link clicks (`FOLLOW_TAKE`), the
//! driver reads them and opens them in the follower tile. Servo is read by polling over
//! WebDriver (it has no events); Chromium the same way over CDP, for one code path.
use crate::{
    driver::{Driver, Kind},
    runtime::Event,
};
use anyhow::Result;
use std::time::{Duration, Instant};

/// How often the leader's followed links are read.
const FOLLOW_EVERY: Duration = Duration::from_millis(250);
/// Installed in a leader on its first read, then reads and empties its queue of followed
/// links. A plain left click on a link that would navigate this page (no modifier, no
/// `target=_blank` or named target, not a download, not a same-page `#` jump, not already
/// handled by the page's own script) is held and opened in the follower instead.
macro_rules! follow_script {
    ($end:literal) => { concat!(r#"(() => { const w = window;
 if (!w.__adosFollowOn) { w.__adosFollowOn = true; w.__adosFollow = [];
  w.addEventListener('click', e => {
   if (!w.__adosLead || e.defaultPrevented || e.button !== 0 || e.ctrlKey || e.metaKey || e.shiftKey || e.altKey) return;
   const a = e.target && e.target.closest && e.target.closest('a[href]');
   if (!a || a.hasAttribute('download')) return;
   const t = (a.getAttribute('target') || '').toLowerCase();
   if (t && t !== '_self' && t !== '_top' && t !== '_parent') return;
   const u = new URL(a.href, location.href);
   if (u.protocol !== 'http:' && u.protocol !== 'https:') return;
   if (u.hash && u.href.split('#')[0] === location.href.split('#')[0]) return;
   e.preventDefault(); w.__adosFollow.push(u.href); }); }
 w.__adosLead = true; "#, $end, " })()") };
}
pub(crate) const FOLLOW_TAKE: &str = follow_script!("const q = w.__adosFollow; w.__adosFollow = []; return JSON.stringify(q);");
/// Only installs: links already held stay for the next `FOLLOW_TAKE`.
pub(crate) const FOLLOW_INSTALL: &str = follow_script!("return '[]';");

fn parse_links(text: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(text).unwrap_or_default()
}

impl Driver {
    /// New follower links from the panel. A leader that leads no more stops holding its links.
    pub(crate) fn set_follows(&mut self, follows: Vec<(u64, u64)>) -> Result<()> {
        let dropped: Vec<u64> = self.follows.iter().filter(|p| !follows.contains(p)).map(|(_, l)| *l).collect();
        self.follows = follows;
        let still: Vec<u64> = self.follows.iter().map(|(_, l)| *l).collect();
        for leader in dropped.into_iter().filter(|l| !still.contains(l)) {
            self.eval_in(leader, "window.__adosLead = false");
        }
        self.next_follow = Some(Instant::now());
        self.follow_reply = None;
        // A leader holds plain link clicks from now on, even before it is the active tile.
        for (_, leader) in self.follows.clone() { self.eval_in(leader, FOLLOW_INSTALL); }
        Ok(())
    }
    /// Run `script` in tab `id` and forget the result; WebDriver goes back to the selected tab.
    pub(crate) fn eval_in(&mut self, id: u64, script: &str) {
        let Some(handle) = self.handle_of(id) else { return };
        let back = self.selected.and_then(|s| self.handle_of(s));
        match &mut self.kind {
            Kind::Servo { wd, .. } => {
                if wd.switch(&handle).is_ok() { let _ = wd.eval(script); }
                if let Some(back) = back { let _ = wd.switch(&back); }
            }
            Kind::Cdp { pages, .. } => {
                if let Some(page) = pages.iter_mut().find(|p| p.target == handle) {
                    let (tx, _) = async_channel::bounded(1);
                    let _ = page.eval(script, tx);
                }
            }
        }
    }
    /// The follower of the selected tab, when both are shown.
    fn follower(&self) -> Option<u64> {
        let leader = self.selected?;
        let (follower, _) = self.follows.iter().find(|(_, l)| *l == leader)?;
        self.shown_ids().contains(follower).then_some(*follower)
    }
    /// When the driver loop must run `tick` next.
    pub fn timer_due(&self) -> Option<Duration> {
        let follow = self.follower().and(self.next_follow);
        let read = self.reads.iter().map(|(_, at)| *at).min();
        follow.into_iter().chain(read).min().map(|at| at.saturating_duration_since(Instant::now()))
    }
    /// Read the leader's queue of followed links (`FOLLOW_TAKE`) and open them in its
    /// follower; read Servo followers that navigated.
    pub fn tick(&mut self, events: &async_channel::Sender<Event>) -> Result<()> {
        let now = Instant::now();
        if let Some(follower) = self.follower() {
            if self.next_follow.is_some_and(|at| at <= now) {
                self.next_follow = Some(now + FOLLOW_EVERY);
                let leader = self.handle_of(self.selected.unwrap_or_default());
                let follower_handle = self.handle_of(follower);
                let mut links = vec![];
                match &mut self.kind {
                    Kind::Servo { wd, .. } => {
                        if let Ok(text) = wd.eval(FOLLOW_TAKE) { links = parse_links(&text); }
                    }
                    Kind::Cdp { pages, .. } => {
                        if let Some(rx) = self.follow_reply.take() {
                            match rx.try_recv() {
                                Ok(Ok(text)) => links = parse_links(&text),
                                Err(async_channel::TryRecvError::Empty) => self.follow_reply = Some(rx),
                                _ => {}
                            }
                        }
                        if self.follow_reply.is_none() {
                            if let Some(page) = pages.iter_mut().find(|p| Some(&p.target) == leader.as_ref()) {
                                let (tx, rx) = async_channel::bounded(1);
                                page.eval(FOLLOW_TAKE, tx)?;
                                self.follow_reply = Some(rx);
                            }
                        }
                    }
                }
                if let (Some(url), Some(target)) = (links.pop(), follower_handle) {
                    eprintln!("Browser follower tab {follower} opens {url}");
                    let _ = events.try_send(Event::Followed(follower, url.clone()));
                    match &mut self.kind {
                        Kind::Servo { wd, .. } => {
                            wd.switch(&target)?;
                            wd.navigate(&url)?;
                            if let Some(leader) = &leader { wd.switch(leader)?; }
                            self.reads.retain(|(id, _)| *id != follower);
                            self.reads.push((follower, now + Duration::from_millis(800)));
                            self.reads.push((follower, now + Duration::from_secs(3)));
                        }
                        Kind::Cdp { pages, .. } => {
                            if let Some(page) = pages.iter_mut().find(|p| p.target == target) { page.navigate(&url)?; }
                        }
                    }
                }
            }
        }
        let due: Vec<u64> = self.reads.iter().filter(|(_, at)| *at <= now).map(|(id, _)| *id).collect();
        self.reads.retain(|(_, at)| *at > now);
        for id in due {
            let (Some(handle), back) = (self.handle_of(id), self.selected.and_then(|s| self.handle_of(s))) else { continue };
            let leads = self.follows.iter().any(|(_, l)| *l == id);
            if let Kind::Servo { wd, .. } = &self.kind {
                wd.switch(&handle)?;
                if let Ok(page) = wd.snapshot() { let _ = events.try_send(Event::Page(id, page)); }
                if leads { let _ = wd.eval(FOLLOW_INSTALL); }
                if let Some(back) = back { wd.switch(&back)?; }
            }
        }
        Ok(())
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn followed_links_parse() {
        assert_eq!(parse_links(r#"["http://a/","http://b/"]"#), ["http://a/", "http://b/"]);
        assert!(parse_links("").is_empty());
        assert!(FOLLOW_TAKE.ends_with("return JSON.stringify(q); })()"));
        assert!(FOLLOW_INSTALL.contains("__adosFollowOn"));
    }
}
