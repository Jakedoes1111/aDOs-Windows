//! Links in a note open in the aDOs Browser (`ados_core::links`).
use crate::panel::NotesPanel;
use gpui_kit::component::WindowExt;
use gpui_kit::*;

impl NotesPanel {
    /// Keyboard: the link in the selection, else under the cursor, else the note's first link.
    pub(crate) fn open_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.editor.read(cx);
        let selected = editor.selected_text().to_string();
        let url = ados_core::links::find_urls(&selected).first().map(|r| selected[r.clone()].to_string())
            .or_else(|| {
                let at = editor.cursor().min(self.text.len());
                let start = self.text[..at].rfind('\n').map_or(0, |i| i + 1);
                let end = self.text[at..].find('\n').map_or(self.text.len(), |i| at + i);
                ados_core::links::url_at(&self.text[start..end], at - start).map(str::to_string)
            })
            .or_else(|| ados_core::links::find_urls(&self.text).first().map(|r| self.text[r.clone()].to_string()));
        match url {
            Some(url) => ados_core::links::open_url(&url, window, cx),
            None => window.push_notification("This note has no link to open", cx),
        }
    }
}
