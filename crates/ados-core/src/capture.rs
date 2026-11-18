//! Quick capture: one input, type detected automatically, saved as a markdown file in the inbox.
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{sync::{Arc, Mutex, OnceLock, Weak}, task::{Poll, Waker}};

type ChangeState = Mutex<(bool, Option<Waker>)>;
static LISTENERS: OnceLock<Mutex<Vec<Weak<ChangeState>>>> = OnceLock::new();

/// Wake-driven, coalescing save notification; dropping it unsubscribes.
pub struct CaptureChanges(Arc<ChangeState>);

pub fn subscribe() -> CaptureChanges {
    let state = Arc::new(Mutex::new((false, None)));
    LISTENERS.get_or_init(Default::default).lock()
        .unwrap_or_else(|e| e.into_inner()).push(Arc::downgrade(&state));
    CaptureChanges(state)
}

impl CaptureChanges {
    pub async fn changed(&mut self) {
        std::future::poll_fn(|cx| {
            let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if std::mem::take(&mut state.0) {
                Poll::Ready(())
            } else {
                state.1 = Some(cx.waker().clone());
                Poll::Pending
            }
        }).await
    }
}

fn notify_saved() {
    notify_listeners(LISTENERS.get_or_init(Default::default));
}

fn notify_listeners(listeners: &Mutex<Vec<Weak<ChangeState>>>) {
    let mut wake = Vec::new();
    listeners.lock().unwrap_or_else(|e| e.into_inner()).retain(|weak| {
            let Some(listener) = weak.upgrade() else { return false; };
            let mut state = listener.lock().unwrap_or_else(|e| e.into_inner());
            state.0 = true;
            if let Some(waker) = state.1.take() { wake.push(waker); }
            true
        });
    for waker in wake { waker.wake(); }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureKind { Thought, Url, File, Image }

impl CaptureKind {
    pub fn label(self) -> &'static str {
        match self { Self::Thought => "thought", Self::Url => "url", Self::File => "file", Self::Image => "image" }
    }
}

#[derive(Debug, Clone)]
pub struct Capture { pub kind: CaptureKind, pub text: String, pub created: u64, pub path: PathBuf }

const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp"];

/// Decide what the user typed or pasted.
pub fn detect(input: &str) -> CaptureKind {
    let t = input.trim();
    let one_line = !t.contains('\n');
    if one_line && (t.starts_with("http://") || t.starts_with("https://")) && !t.contains(' ') {
        return CaptureKind::Url;
    }
    let p = t.strip_prefix("file://").unwrap_or(t);
    // `is_absolute`: `/…` on Linux, `C:\…` on Windows.
    if one_line && (Path::new(p).is_absolute() || p.starts_with("~/")) {
        let full = expand(p);
        if full.exists() {
            let ext = full.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            return if IMAGE_EXT.contains(&ext.as_str()) { CaptureKind::Image } else { CaptureKind::File };
        }
    }
    CaptureKind::Thought
}

fn expand(p: &str) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => std::env::home_dir().unwrap_or_default().join(rest),
        None => PathBuf::from(p),
    }
}

/// Save a capture. Returns the saved record, or an error to show the user.
pub fn save(input: &str) -> anyhow::Result<Capture> {
    let text = input.trim();
    anyhow::ensure!(!text.is_empty(), "Nothing to capture");
    let kind = detect(text);
    let created = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
    let path = crate::paths::inbox_dir().join(format!("{created}-{}.md", kind.label()));
    std::fs::write(&path, format!("---\nkind: {}\ncreated: {created}\n---\n{text}\n", kind.label()))?;
    notify_saved();
    Ok(Capture { kind, text: text.to_string(), created, path })
}

/// All captures, newest first. Unreadable files are skipped.
pub fn list() -> Vec<Capture> {
    let mut out: Vec<Capture> = std::fs::read_dir(crate::paths::inbox_dir())
        .into_iter().flatten().flatten()
        .filter_map(|e| read(&e.path()))
        .collect();
    out.sort_by(|a, b| b.created.cmp(&a.created));
    out
}

fn read(path: &Path) -> Option<Capture> {
    let raw = std::fs::read_to_string(path).ok()?;
    let rest = raw.strip_prefix("---\n")?;
    let (head, body) = rest.split_once("\n---\n")?;
    let field = |k: &str| head.lines().find_map(|l| l.strip_prefix(k).map(str::trim));
    let kind = match field("kind:")? {
        "url" => CaptureKind::Url, "file" => CaptureKind::File, "image" => CaptureKind::Image, _ => CaptureKind::Thought,
    };
    Some(Capture { kind, text: body.trim().to_string(), created: field("created:")?.parse().ok()?, path: path.to_path_buf() })
}

pub fn delete(capture: &Capture) -> anyhow::Result<()> { Ok(std::fs::remove_file(&capture.path)?) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_notifications_wake_and_coalesce() {
        use std::{future::Future, sync::atomic::{AtomicUsize, Ordering}, task::{Context, Wake}};
        struct Counter(AtomicUsize);
        impl Wake for Counter {
            fn wake(self: Arc<Self>) { self.0.fetch_add(1, Ordering::SeqCst); }
        }
        let counter = Arc::new(Counter(AtomicUsize::new(0)));
        let waker = Waker::from(counter.clone());
        let mut cx = Context::from_waker(&waker);
        let state = Arc::new(Mutex::new((false, None)));
        let listeners = Mutex::new(vec![Arc::downgrade(&state)]);
        let mut listener = CaptureChanges(state);
        {
            let mut next = std::pin::pin!(listener.changed());
            assert!(next.as_mut().poll(&mut cx).is_pending());
            notify_listeners(&listeners);
            assert!(counter.0.load(Ordering::SeqCst) > 0);
            assert!(next.as_mut().poll(&mut cx).is_ready());
        }
        notify_listeners(&listeners);
        notify_listeners(&listeners);
        assert!(std::pin::pin!(listener.changed()).as_mut().poll(&mut cx).is_ready());
        assert!(std::pin::pin!(listener.changed()).as_mut().poll(&mut cx).is_pending());
    }

    #[test]
    fn detects_kinds_and_round_trips() {
        let dir = std::env::temp_dir().join(format!("ados-capture-test-{}", std::process::id()));
        std::env::set_var("ADOS_HOME", &dir);
        assert_eq!(detect("https://example.com/a?b=1"), CaptureKind::Url);
        assert_eq!(detect("https://example.com is nice"), CaptureKind::Thought);
        assert_eq!(detect("/definitely/not/here.png"), CaptureKind::Thought);
        let img = dir.join("x.PNG");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&img, b"x").unwrap();
        assert_eq!(detect(img.to_str().unwrap()), CaptureKind::Image);
        let txt = dir.join("x.txt");
        std::fs::write(&txt, b"x").unwrap();
        assert_eq!(detect(txt.to_str().unwrap()), CaptureKind::File);
        assert!(save("   ").is_err());
        let c = save("buy milk\nand eggs").unwrap();
        let all = list();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].text, "buy milk\nand eggs");
        delete(&c).unwrap();
        assert!(list().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
