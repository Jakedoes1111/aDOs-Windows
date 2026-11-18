//! Inline monochrome icons: the kit's minimal default asset pack omits these.
//! Raw SVGs keep the palette independent of the shell's asset-source selection.
use gpui_kit::*;

const ICONS: [&[u8]; 4] = [
    br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M21 11a8 8 0 0 1-8 8H9l-5 3v-6a8 8 0 0 1 7-13h2a8 8 0 0 1 8 8Z"/></svg>"#,
    br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H5v20h14V7Z M14 2v6h5"/></svg>"#,
    br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="8" cy="8" r="1.4"/><path d="m3 17 5-5 4 4 4-6 5 7"/></svg>"#,
    br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="m10 13 4-4 M8 10l-3 3a4 4 0 0 0 6 6l3-3 M10 8l3-3a4 4 0 0 1 6 6l-3 3"/></svg>"#,
];

pub(crate) fn footer(index: usize, color: Hsla) -> Svg {
    svg().data(ICONS[index]).size(px(19.)).text_color(color)
}
