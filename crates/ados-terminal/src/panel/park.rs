//! A Terminal panel that is in no layout stops its shell after the shell's idle delay, but
//! only when no job holds the terminal; with one running (a build, `vim`) it keeps it and
//! looks again after another delay. Back in a layout, a new shell starts below the saved
//! transcript, as after a relaunch.
use super::TerminalPanel;
use gpui_kit::*;
use std::time::Duration;

impl TerminalPanel {
    /// The shell calls this on every dock change. Event driven: one timer while out of layout.
    pub fn set_in_layout(&mut self, in_layout: bool, idle: Duration, window: &mut Window, cx: &mut Context<Self>) {
        if in_layout {
            self.park_task = None;
            if let Some(history) = self.parked.take() {
                self.start(&history, window, cx);
                cx.notify();
            }
            return;
        }
        if self.park_task.is_some() || self.parked.is_some() || self.backend.is_none() {
            return;
        }
        self.park_task = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(idle).await;
            match this.update(cx, |this, cx| this.park(cx)) {
                Ok(false) => continue,
                _ => break,
            }
        }));
    }

    /// Stop the shell unless a job runs. True when there is nothing more to wait for.
    fn park(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(backend) = &self.backend else { return true };
        if backend.foreground_job() {
            return false;
        }
        self.parked = Some(backend.history(2000));
        self.receiver_task = None;
        self.backend = None;
        cx.notify();
        true
    }
}
