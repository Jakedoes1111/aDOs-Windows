//! `ADOS_HOME/tasks/tasks.json`: one human-readable file for every task.
//! One file (not one per list) because reordering and moving between projects then stays a
//! single atomic rename, and a few hundred tasks are a few tens of kilobytes.
//! Every change reloads the file first, so edits made by other callers are never lost, and a file
//! that does not parse is never written over.
use crate::model::{Status, Task};
use anyhow::{Context, Result};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Serialize, Deserialize)]
struct TaskFile {
    version: u32,
    tasks: Vec<Task>,
}

pub fn default_path() -> PathBuf {
    ados_core::paths::home().join("tasks").join("tasks.json")
}

/// Tasks in list order. A missing file is an empty list.
pub fn load(path: &Path) -> Result<Vec<Task>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("Could not read {}", path.display())),
    };
    let file: TaskFile = serde_json::from_str(&text).with_context(|| {
        format!("{} could not be read, so it is kept as it is and nothing will be saved until it is fixed or moved away", path.display())
    })?;
    let mut tasks = file.tasks;
    tasks.sort_by_key(|t| t.order);
    Ok(tasks)
}

pub fn save(path: &Path, tasks: &[Task]) -> Result<()> {
    let text = serde_json::to_string_pretty(&TaskFile { version: 1, tasks: tasks.to_vec() })?;
    atomic_write(path, &(text + "\n"))
}

/// Load, change, save. Fails without writing if the file cannot be read.
pub fn update<R>(path: &Path, change: impl FnOnce(&mut Vec<Task>) -> Result<R>) -> Result<R> {
    let mut tasks = load(path)?;
    let result = change(&mut tasks)?;
    save(path, &tasks)?;
    Ok(result)
}

pub fn new_task(tasks: &[Task], title: &str) -> Task {
    let now = Local::now();
    Task {
        id: new_id(),
        title: title.trim().to_string(),
        notes: String::new(),
        status: Status::Open,
        due: None,
        project: None,
        my_day: None,
        created: now,
        updated: now,
        order: tasks.iter().map(|t| t.order).max().unwrap_or(0) + 1,
    }
}

/// Change one task by id and stamp `updated`.
pub fn edit(tasks: &mut [Task], id: &str, change: impl FnOnce(&mut Task)) -> Result<Task> {
    let task = tasks.iter_mut().find(|t| t.id == id).context("That task no longer exists")?;
    change(task);
    task.updated = Local::now();
    Ok(task.clone())
}

fn new_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() as u64;
    format!("t{:x}{:02x}", now, NEXT.fetch_add(1, Ordering::Relaxed) & 0xff)
}

/// Temp file in the same folder, flushed, then renamed over the original.
fn atomic_write(path: &Path, text: &str) -> Result<()> {
    let parent = path.parent().context("The tasks file has no folder")?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".tasks-{}-{}.tmp", std::process::id(), new_id()));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.with_context(|| format!("Could not save {}", path.display()))
}
