//! Tasks module for aDOs: a My Day / All list stored in `ADOS_HOME/tasks/tasks.json`.
//! Other crates use the small API below; it reads and writes the same file the panel shows.
pub mod model;
mod operations;
pub mod panel;
pub mod parse;
pub mod store;
mod links;
mod view;
#[cfg(test)]
mod tests;

pub use model::{Status, Task};
pub use panel::{init, refresh, TasksPanel};

fn today() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}

/// Every task, in list order.
pub fn list() -> anyhow::Result<Vec<Task>> {
    store::load(&store::default_path())
}

/// Add a task from quick-add text: `Call the bank friday #home`.
pub fn add(text: &str) -> anyhow::Result<Task> {
    add_at(&store::default_path(), text, today())
}

/// Mark a task done or open again.
pub fn set_done(id: &str, done: bool) -> anyhow::Result<Task> {
    set_done_at(&store::default_path(), id, done)
}

pub(crate) fn add_at(path: &std::path::Path, text: &str, today: chrono::NaiveDate) -> anyhow::Result<Task> {
    let parsed = parse::quick_add(text, today);
    anyhow::ensure!(!parsed.title.is_empty(), "A task needs a title");
    store::update(path, |tasks| {
        let mut task = store::new_task(tasks, &parsed.title);
        task.due = parsed.due;
        task.project = parsed.project;
        tasks.push(task.clone());
        Ok(task)
    })
}

pub(crate) fn set_done_at(path: &std::path::Path, id: &str, done: bool) -> anyhow::Result<Task> {
    store::update(path, |tasks| {
        store::edit(tasks, id, |t| t.status = if done { Status::Done } else { Status::Open })
    })
}
