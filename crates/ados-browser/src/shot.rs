//! A cropped PNG of a comment target, read straight from the page's X window. Engine
//! neutral: it needs no screenshot call from Servo or Chromium, only the window pixels.
use crate::xembed::Rect;
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use x11rb::{
    connection::Connection,
    protocol::xproto::{ConnectionExt as _, ImageFormat, ImageOrder},
    rust_connection::RustConnection,
};

/// Raw RGB pixels of `rect` (viewport pixels) inside the engine's `window`, clipped to it.
/// Anything above the viewport (Servo's toolbar, a Chromium infobar) is the window height
/// minus `viewport_h`; without a height, `top` is used.
pub fn grab(conn: &RustConnection, window: u32, rect: Rect, viewport_h: u32, top: u32) -> Result<(u32, u32, Vec<u8>)> {
    let geometry = conn.get_geometry(window)?.reply()?;
    let bars = match u32::from(geometry.height).checked_sub(viewport_h) {
        Some(bars) if viewport_h > 0 => bars,
        _ => top,
    };
    let rect = Rect { y: rect.y + bars as i32, ..rect };
    let left = rect.x.max(0);
    let top = rect.y.max(0);
    let right = (rect.x + rect.w as i32).min(i32::from(geometry.width));
    let bottom = (rect.y + rect.h as i32).min(i32::from(geometry.height));
    if right <= left || bottom <= top { bail!("the target is outside the visible page"); }
    let (w, h) = ((right - left) as u16, (bottom - top) as u16);
    let reply = conn.get_image(ImageFormat::Z_PIXMAP, window, left as i16, top as i16, w, h, !0)?.reply()?;
    let setup = conn.setup();
    let bpp = setup.pixmap_formats.iter().find(|f| f.depth == reply.depth).map(|f| f.bits_per_pixel);
    if bpp != Some(32) || setup.image_byte_order != ImageOrder::LSB_FIRST {
        bail!("unsupported pixel format");
    }
    let pixels = usize::from(w) * usize::from(h);
    if reply.data.len() < pixels * 4 { bail!("short image reply"); }
    // Little-endian 32 bpp is B, G, R, X in memory.
    let mut rgb = Vec::with_capacity(pixels * 3);
    for px in reply.data.chunks_exact(4).take(pixels) {
        rgb.extend_from_slice(&[px[2], px[1], px[0]]);
    }
    Ok((u32::from(w), u32::from(h), rgb))
}

pub fn write_png(path: &Path, width: u32, height: u32, rgb: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() { std::fs::create_dir_all(dir)?; }
    let file = std::fs::File::create(path).with_context(|| format!("Could not write {}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(rgb)?;
    Ok(())
}

/// Where a comment's crop goes: `ADOS_HOME/cache/comments/<ms>-<n>.png`.
pub fn path_for(number: usize) -> PathBuf {
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    ados_core::paths::home().join("cache").join("comments").join(format!("{ms}-{number}.png"))
}

/// A viewport rectangle in CSS px to page-window pixels.
pub fn to_window(viewport: [f64; 4], dpr: f64) -> Rect {
    let k = if dpr > 0. { dpr } else { 1. };
    let x = (viewport[0] * k).floor();
    let y = (viewport[1] * k).floor();
    let r = ((viewport[0] + viewport[2]) * k).ceil();
    let b = ((viewport[1] + viewport[3]) * k).ceil();
    Rect { x: x as i32, y: y as i32, w: (r - x).max(0.) as u32, h: (b - y).max(0.) as u32 }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn css_to_window_pixels() {
        let r = to_window([10.5, 20., 30., 40.25], 2.);
        assert_eq!((r.x, r.y, r.w, r.h), (21, 40, 60, 81));
        let r = to_window([0., 0., 5., 5.], 0.);
        assert_eq!((r.w, r.h), (5, 5));
    }
}
