//! The assistant card floats over the page. The page is a native child window that
//! always draws above GPUI, so instead of hiding the whole page while the card is
//! open, the card's rectangle is cut out of the page window with the X SHAPE
//! extension. Clicks there reach the card too: a window's input region never
//! extends past its bounding shape.
use crate::xembed::Rect;
use anyhow::Result;
use x11rb::{
    connection::{Connection, RequestConnection},
    protocol::{
        shape::{self, ConnectionExt as _, SK, SO},
        xproto::{ClipOrdering, Rectangle},
    },
    rust_connection::RustConnection,
};

/// Whether this X server can cut holes in windows. Without it the page hides while a
/// card is open, as before.
pub fn supported(conn: &RustConnection) -> bool {
    matches!(conn.extension_information(shape::X11_EXTENSION_NAME), Ok(Some(_)))
}

/// What to cut out of the page window. `modal` also empties the page's input region, so a
/// click on the visible page falls through to aDOs (whose backdrop closes the overlay)
/// instead of giving the page the keyboard while an overlay owns it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Holes {
    pub rects: Vec<Rect>,
    pub modal: bool,
}
impl Holes {
    pub fn is_empty(&self) -> bool { self.rects.is_empty() && !self.modal }
}

/// Give `clip` (sized `size`) its full shape, minus the holes in its own coordinates.
pub fn apply(conn: &RustConnection, clip: u32, size: Rect, holes: &Holes) -> Result<()> {
    let cut: Vec<Rectangle> = holes.rects.iter().filter_map(|hole| inside(*hole, size)).collect();
    if cut.is_empty() {
        conn.shape_mask(SO::SET, SK::BOUNDING, clip, 0, 0, x11rb::NONE)?.check()?;
    } else {
        let full = Rectangle { x: 0, y: 0, width: size.w as u16, height: size.h as u16 };
        conn.shape_rectangles(SO::SET, SK::BOUNDING, ClipOrdering::UNSORTED, clip, 0, 0, &[full])?.check()?;
        conn.shape_rectangles(SO::SUBTRACT, SK::BOUNDING, ClipOrdering::UNSORTED, clip, 0, 0, &cut)?.check()?;
    }
    if holes.modal {
        conn.shape_rectangles(SO::SET, SK::INPUT, ClipOrdering::UNSORTED, clip, 0, 0, &[])?.check()?;
    } else {
        conn.shape_mask(SO::SET, SK::INPUT, clip, 0, 0, x11rb::NONE)?.check()?;
    }
    conn.flush()?;
    Ok(())
}

/// The part of `hole` that lies inside a window of `size`, or None if they do not meet.
fn inside(hole: Rect, size: Rect) -> Option<Rectangle> {
    let left = hole.x.max(0);
    let top = hole.y.max(0);
    let right = (hole.x + hole.w as i32).min(size.w as i32);
    let bottom = (hole.y + hole.h as i32).min(size.h as i32);
    (right > left && bottom > top).then(|| Rectangle {
        x: left as i16,
        y: top as i16,
        width: (right - left) as u16,
        height: (bottom - top) as u16,
    })
}

#[cfg(test)]
mod tests {
    use super::{inside, Rect};

    #[test]
    fn hole_is_clipped_to_the_page() {
        let page = Rect { x: 0, y: 0, w: 800, h: 600 };
        let hole = inside(Rect { x: 600, y: -50, w: 440, h: 300 }, page).map(|r| (r.x, r.y, r.width, r.height));
        assert_eq!(hole, Some((600, 0, 200, 250)));
        assert!(inside(Rect { x: 900, y: 0, w: 40, h: 40 }, page).is_none());
    }
}
