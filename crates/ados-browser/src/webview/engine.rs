//! Windows: the engine is WebView2 (part of Windows 11), so there is nothing to find or
//! start, and Servo's automatic switch to Chromium never applies (`fallback.rs`).
pub fn chromium_found() -> bool { false }
