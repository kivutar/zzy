use alacritty_terminal::{
    term::{
        cell::{Cell, Flags},
        color::{Colors, COUNT as COLOR_COUNT},
    },
    vte::ansi,
};
use gpui_kit::Rgba;

#[derive(Clone, Copy)]
pub(crate) struct ThemeOverrides {
    pub indexed: [u32; 16],
    pub named: &'static [(ansi::NamedColor, u32)],
    pub default_fg: u32,
    pub default_bg: u32,
    pub border: u32,
}

pub(crate) struct Palette {
    /// Indexed by `alacritty_terminal::term::color` slot: 0..=255 are the
    /// indexed colors, the rest are named colors (foreground, cursor, dims...).
    resolved: Vec<Option<Rgba>>,
    pub(crate) default_fg: Rgba,
    pub(crate) default_bg: Rgba,
    indexed: [u32; 16],
}

impl Palette {
    pub(crate) fn new(colors: &Colors, theme: &'static ThemeOverrides) -> Self {
        let mut palette = Self {
            resolved: (0..COLOR_COUNT).map(|i| colors[i].map(to_rgba)).collect(),
            default_fg: rgba_hex(theme.default_fg),
            default_bg: rgba_hex(theme.default_bg),
            indexed: theme.indexed,
        };
        palette.apply_theme_overrides(theme);
        palette
    }

    pub(crate) fn resolve(&self, color: &ansi::Color) -> Rgba {
        match color {
            ansi::Color::Spec(rgb) => to_rgba(*rgb),
            ansi::Color::Indexed(idx) => self
                .resolved
                .get(*idx as usize)
                .and_then(|c| *c)
                .unwrap_or_else(|| self.resolve_indexed(*idx)),
            // Every named slot is filled by `apply_theme_overrides`.
            ansi::Color::Named(name) => self
                .resolved
                .get(*name as usize)
                .and_then(|c| *c)
                .unwrap_or(self.default_fg),
        }
    }

    /// Color for text with the DIM attribute (SGR 2), following alacritty:
    /// named and base-8 colors use their dim palette entry, anything else is
    /// blended toward the background.
    fn resolve_dim(&self, color: &ansi::Color) -> Rgba {
        match color {
            ansi::Color::Named(name) => self.resolve(&ansi::Color::Named(name.to_dim())),
            ansi::Color::Indexed(idx) if (*idx as usize) < DIMS.len() => {
                self.resolve(&ansi::Color::Named(DIMS[*idx as usize]))
            }
            _ => dim(self.resolve(color), self.default_bg),
        }
    }

    /// "Bold is bright": the 8 base colors (and the foreground) switch to
    /// their bright variant.
    fn resolve_bright(&self, color: &ansi::Color) -> Rgba {
        match color {
            ansi::Color::Named(name) => self.resolve(&ansi::Color::Named(name.to_bright())),
            ansi::Color::Indexed(idx) if *idx < 8 => self.resolve(&ansi::Color::Indexed(idx + 8)),
            _ => self.resolve(color),
        }
    }

    /// The color in palette slot `index` (0..=255 indexed, then the named
    /// colors), as reported to programs that query it (OSC 4/10/11/12).
    pub(crate) fn rgb_for_slot(&self, index: usize) -> ansi::Rgb {
        let color = match u8::try_from(index) {
            Ok(idx) => self.resolve(&ansi::Color::Indexed(idx)),
            Err(_) => self
                .resolved
                .get(index)
                .and_then(|c| *c)
                .unwrap_or(self.default_fg),
        };
        let channel = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        ansi::Rgb {
            r: channel(color.r),
            g: channel(color.g),
            b: channel(color.b),
        }
    }

