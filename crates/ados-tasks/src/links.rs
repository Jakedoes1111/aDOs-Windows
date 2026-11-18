//! URLs in a task's title or details open in the aDOs Browser (`ados_core::links`).
use crate::model::Task;
use crate::panel::TasksPanel;
use gpui_kit::component::WindowExt;
use gpui_kit::*;

/// The first web address in the title, else in the details.
pub(crate) fn task_url(task: &Task) -> Option<String> {
    [task.title.as_str(), task.notes.as_str()].into_iter()
        .find_map(|t| ados_core::links::find_urls(t).first().map(|r| t[r.clone()].to_string()))
}

impl TasksPanel {
    pub(crate) fn open_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.selected_task().as_ref().and_then(task_url) {
            Some(url) => ados_core::links::open_url(&url, window, cx),
            None => window.push_notification("This task has no link to open", cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn url_from_title_or_details() {
        let mut t = crate::store::new_task(&[], "Read https://a.org/x now");
        assert_eq!(task_url(&t).as_deref(), Some("https://a.org/x"));
        t.title = "Read".into();
        t.notes = "https://b.org".into();
        assert_eq!(task_url(&t).as_deref(), Some("https://b.org"));
        t.notes.clear();
        assert_eq!(task_url(&t), None);
    }
}
