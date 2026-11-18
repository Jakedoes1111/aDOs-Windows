//! Notes module for aDOs.
//! See `panel` for the `NotesPanel` (dock panel name `ados_core::ids::NOTES`).

mod links;
pub mod disk;
pub mod file;
mod journal;
mod operations;
mod preview;
mod rows;
pub mod panel;
pub mod state;
mod sync;
pub use sync::flush_open;
mod view;

pub use panel::{NotesPanel, init};
pub use state::NotesState;

/// Most recently modified notes, with display titles and absolute file paths.
/// Missing/unreadable storage yields an empty list; the Notes panel surfaces I/O errors.
pub fn recent(limit: usize) -> Vec<(String, std::path::PathBuf)> {
    file::list_notes().unwrap_or_default().into_iter().take(limit)
        .map(|note| (note.title, note.path)).collect()
}