    pub(crate) fn fg_bg(&self, cell: &Cell, bold_is_bright: bool) -> (Rgba, Rgba) {
        let mut fg = if cell.flags.contains(Flags::DIM) {
            self.resolve_dim(&cell.fg)
        } else if bold_is_bright && cell.flags.contains(Flags::BOLD) {
            self.resolve_bright(&cell.fg)
        } else {
            self.resolve(&cell.fg)
        };
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

    fn apply_theme_overrides(&mut self, theme: &ThemeOverrides) {
        use ansi::NamedColor::*;

        let bg = rgba_hex(theme.default_bg);
        for name in [Foreground, BrightForeground, Cursor] {
            self.set_named(name, theme.default_fg);
        }
        self.set_named(Background, theme.default_bg);
        // Dim variants default to the base colors faded toward the background.
        for (name, &hex) in DIMS.into_iter().zip(&theme.indexed) {
            self.resolved[name as usize] = Some(dim(rgba_hex(hex), bg));
        }
        self.resolved[DimForeground as usize] = Some(dim(rgba_hex(theme.default_fg), bg));
        // Theme-specific colors win over the defaults above.
        for &(name, hex) in theme.named {
            self.set_named(name, hex);
        }
        // The base 16 always follow `indexed`. Named colors Black..=BrightWhite
        // share slots 0..16 with the indexed palette.
        for (slot, &hex) in theme.indexed.iter().enumerate() {
            self.resolved[slot] = Some(rgba_hex(hex));
        }
    }

    fn set_named(&mut self, name: ansi::NamedColor, hex: u32) {
        self.resolved[name as usize] = Some(rgba_hex(hex));
    }

    fn resolve_indexed(&self, idx: u8) -> Rgba {
        match idx {
            0..16 => rgba_hex(self.indexed[idx as usize]),
            16..=231 => to_rgba(color_cube(idx)),
            _ => to_rgba(grayscale(idx)),
        }
    }
}

const DIMS: [ansi::NamedColor; 8] = [
    ansi::NamedColor::DimBlack,
    ansi::NamedColor::DimRed,
    ansi::NamedColor::DimGreen,
    ansi::NamedColor::DimYellow,
    ansi::NamedColor::DimBlue,
    ansi::NamedColor::DimMagenta,
    ansi::NamedColor::DimCyan,
    ansi::NamedColor::DimWhite,
];

/// How much of the original color a dimmed color keeps (alacritty uses 0.66).
const DIM_FACTOR: f32 = 0.66;

fn dim(color: Rgba, bg: Rgba) -> Rgba {
    let mix = |c: f32, b: f32| c * DIM_FACTOR + b * (1.0 - DIM_FACTOR);
    Rgba {
        r: mix(color.r, bg.r),
        g: mix(color.g, bg.g),
        b: mix(color.b, bg.b),
        a: color.a,
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

pub(crate) fn rgba_hex(hex: u32) -> Rgba {
    let r = ((hex >> 24) & 0xff) as f32 / 255.0;
    let g = ((hex >> 16) & 0xff) as f32 / 255.0;
    let b = ((hex >> 8) & 0xff) as f32 / 255.0;
    let a = (hex & 0xff) as f32 / 255.0;
    Rgba { r, g, b, a }
}

const DEFAULT_THEME: ThemeOverrides = ThemeOverrides {
    indexed: [
        0x282c33ff, 0xd34949ff, 0x88b292ff, 0xcb9484ff, 0x7490beff, 0x9a5061ff, 0x66a6b0ff,
        0xc8ccd4ff, 0x525561ff, 0xd34949ff, 0x88b292ff, 0xcb9484ff, 0x7490beff, 0x9a5061ff,
        0x66a6b0ff, 0xc8ccd4ff,
    ],
    named: &[
        (ansi::NamedColor::Foreground, 0xccccccff),
        (ansi::NamedColor::BrightForeground, 0xb3b3b3ff),
        (ansi::NamedColor::Cursor, 0x5c78e2ff),
        (ansi::NamedColor::Black, 0x282c33ff),
        (ansi::NamedColor::BrightBlack, 0x525561ff),
        (ansi::NamedColor::Red, 0xd34949ff),
        (ansi::NamedColor::BrightRed, 0xd34949ff),
        (ansi::NamedColor::Green, 0x88b292ff),
        (ansi::NamedColor::BrightGreen, 0x88b292ff),
        (ansi::NamedColor::Yellow, 0xcb9484ff),
        (ansi::NamedColor::BrightYellow, 0xcb9484ff),
        (ansi::NamedColor::Blue, 0x7490beff),
        (ansi::NamedColor::BrightBlue, 0x7490beff),
        (ansi::NamedColor::Magenta, 0x9a5061ff),
        (ansi::NamedColor::BrightMagenta, 0x9a5061ff),
        (ansi::NamedColor::Cyan, 0x66a6b0ff),
        (ansi::NamedColor::BrightCyan, 0x66a6b0ff),
        (ansi::NamedColor::White, 0xc8ccd4ff),
        (ansi::NamedColor::BrightWhite, 0xc8ccd4ff),
    ],
    default_fg: 0xccccccff,
    default_bg: 0x161616ff,
    border: 0x2a2a2aff,
};

const ROSE_PINE: ThemeOverrides = ThemeOverrides {
    indexed: [
        0x191724ff, 0xeb6f92ff, 0x31748fff, 0xf6c177ff, 0x9ccfd8ff, 0xc4a7e7ff, 0xebbcabff,
        0xe0def4ff, 0x26233aff, 0xeb6f92ff, 0x31748fff, 0xf6c177ff, 0x9ccfd8ff, 0xc4a7e7ff,
        0xebbcabff, 0xe0def4ff,
    ],
    named: &[
        (ansi::NamedColor::Foreground, 0xe0def4ff),
        (ansi::NamedColor::BrightForeground, 0xe0def4ff),
        (ansi::NamedColor::DimForeground, 0x6e6a86ff),
        (ansi::NamedColor::Background, 0x191724ff),
        (ansi::NamedColor::Cursor, 0xe0def4ff),
    ],
    default_fg: 0xe0def4ff,
    default_bg: 0x191724ff,
    border: 0x1c1c2eff,
};

const DRACULA: ThemeOverrides = ThemeOverrides {
    indexed: [
        0x282a36ff, 0xff5555ff, 0x50fa7bff, 0xf1fa8cff, 0xbd93f9ff, 0xff79c6ff, 0x8be9fdff,
        0xf8f8f2ff, 0x44475aff, 0xff5555ff, 0x50fa7bff, 0xf1fa8cff, 0xbd93f9ff, 0xff79c6ff,
        0x8be9fdff, 0xf8f8f2ff,
    ],
    named: &[
        (ansi::NamedColor::Foreground, 0xf8f8f2ff),
        (ansi::NamedColor::BrightForeground, 0xf8f8f2ff),
        (ansi::NamedColor::DimForeground, 0x6272a4ff),
        (ansi::NamedColor::Background, 0x282a36ff),
        (ansi::NamedColor::Cursor, 0xf8f8f2ff),
    ],
    default_fg: 0xf8f8f2ff,
    default_bg: 0x282a36ff,
    border: 0x383a44ff,
};

const GRUVBOX_DARK: ThemeOverrides = ThemeOverrides {
    indexed: [
        0x282828ff, 0xcc241dff, 0x98971aff, 0xd79921ff, 0x458588ff, 0xb16286ff, 0x689d6aff,
        0xa89984ff, 0x3c3836ff, 0xfb4934ff, 0xb8bb26ff, 0xfabd2fff, 0x83a598ff, 0xd3869bff,
        0x8ec07cff, 0xebdbb2ff,
    ],
    named: &[
        (ansi::NamedColor::Foreground, 0xebdbb2ff),
        (ansi::NamedColor::BrightForeground, 0xebdbb2ff),
        (ansi::NamedColor::DimForeground, 0xa89984ff),
        (ansi::NamedColor::Background, 0x282828ff),
        (ansi::NamedColor::Cursor, 0xebdbb2ff),
    ],
    default_fg: 0xebdbb2ff,
    default_bg: 0x282828ff,
    border: 0x5c534dff,
};

const AYU_DARK: ThemeOverrides = ThemeOverrides {
    indexed: [
        0x0f1419ff, 0xf07178ff, 0x86b300ff, 0xf2ae49ff, 0x36a3d9ff, 0xa37accff, 0x4cbf99ff,
        0xe6e1cfff, 0x131721ff, 0xf07178ff, 0x86b300ff, 0xf2ae49ff, 0x36a3d9ff, 0xa37accff,
        0x4cbf99ff, 0xfff9e5ff,
    ],
    named: &[
        (ansi::NamedColor::Foreground, 0xe6e1cfff),
        (ansi::NamedColor::BrightForeground, 0xfff9e5ff),
        (ansi::NamedColor::DimForeground, 0x5c6773ff),
        (ansi::NamedColor::Background, 0x0f1419ff),
        (ansi::NamedColor::Cursor, 0xf2ae49ff),
    ],
    default_fg: 0xe6e1cfff,
    default_bg: 0x0f1419ff,
    border: 0x404042ff,
};

pub(crate) fn palette_for(theme: crate::theme::ThemeChoice) -> &'static ThemeOverrides {
    match theme {
        crate::theme::ThemeChoice::RosePine => &ROSE_PINE,
        crate::theme::ThemeChoice::Dracula => &DRACULA,
        crate::theme::ThemeChoice::GruvboxDark => &GRUVBOX_DARK,
        crate::theme::ThemeChoice::AyuDark => &AYU_DARK,
        crate::theme::ThemeChoice::HamiMelonDark => &DEFAULT_THEME,
    }
}

pub(crate) fn theme_background(theme: crate::theme::ThemeChoice) -> Rgba {
    rgba_hex(palette_for(theme).default_bg)
}

pub(crate) fn theme_foreground(theme: crate::theme::ThemeChoice) -> Rgba {
    rgba_hex(palette_for(theme).default_fg)
}

pub(crate) fn theme_border(theme: crate::theme::ThemeChoice) -> Rgba {
    rgba_hex(palette_for(theme).border)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeChoice;

    fn cell(fg: ansi::Color, flags: Flags) -> Cell {
        Cell {
            fg,
            flags,
            ..Cell::default()
        }
    }

    #[test]
    fn bold_is_bright() {
        let theme = palette_for(ThemeChoice::Dracula);
        let palette = Palette::new(&Colors::default(), theme);
        let red = ansi::Color::Named(ansi::NamedColor::Red);
        let bright_red = rgba_hex(theme.indexed[9]);
        let bold_red = cell(red, Flags::BOLD);

        assert_eq!(palette.fg_bg(&bold_red, true).0, bright_red);
        assert_eq!(
            palette.fg_bg(&bold_red, false).0,
            rgba_hex(theme.indexed[1])
        );
        assert_eq!(
            palette
                .fg_bg(&cell(ansi::Color::Indexed(1), Flags::BOLD), true)
                .0,
            bright_red
        );
        // 256-color and truecolor are left alone.
        let orange = ansi::Color::Indexed(208);
        assert_eq!(
            palette.fg_bg(&cell(orange, Flags::BOLD), true).0,
            palette.resolve(&orange)
        );
    }
}
