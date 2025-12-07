use alacritty_terminal::{
    term::{
        cell::{Cell, Flags},
        color::{Colors, COUNT as COLOR_COUNT},
    },
    vte::ansi,
};
use gpui::Rgba;

#[derive(Clone, Copy)]
pub(crate) struct ThemeOverrides {
    pub indexed: [u32; 16],
    pub named: &'static [(ansi::NamedColor, u32)],
    pub default_fg: u32,
    pub default_bg: u32,
}

pub(crate) struct Palette {
    resolved: Vec<Option<Rgba>>,
    pub(crate) default_fg: Rgba,
    pub(crate) default_bg: Rgba,
    base_colors: Colors,
    indexed: [u32; 16],
    theme: &'static ThemeOverrides,
}

impl Palette {
    pub(crate) fn new(colors: &Colors, theme: &'static ThemeOverrides) -> Self {
        let resolved = (0..COLOR_COUNT)
            .map(|i| colors[i].map(to_rgba))
            .collect::<Vec<_>>();
        let mut palette = Self {
            resolved,
            default_fg: to_rgba(resolve_named_color(
                ansi::NamedColor::Foreground,
                colors,
                theme,
            )),
            default_bg: to_rgba(resolve_named_color(
                ansi::NamedColor::Background,
                colors,
                theme,
            )),
            base_colors: *colors,
            indexed: theme.indexed,
            theme,
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
            ansi::Color::Named(name) => self
                .resolved
                .get(*name as usize)
                .and_then(|c| *c)
                .unwrap_or_else(|| {
                    to_rgba(resolve_named_color(*name, &self.base_colors, self.theme))
                }),
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

    fn apply_theme_overrides(&mut self, theme: &ThemeOverrides) {
        for &(name, hex) in theme.named {
            self.set_named(name, hex);
        }
        // Ensure all standard named colors are populated from the theme's indexed palette.
        for (idx, name) in [
            ansi::NamedColor::Black,
            ansi::NamedColor::Red,
            ansi::NamedColor::Green,
            ansi::NamedColor::Yellow,
            ansi::NamedColor::Blue,
            ansi::NamedColor::Magenta,
            ansi::NamedColor::Cyan,
            ansi::NamedColor::White,
            ansi::NamedColor::BrightBlack,
            ansi::NamedColor::BrightRed,
            ansi::NamedColor::BrightGreen,
            ansi::NamedColor::BrightYellow,
            ansi::NamedColor::BrightBlue,
            ansi::NamedColor::BrightMagenta,
            ansi::NamedColor::BrightCyan,
            ansi::NamedColor::BrightWhite,
        ]
        .into_iter()
        .enumerate()
        {
            self.set_named(name, theme.indexed[idx]);
        }
        // Dim variants fallback to base colors for now.
        for name in [
            ansi::NamedColor::DimBlack,
            ansi::NamedColor::DimRed,
            ansi::NamedColor::DimGreen,
            ansi::NamedColor::DimYellow,
            ansi::NamedColor::DimBlue,
            ansi::NamedColor::DimMagenta,
            ansi::NamedColor::DimCyan,
            ansi::NamedColor::DimWhite,
            ansi::NamedColor::DimForeground,
        ] {
            let base = match name {
                ansi::NamedColor::DimBlack => theme.indexed[0],
                ansi::NamedColor::DimRed => theme.indexed[1],
                ansi::NamedColor::DimGreen => theme.indexed[2],
                ansi::NamedColor::DimYellow => theme.indexed[3],
                ansi::NamedColor::DimBlue => theme.indexed[4],
                ansi::NamedColor::DimMagenta => theme.indexed[5],
                ansi::NamedColor::DimCyan => theme.indexed[6],
                ansi::NamedColor::DimWhite => theme.indexed[7],
                ansi::NamedColor::DimForeground => theme.default_fg,
                _ => theme.default_fg,
            };
            self.set_named(name, base);
        }
        self.default_fg = rgba_hex(theme.default_fg);
        self.default_bg = rgba_hex(theme.default_bg);
    }

    fn set_named(&mut self, name: ansi::NamedColor, hex: u32) {
        let idx = name as usize;
        if let Some(slot) = self.resolved.get_mut(idx) {
            *slot = Some(rgba_hex(hex));
        }
    }

    fn resolve_indexed(&self, idx: u8) -> Rgba {
        if idx < 16 {
            to_rgba(hex_to_rgb(self.indexed[idx as usize]))
        } else if (16..=231).contains(&idx) {
            to_rgba(color_cube(idx))
        } else {
            to_rgba(grayscale(idx))
        }
    }
}

fn resolve_named_color(
    name: ansi::NamedColor,
    palette: &Colors,
    theme: &ThemeOverrides,
) -> ansi::Rgb {
    match palette[name] {
        Some(rgb) => rgb,
        None => {
            // Try theme specific named overrides.
            if let Some(hex) = theme
                .named
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, hex)| *hex)
            {
                return hex_to_rgb(hex);
            }
            // Map standard names to indexed palette.
            let idx = match name {
                ansi::NamedColor::Black => Some(0),
                ansi::NamedColor::Red => Some(1),
                ansi::NamedColor::Green => Some(2),
                ansi::NamedColor::Yellow => Some(3),
                ansi::NamedColor::Blue => Some(4),
                ansi::NamedColor::Magenta => Some(5),
                ansi::NamedColor::Cyan => Some(6),
                ansi::NamedColor::White => Some(7),
                ansi::NamedColor::BrightBlack => Some(8),
                ansi::NamedColor::BrightRed => Some(9),
                ansi::NamedColor::BrightGreen => Some(10),
                ansi::NamedColor::BrightYellow => Some(11),
                ansi::NamedColor::BrightBlue => Some(12),
                ansi::NamedColor::BrightMagenta => Some(13),
                ansi::NamedColor::BrightCyan => Some(14),
                ansi::NamedColor::BrightWhite => Some(15),
                ansi::NamedColor::Foreground => return hex_to_rgb(theme.default_fg),
                ansi::NamedColor::BrightForeground => return hex_to_rgb(theme.default_fg),
                ansi::NamedColor::DimForeground => return hex_to_rgb(theme.default_fg),
                ansi::NamedColor::Background => return hex_to_rgb(theme.default_bg),
                ansi::NamedColor::Cursor => return hex_to_rgb(theme.default_fg),
                ansi::NamedColor::DimBlack => Some(0),
                ansi::NamedColor::DimRed => Some(1),
                ansi::NamedColor::DimGreen => Some(2),
                ansi::NamedColor::DimYellow => Some(3),
                ansi::NamedColor::DimBlue => Some(4),
                ansi::NamedColor::DimMagenta => Some(5),
                ansi::NamedColor::DimCyan => Some(6),
                ansi::NamedColor::DimWhite => Some(7),
            };

            if let Some(idx) = idx {
                hex_to_rgb(theme.indexed[idx])
            } else {
                hex_to_rgb(theme.indexed[(name as usize) % 16])
            }
        }
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
        0x282c33ff, 0xD34949ff, 0x88B292ff, 0xCB9484ff, 0x7490BEff, 0x9A5061ff, 0x66A6B0ff,
        0xc8ccd4ff, 0x525561ff, 0xD34949ff, 0x88B292ff, 0xCB9484ff, 0x7490BEff, 0x9A5061ff,
        0x66A6B0ff, 0xc8ccd4ff,
    ],
    named: &[
        (ansi::NamedColor::Foreground, 0xccccccff),
        (ansi::NamedColor::BrightForeground, 0xb3b3b3ff),
        (ansi::NamedColor::DimForeground, 0x282c33ff),
        (ansi::NamedColor::Background, 0x00000000),
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
    ],
    default_fg: 0xccccccff,
    default_bg: 0x161616ff,
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
};

