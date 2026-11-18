//! A Browser panel that is in no layout lets go of its engine (Servo is ~270 MB) after the
//! shell's idle delay, and starts it again, on the saved URL, when it is back in a layout.
use super::*;
use std::time::Duration;

impl BrowserPanel {
    /// The shell calls this on every dock change. Event driven: one timer while out of layout.
    pub fn set_in_layout(&mut self, in_layout: bool, idle: Duration, window: &mut Window, cx: &mut Context<Self>) {
        if in_layout {
            self.park_task = None;
            if self.parked {
                self.parked = false;
                self.start(window, cx);
                cx.notify();
            }
            return;
        }
        if self.park_task.is_some() || self.parked || self.runtime.is_none() {
            return;
        }
        self.park_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(idle).await;
            let _ = this.update(cx, |this, cx| {
                if let Some(runtime) = this.runtime.take() {
                    runtime.stop();
                }
                this.quit_subscription = None;
                this.ready = false;
                this.parked = true;
                ados_core::overlay::set_native_rect(None, cx);
                cx.notify();
            });
        }));
    }
}
