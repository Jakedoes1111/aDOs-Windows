//! One custom element: background quads and one shaped line per terminal row.
use crate::{colors, panel::TerminalPanel, pty::Size};
use alacritty_terminal::{
    term::cell::Flags,
    vte::ansi::{CursorShape, Rgb},
};
use gpui_kit::component::ActiveTheme;
use gpui_kit::*;

pub struct TerminalElement {
    pub panel: Entity<TerminalPanel>,
}
pub struct PaintedGrid {
    backgrounds: Vec<(Bounds<Pixels>, Hsla)>,
    lines: Vec<(Point<Pixels>, ShapedLine)>,
    cursor: Option<(Bounds<Pixels>, Hsla)>,
    line_height: Pixels,
}
impl IntoElement for TerminalElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for TerminalElement {
    type RequestLayoutState = (Pixels, Pixels);
    type PrepaintState = PaintedGrid;
    fn id(&self) -> Option<ElementId> {
        Some("terminal-grid".into())
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let font = terminal_font(window);
        let font_size = self.panel.read(cx).font_size;
        let font_id = window.text_system().resolve_font(&font);
        let width = window
            .text_system()
            .advance(font_id, px(font_size), 'M')
            .map(|s| s.width)
            .unwrap_or(px(9.));
        let style = Style {
            size: size(relative(1.).into(), relative(1.).into()),
            ..Default::default()
        };
        (window.request_layout(style, [], cx), (width, px((font_size * 1.4).ceil())))
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        metrics: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> PaintedGrid {
        let (cw, ch) = *metrics;
        self.panel.update(cx, |panel, _| {
            panel.bounds = bounds;
            panel.cell_size = size(cw, ch);
            if let Some(backend) = panel.backend.as_mut() {
                let dimensions = Size {
                    columns: ((bounds.size.width / cw).floor() as usize)
                        .clamp(2, u16::MAX as usize),
                    rows: ((bounds.size.height / ch).floor() as usize).clamp(1, u16::MAX as usize),
                    cell_width: f32::from(cw).ceil() as u16,
                    cell_height: f32::from(ch).ceil() as u16,
                };
                if !backend.exited() {
                    if let Err(error) = backend.resize(dimensions) {
                        panel.error = Some(format!("Could not resize the terminal: {error}"));
                    }
                }
            }
        });
        let mut state = PaintedGrid {
            backgrounds: vec![],
            lines: vec![],
            cursor: None,
            line_height: ch,
        };
        let panel = self.panel.read(cx);
        let Some(backend) = &panel.backend else {
            return state;
        };
        let term = backend.term.lock();
        let content = term.renderable_content();
        let fg = to_rgb(cx.theme().foreground);
        let bg = to_rgb(cx.theme().background);
        let selection_color = cx.theme().selection;
        let cursor_color: Hsla = rgb(ados_core::brand::BLUE).into();
        let base_font = terminal_font(window);
        let offset = content.display_offset as i32;
        let cursor = content.cursor;
        let mut rows: Vec<(String, Vec<TextRun>)> =
            vec![(String::new(), Vec::new()); (bounds.size.height / ch).floor().max(1.) as usize];
        for indexed in content.display_iter {
            let row = (indexed.point.line.0 + offset) as usize;
            let Some((text, runs)) = rows.get_mut(row) else {
                continue;
            };
            let cell = indexed.cell;
            let flags = cell.flags;
            let mut foreground = colors::resolve(cell.fg, content.colors, fg, bg);
            let mut background = colors::resolve(cell.bg, content.colors, fg, bg);
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut foreground, &mut background);
            }
            if flags.contains(Flags::DIM) {
                foreground = Rgb {
                    r: foreground.r / 2,
                    g: foreground.g / 2,
                    b: foreground.b / 2,
                };
            }
            if flags.contains(Flags::HIDDEN) {
                foreground = background;
            }
            let mut foreground = colors::gpui(foreground);
            let mut background = colors::gpui(background);
            let selected = content.selection.is_some_and(|s| s.contains(indexed.point));
            if selected {
                background = selection_color;
            }
            let is_cursor = indexed.point == cursor.point && cursor.shape != CursorShape::Hidden;
            if is_cursor && cursor.shape == CursorShape::Block && panel.focus.is_focused(window) {
                background = cursor_color;
                foreground = cx.theme().background;
            }
            let origin = bounds.origin + point(cw * indexed.point.column.0, ch * row);
            let cell_bounds = Bounds::new(origin, size(cw, ch));
            if background != colors::gpui(bg) {
                state.backgrounds.push((cell_bounds, background));
            }
            if is_cursor && (cursor.shape != CursorShape::Block || !panel.focus.is_focused(window))
            {
                let rect = match cursor.shape {
                    CursorShape::Beam => Bounds::new(origin, size(px(2.), ch)),
                    _ => Bounds::new(origin + point(px(0.), ch - px(2.)), size(cw, px(2.))),
                };
                state.cursor = Some((rect, cursor_color));
            }
            let before = text.len();
            if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                text.push(' ');
            } else {
                text.push(if cell.c.is_control() { ' ' } else { cell.c });
                if let Some(extra) = cell.zerowidth() {
                    text.extend(extra.iter().filter(|c| !c.is_control()));
                }
            }
            let mut font = base_font.clone();
            if flags.contains(Flags::BOLD) {
                font.weight = FontWeight::BOLD;
            }
            if flags.contains(Flags::ITALIC) {
                font.style = FontStyle::Italic;
            }
            let underline = flags
                .intersects(Flags::ALL_UNDERLINES)
                .then_some(UnderlineStyle {
                    thickness: px(1.),
                    color: Some(foreground),
                    wavy: flags.contains(Flags::UNDERCURL),
                });
            let strikethrough = flags
                .contains(Flags::STRIKEOUT)
                .then_some(StrikethroughStyle {
                    thickness: px(1.),
                    color: Some(foreground),
                });
            let run = TextRun {
                len: text.len() - before,
                font,
                color: foreground,
                background_color: None,
                underline,
                strikethrough,
            };
            if let Some(last) = runs.last_mut().filter(|last| {
                last.font == run.font
                    && last.color == run.color
                    && last.underline == run.underline
                    && last.strikethrough == run.strikethrough
            }) {
                last.len += run.len;
            } else {
                runs.push(run);
            }
        }
        // Release the parser lock before expensive shaping/painting.
        drop(term);
        for (row, (text, runs)) in rows.into_iter().enumerate() {
            if !text.is_empty() {
                let line = window
                    .text_system()
                    .shape_line(text.into(), px(panel.font_size), &runs, Some(cw));
                state
                    .lines
                    .push((bounds.origin + point(px(0.), ch * row), line));
            }
        }
        state
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        state: &mut PaintedGrid,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for (rect, color) in &state.backgrounds {
                window.paint_quad(fill(*rect, *color));
            }
            for (origin, line) in &state.lines {
                if let Err(error) = line.paint(
                    *origin,
                    state.line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                ) {
                    eprintln!("Could not paint terminal text: {error}");
                }
            }
            if let Some((rect, color)) = state.cursor {
                window.paint_quad(fill(rect, color));
            }
        });
    }
}
/// Monospace families in order of preference. The Nerd Font build of JetBrains Mono
/// has real regular/bold/italic/bold-italic faces (the plain one is variable, with no
/// italic), so it comes first; the rest are common system monospace fonts.
const MONO_FAMILIES: &[&str] = &[
    "JetBrainsMono Nerd Font",
    "JetBrainsMono Nerd Font Mono",
    "JetBrains Mono",
    "DejaVu Sans Mono",
    "Liberation Mono",
    "Noto Sans Mono",
    "Ubuntu Mono",
    "Cascadia Mono",
    "Source Code Pro",
    "FreeMono",
];
/// The first installed monospace family. A missing family is not an error in GPUI:
/// it silently falls back to its proportional UI font, and the grid then shows
/// broken letter spacing ("roo t@ m:~#"). Looked up once per process.
fn terminal_font(window: &Window) -> Font {
    static FAMILY: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    let family = *FAMILY.get_or_init(|| pick_family(&window.text_system().all_font_names()));
    let mut font = font(family);
    font.features = FontFeatures::disable_ligatures();
    font
}
fn pick_family(installed: &[String]) -> &'static str {
    MONO_FAMILIES
        .iter()
        .copied()
        .find(|family| installed.iter().any(|name| name == family))
        .unwrap_or(MONO_FAMILIES[0])
}
fn to_rgb(color: Hsla) -> Rgb {
    let c = color.to_rgb();
    Rgb {
        r: (c.r * 255.).round() as u8,
        g: (c.g * 255.).round() as u8,
        b: (c.b * 255.).round() as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::pick_family;

    #[test]
    fn prefers_the_nerd_font_then_any_installed_mono() {
        let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(pick_family(&names(&["DejaVu Sans Mono", "JetBrainsMono Nerd Font"])), "JetBrainsMono Nerd Font");
        assert_eq!(pick_family(&names(&["DejaVu Sans", "DejaVu Sans Mono", "JetBrains Mono"])), "JetBrains Mono");
        assert_eq!(pick_family(&names(&["DejaVu Sans"])), "JetBrainsMono Nerd Font");
    }
}
