//! Terminal application colours are data, not UI accents. Keep ANSI hues distinct.
use alacritty_terminal::{
    term::color::Colors,
    vte::ansi::{Color, NamedColor, Rgb},
};

pub fn indexed(index: usize) -> Rgb {
    const ANSI: [u32; 16] = [
        0x08090c, 0xf04b55, 0x8cbd83, 0xd2b774, 0x376bfa, 0xb18bc9, 0x79b8c5, 0xd4d7df, 0x939ba8,
        0xff7983, 0xa7d69e, 0xe5cd94, 0x759aff, 0xcda7e5, 0x99d5df, 0xffffff,
    ];
    let value = match index {
        0..=15 => ANSI[index],
        16..=231 => {
            let n = index - 16;
            let ramp = [0, 95, 135, 175, 215, 255];
            (ramp[n / 36] << 16) | (ramp[n / 6 % 6] << 8) | ramp[n % 6]
        }
        232..=255 => {
            let n = 8 + 10 * (index as u32 - 232);
            (n << 16) | (n << 8) | n
        }
        257 => 0x030405,
        258 => 0x376bfa,
        259..=266 => {
            let c = indexed(index - 259);
            return Rgb {
                r: c.r / 2,
                g: c.g / 2,
                b: c.b / 2,
            };
        }
        268 => 0x939ba8,
        _ => 0xd4d7df,
    };
    Rgb {
        r: (value >> 16) as u8,
        g: (value >> 8) as u8,
        b: value as u8,
    }
}

pub fn resolve(color: Color, colors: &Colors, foreground: Rgb, background: Rgb) -> Rgb {
    let index = match color {
        Color::Spec(value) => return value,
        Color::Indexed(index) => index as usize,
        Color::Named(name) => name as usize,
    };
    colors[index].unwrap_or_else(|| match color {
        Color::Named(NamedColor::Foreground) => foreground,
        Color::Named(NamedColor::Background) => background,
        _ => indexed(index),
    })
}

pub fn gpui(c: Rgb) -> gpui_kit::Hsla {
    gpui_kit::rgb((u32::from(c.r) << 16) | (u32::from(c.g) << 8) | u32::from(c.b)).into()
}