const GITHUB_DARK: ThemeOverrides = ThemeOverrides {
    indexed: [
        0x0d1117ff, 0xff7b72ff, 0x3fb950ff, 0xd29922ff, 0x58a6ffff, 0xbc8cffff, 0x39c5cfff,
        0xc9d1d9ff, 0x6e7681ff, 0xff7b72ff, 0x3fb950ff, 0xe3b341ff, 0x79c0ffff, 0xd2a8ffff,
        0x56d4ddff, 0xf0f6fcff,
    ],
    named: &[
        (ansi::NamedColor::Foreground, 0xc9d1d9ff),
        (ansi::NamedColor::BrightForeground, 0xf0f6fcff),
        (ansi::NamedColor::DimForeground, 0x6e7681ff),
        (ansi::NamedColor::Background, 0x0d1117ff),
        (ansi::NamedColor::Cursor, 0x58a6ffff),
    ],
    default_fg: 0xc9d1d9ff,
    default_bg: 0x0d1117ff,
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
};

pub(crate) fn palette_for(theme: crate::theme::ThemeChoice) -> &'static ThemeOverrides {
    match theme {
        crate::theme::ThemeChoice::RosePine => &ROSE_PINE,
        crate::theme::ThemeChoice::Dracula => &DRACULA,
        crate::theme::ThemeChoice::GithubDark => &GITHUB_DARK,
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
