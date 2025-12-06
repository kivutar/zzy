use alacritty_terminal::{
    term::{
        cell::{Cell, Flags},
        color::{Colors, COUNT as COLOR_COUNT},
    },
    vte::ansi,
};
use gpui::Rgba;

pub(crate) struct Palette {
    resolved: Vec<Option<Rgba>>,
    pub(crate) default_fg: Rgba,
    pub(crate) default_bg: Rgba,
    base_colors: Colors,
}

impl Palette {
    pub(crate) fn new(colors: &Colors) -> Self {
        let resolved = (0..COLOR_COUNT)
            .map(|i| colors[i].map(to_rgba))
            .collect::<Vec<_>>();
        let mut palette = Self {
            resolved,
            default_fg: to_rgba(resolve_named_color(ansi::NamedColor::Foreground, colors)),
            default_bg: to_rgba(resolve_named_color(ansi::NamedColor::Background, colors)),
            base_colors: *colors,
        };
        palette.apply_theme_overrides();
        palette
    }

    pub(crate) fn resolve(&self, color: &ansi::Color) -> Rgba {
        match color {
            ansi::Color::Spec(rgb) => to_rgba(*rgb),
            ansi::Color::Indexed(idx) => self
                .resolved
                .get(*idx as usize)
                .and_then(|c| *c)
                .unwrap_or_else(|| resolve_indexed(*idx)),
            ansi::Color::Named(name) => self
                .resolved
                .get(*name as usize)
                .and_then(|c| *c)
                .unwrap_or_else(|| to_rgba(resolve_named_color(*name, &self.base_colors))),
        }
    }

    pub(crate) fn fg_bg(&self, cell: &Cell) -> (Rgba, Rgba) {
        let mut fg = self.resolve(&cell.fg);
        let mut bg = self.resolve(&cell.bg);
        if cell.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        (fg, bg)
    }

    pub(crate) fn cursor_bg(&self) -> Rgba {
        self.resolved
            .get(ansi::NamedColor::Cursor as usize)
            .and_then(|c| *c)
            .unwrap_or(self.default_fg)
    }

    pub(crate) fn cursor_color(&self) -> Rgba {
        self.default_bg
    }

    fn apply_theme_overrides(&mut self) {
        for &(name, hex) in HAMI_MELON_NAMED {
            self.set_named(name, hex);
        }
        self.default_fg = rgba_hex(THEME_FG);
        self.default_bg = rgba_hex(THEME_BG);
    }

    fn set_named(&mut self, name: ansi::NamedColor, hex: u32) {
        let idx = name as usize;
        if let Some(slot) = self.resolved.get_mut(idx) {
            *slot = Some(rgba_hex(hex));
        }
    }
}

fn resolve_indexed(idx: u8) -> Rgba {
    if idx < 16 {
        to_rgba(hex_to_rgb(HAMI_INDEXED[idx as usize]))
    } else if (16..=231).contains(&idx) {
        to_rgba(color_cube(idx))
    } else {
        to_rgba(grayscale(idx))
    }
}

fn resolve_named_color(name: ansi::NamedColor, palette: &Colors) -> ansi::Rgb {
    match palette[name] {
        Some(rgb) => rgb,
        None => hami_named_color(name).unwrap_or(hex_to_rgb(HAMI_INDEXED[(name as usize) % 16])),
    }
}

fn color_cube(idx: u8) -> ansi::Rgb {
    let idx = idx.saturating_sub(16);
    let r = idx / 36;
    let g = (idx / 6) % 6;
    let b = idx % 6;
    let level = |v: u8| -> u8 {
        if v == 0 {
            0
        } else {
            55 + v * 40
        }
    };
    ansi::Rgb {
        r: level(r),
        g: level(g),
        b: level(b),
    }
}

fn hex_to_rgb(hex: u32) -> ansi::Rgb {
    ansi::Rgb {
        r: ((hex >> 24) & 0xff) as u8,
        g: ((hex >> 16) & 0xff) as u8,
        b: ((hex >> 8) & 0xff) as u8,
    }
}

