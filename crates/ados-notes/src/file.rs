//! Note storage. Writes are atomic; moves never overwrite an existing document.
use anyhow::{Context, Result};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone)]
pub struct NoteMeta {
    pub path: PathBuf,
    pub title: String,
    pub search_text: String,
    pub modified: SystemTime,
}

pub fn title_from_text(text: &str) -> String {
    let title = text
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .trim_start_matches('#')
        .trim();
    if title.is_empty() {
        "Untitled".into()
    } else {
        title.into()
    }
}

pub fn slugify(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars() {
        if c.is_alphanumeric() {
            if out.chars().count() >= 60 {
                break;
            }
            out.extend(c.to_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    if out.is_empty() {
        "untitled".into()
    } else {
        out.into()
    }
}

fn short_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{:010x}",
        (now as u64 ^ NEXT.fetch_add(1, Ordering::Relaxed)) & 0xffffffffff
    )
}

/// Stage in the same directory, flush, then atomically replace. The original survives failure.
pub fn atomic_write(path: &Path, text: &str) -> Result<()> {
    let parent = path.parent().context("The file has no parent folder")?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".notes-{}-{}.tmp", std::process::id(), short_id()));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.with_context(|| format!("Could not save {}", path.display()))
}

pub fn create_note_in(dir: &Path, text: &str) -> Result<PathBuf> {
    fs::create_dir_all(dir)?;
    for _ in 0..100 {
        let path = dir.join(format!(
            "{}-{}.md",
            slugify(&title_from_text(text)),
            short_id()
        ));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(text.as_bytes())?;
                file.sync_all()?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    anyhow::bail!("Could not find a free note filename")
}

/// Write `text` to a new file at `path`; fails if it exists.
pub fn write_new(path: &Path, text: &str) -> Result<()> {
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(path)
        .with_context(|| format!("Could not save {}", path.display()))?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

pub fn create_note(text: &str) -> Result<PathBuf> {
    create_note_in(&ados_core::paths::notes_dir(), text)
}

/// Hard-link then unlink is a no-clobber move on this same filesystem.
/// A crash between these operations leaves two copies, never zero.
pub fn move_no_replace(from: &Path, to: &Path) -> Result<()> {
    fs::hard_link(from, to)
        .with_context(|| format!("Could not move to {} (it may already exist)", to.display()))?;
    fs::remove_file(from).with_context(|| {
        format!(
            "The original {} was kept; the destination is also safe",
            from.display()
        )
    })?;
    Ok(())
}

pub fn rename_for_title(path: &Path, title: &str) -> Result<PathBuf> {
    let parent = path.parent().context("The note has no parent folder")?;
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .context("Invalid note filename")?;
    let id = stem
        .rsplit_once('-')
        .map(|(_, id)| id.to_owned())
        .unwrap_or_else(short_id);
    let slug = slugify(title);
    let canonical = parent.join(format!("{slug}-{id}.md"));
    if path == canonical || crate::disk::is_conflict_copy(path) {
        return Ok(path.into());
    }
    for n in 0..100 {
        let destination = if n == 0 {
            canonical.clone()
        } else {
            parent.join(format!("{slug}-{n}-{id}.md"))
        };
        if destination == path {
            return Ok(path.into());
        }
        match move_no_replace(path, &destination) {
            Ok(()) => return Ok(destination),
            Err(e)
                if e.downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::AlreadyExists) =>
            {
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    anyhow::bail!("Could not find a free filename for the renamed note")
}

pub fn write_note(path: &Path, text: &str) -> Result<PathBuf> {
    atomic_write(path, text)?;
    rename_for_title(path, &title_from_text(text))
}

pub fn list_notes() -> Result<Vec<NoteMeta>> {
    list_notes_in(&ados_core::paths::notes_dir())
}

pub fn list_notes_in(dir: &Path) -> Result<Vec<NoteMeta>> {
    fs::create_dir_all(dir)?;
    let mut notes = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md")
        {
            continue;
        }
        let text = fs::read_to_string(&path)
            .with_context(|| format!("Could not read {}", path.display()))?;
        notes.push(NoteMeta {
            path,
            title: title_from_text(&text),
            search_text: text.to_lowercase(),
            modified: entry.metadata()?.modified()?,
        });
    }
    notes.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(notes)
}

pub fn trash_note(path: &Path) -> Result<PathBuf> {
    let trash = path
        .parent()
        .context("Missing parent folder")?
        .join(".trash");
    fs::create_dir_all(&trash)?;
    let name = path
        .file_name()
        .context("Missing filename")?
        .to_string_lossy();
    for n in 0..100 {
        let to = trash.join(if n == 0 {
            name.to_string()
        } else {
            format!("{}-{name}", short_id())
        });
        match move_no_replace(path, &to) {
            Ok(()) => return Ok(to),
            Err(e)
                if e.downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::AlreadyExists) =>
            {
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    anyhow::bail!("Could not move the note to trash")
}

pub fn restore_note(trash: &Path, original: &Path) -> Result<()> {
    move_no_replace(trash, original)
}

/// What every undo of an assistant change says when the owner wrote in the note since.
pub const CHANGED_SINCE: &str = "This note changed since; undo would remove your edits.";

/// The text an append wrote: `before`, a blank line, then `added`.
pub fn appended(before: &str, added: &str) -> String {
    format!("{}\n\n{}\n", before.trim_end(), added.trim_end())
}

/// Undo of an append, given the note as it is now: the note without the appended block,
/// or `None` when that block is no longer there exactly once (the owner edited it).
/// Edits elsewhere in the note are kept.
pub fn without_appended(current: &str, before: &str, added: &str) -> Option<String> {
    if current == appended(before, added) {
        return Some(before.to_string());
    }
    if added.trim().is_empty() {
        return None;
    }
    // The block whole: a blank line, the text, then a line end (or the end of the note).
    let block = format!("\n\n{}", added.trim_end());
    let ends = |at: usize| {
        let rest = &current[at + block.len()..];
        rest.is_empty() || rest.starts_with('\n')
    };
    let found: Vec<usize> = current.match_indices(&block).map(|(i, _)| i).filter(|&i| ends(i)).collect();
    let [start] = found[..] else { return None };
    Some(format!("{}{}", &current[..start], &current[start + block.len()..]))
}

#[cfg(test)]
#[path = "file_tests.rs"]
mod tests;