fn hami_named_color(name: ansi::NamedColor) -> Option<ansi::Rgb> {
    HAMI_MELON_NAMED
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, hex)| hex_to_rgb(*hex))
}

const HAMI_INDEXED: [u32; 16] = [
    0x282c33ff, // black
    0xD34949ff, // red
    0x88B292ff, // green
    0xCB9484ff, // yellow
    0x7490BEff, // blue
    0x9A5061ff, // magenta
    0x66A6B0ff, // cyan
    0xc8ccd4ff, // white
    0x525561ff, // bright black
    0xD34949ff, // bright red
    0x88B292ff, // bright green
    0xCB9484ff, // bright yellow
    0x7490BEff, // bright blue
    0x9A5061ff, // bright magenta
    0x66A6B0ff, // bright cyan
    0xc8ccd4ff, // bright white
];

fn grayscale(idx: u8) -> ansi::Rgb {
    let value = 8 + 10 * idx.saturating_sub(232);
    ansi::Rgb {
        r: value,
        g: value,
        b: value,
    }
}

fn to_rgba(rgb: ansi::Rgb) -> Rgba {
    Rgba {
        r: rgb.r as f32 / 255.0,
        g: rgb.g as f32 / 255.0,
        b: rgb.b as f32 / 255.0,
        a: 1.0,
    }
}

fn rgba_hex(hex: u32) -> Rgba {
    let r = ((hex >> 24) & 0xff) as f32 / 255.0;
    let g = ((hex >> 16) & 0xff) as f32 / 255.0;
    let b = ((hex >> 8) & 0xff) as f32 / 255.0;
    let a = (hex & 0xff) as f32 / 255.0;
    Rgba { r, g, b, a }
}

const THEME_BG: u32 = 0x00000000;
const THEME_FG: u32 = 0xccccccff;

const HAMI_MELON_NAMED: &[(ansi::NamedColor, u32)] = &[
    (ansi::NamedColor::Foreground, THEME_FG),
    (ansi::NamedColor::BrightForeground, 0xb3b3b3ff),
    (ansi::NamedColor::DimForeground, 0x282c33ff),
    (ansi::NamedColor::Background, THEME_BG),
    (ansi::NamedColor::Cursor, 0x5c78e2ff),
    (ansi::NamedColor::Black, 0x282c33ff),
    (ansi::NamedColor::BrightBlack, 0x525561ff),
    (ansi::NamedColor::DimBlack, 0xc8ccd4ff),
    (ansi::NamedColor::Red, 0xD34949ff),
    (ansi::NamedColor::BrightRed, 0xD34949ff),
    (ansi::NamedColor::DimRed, 0xeab7b9ff),
    (ansi::NamedColor::Green, 0x88B292ff),
    (ansi::NamedColor::BrightGreen, 0x88B292ff),
    (ansi::NamedColor::DimGreen, 0xd1e0bfff),
    (ansi::NamedColor::Yellow, 0xCB9484ff),
    (ansi::NamedColor::BrightYellow, 0xCB9484ff),
    (ansi::NamedColor::DimYellow, 0xf1dfc1ff),
    (ansi::NamedColor::Blue, 0x7490BEff),
    (ansi::NamedColor::BrightBlue, 0x7490BEff),
    (ansi::NamedColor::DimBlue, 0xbed5f4ff),
    (ansi::NamedColor::Magenta, 0x9A5061ff),
    (ansi::NamedColor::BrightMagenta, 0x9A5061ff),
    (ansi::NamedColor::DimMagenta, 0xe6a79eff),
    (ansi::NamedColor::Cyan, 0x66A6B0ff),
    (ansi::NamedColor::BrightCyan, 0x66A6B0ff),
    (ansi::NamedColor::DimCyan, 0xb9d9dfff),
    (ansi::NamedColor::White, 0xc8ccd4ff),
    (ansi::NamedColor::BrightWhite, 0xc8ccd4ff),
    (ansi::NamedColor::DimWhite, 0x575d65ff),
];
