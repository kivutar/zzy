use crate::i18n::tr;
use crate::terminal::DEFAULT_FONT_SIZE;
use crate::theme::ThemeChoice;
use gpui_kit::component::group_box::GroupBoxVariant;
use gpui_kit::component::setting::{
    NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage,
    Settings as SettingsPanel,
};
use gpui_kit::component::v_flex;
use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, Styled, Window,
};
use serde::{Deserialize, Serialize};

pub const DEFAULT_FONT_FAMILY: &str = "Adwaita Mono";

/// Saved as `settings.toml` (see `config.rs`); missing keys use the defaults.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct AppSettings {
    pub font_size: f32,
    pub cursor_blink: bool,
    pub cursor_shape: CursorShapeSetting,
    pub font_family: SharedString,
    pub bold_text: BoldText,
    pub scrollback: usize,
    pub scrollbar: ScrollbarVisibility,
    /// Light or dark, or following the desktop.
    pub appearance: Appearance,
    /// Used when the appearance is dark. Read from `theme` in older settings,
    /// which only had dark themes.
    #[serde(alias = "theme")]
    pub dark_theme: ThemeChoice,
    pub light_theme: ThemeChoice,
    /// Window background opacity, 0.0..=1.0.
    pub background_opacity: f32,
    pub background_blur: bool,
    /// Command for new tabs; empty means the user's shell.
    pub command: SharedString,
    /// Start the default shell as a login shell (`-l`).
    pub login_shell: bool,
    pub new_tab_directory: NewTabDirectory,
    /// Space around the terminal content, in pixels.
    pub padding: f32,
    /// Multiplier for the font's natural line height.
    pub line_height: f32,
    pub bell: BellMode,
    /// Show the program name in tabs instead of titles set by programs.
    pub ignore_program_title: bool,
    /// Tint the window background red while the program runs as root.
    pub tint_root: bool,
    /// Tint the window background green during SSH sessions.
    pub tint_ssh: bool,
    /// Language of the interface.
    pub language: Language,
    /// Show the tabs in the title bar instead of a bar of their own.
    pub tabs_in_title_bar: bool,
    /// Keep the window border when maximized, fullscreen or snapped.
    pub border_when_maximized: bool,
    /// Width of the window border, in pixels.
    pub border_width: f32,
    /// Minimum APCA contrast between text and its background (0: off): text
    /// too close to it is made lighter or darker. Same measure and default as
    /// Zed's terminal, which also spares colors a program picked exactly (true
    /// color, 256-color 16..) and box-drawing/Powerline characters; not yet here.
    pub minimum_contrast: f32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            font_size: DEFAULT_FONT_SIZE,
            cursor_blink: true,
            cursor_shape: CursorShapeSetting::Block,
            font_family: DEFAULT_FONT_FAMILY.into(),
            bold_text: BoldText::BoldFont,
            scrollback: 1000,
            scrollbar: ScrollbarVisibility::Auto,
            appearance: Appearance::System,
            dark_theme: ThemeChoice::HamiMelonDark,
            light_theme: ThemeChoice::HamiMelonLight,
            language: Language::System,
            background_opacity: 1.0,
            background_blur: true,
            command: SharedString::default(),
            login_shell: false,
            new_tab_directory: NewTabDirectory::Current,
            padding: 8.0,
            line_height: 1.0,
            bell: BellMode::Sound,
            ignore_program_title: false,
            tint_root: true,
            tint_ssh: true,
            tabs_in_title_bar: false,
            border_when_maximized: false,
            border_width: 1.0,
            minimum_contrast: 45.0,
        }
    }
}

impl AppSettings {
    /// Clamp values (e.g. from a hand-edited file) to what the UI allows.
    /// The theme to use, given whether the desktop prefers dark.
    pub fn theme(&self, system_dark: bool) -> ThemeChoice {
        let dark = match self.appearance {
            Appearance::System => system_dark,
            Appearance::Dark => true,
            Appearance::Light => false,
        };
        if dark {
            self.dark_theme
        } else {
            self.light_theme
        }
    }

    pub fn sanitized(mut self) -> Self {
        let clamp =
            |v: f64, (min, max): (f64, f64)| if v.is_nan() { min } else { v.clamp(min, max) };
        self.font_size = clamp(self.font_size as f64, FONT_SIZE_RANGE) as f32;
        self.padding = clamp(self.padding as f64, PADDING_RANGE) as f32;
        self.minimum_contrast = clamp(self.minimum_contrast as f64, CONTRAST_RANGE) as f32;
        self.border_width = clamp(self.border_width as f64, BORDER_WIDTH_RANGE).round() as f32;
        self.line_height = clamp(self.line_height as f64, LINE_HEIGHT_RANGE) as f32;
        self.scrollback = clamp(self.scrollback as f64, SCROLLBACK_RANGE) as usize;
        self.background_opacity =
            (clamp(self.background_opacity as f64 * 100.0, OPACITY_RANGE) / 100.0) as f32;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CursorShapeSetting {
    Block,
    Beam,
    Underline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScrollbarVisibility {
    Always,
    Auto,
    Never,
}

/// What happens when a program rings the bell (BEL).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BellMode {
    /// The desktop's system bell (xdg-system-bell), per its sound settings.
    Sound,
    /// Briefly tint the terminal.
    Flash,
    SoundAndFlash,
    Off,
}

impl BellMode {
    pub fn sound(self) -> bool {
        matches!(self, Self::Sound | Self::SoundAndFlash)
    }

    pub fn flash(self) -> bool {
        matches!(self, Self::Flash | Self::SoundAndFlash)
    }
}

/// How text with the bold attribute (SGR 1) is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BoldText {
    BoldFont,
    /// The 8 base colors switch to their bright variant, regular weight.
    BrightColors,
    BoldFontAndBrightColors,
    Normal,
}

impl BoldText {
    pub fn bold_font(self) -> bool {
        matches!(self, Self::BoldFont | Self::BoldFontAndBrightColors)
    }

    pub fn bright_colors(self) -> bool {
        matches!(self, Self::BrightColors | Self::BoldFontAndBrightColors)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NewTabDirectory {
    /// The directory of the shell in the current tab.
    Current,
    Home,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Appearance {
    /// Follow the desktop's light or dark preference.
    System,
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    /// Follow the system locale.
    #[serde(rename = "system")]
    System,
    #[serde(rename = "en")]
    English,
    #[serde(rename = "ja")]
    Japanese,
}

/// A dropdown entry's text: translated, or a name shown as is.
#[derive(Clone, Copy)]
enum Label {
    Key(&'static str),
    Name(&'static str),
}

impl Label {
    fn text(self) -> SharedString {
        match self {
            Self::Key(key) => tr(key),
            Self::Name(name) => name.into(),
        }
    }
}

/// Dropdown entries: (value, key, label), in display order.
type Choices<T> = &'static [(T, &'static str, Label)];

/// Language names are always written in their own language.
const LANGUAGES: Choices<Language> = &[
    (
        Language::System,
        "system",
        Label::Key("choice.system_language"),
    ),
    (Language::English, "en", Label::Name("English")),
    (Language::Japanese, "ja", Label::Name("日本語")),
];

const DARK_THEMES: Choices<ThemeChoice> = &[
    (ThemeChoice::RosePine, "rose-pine", Label::Name("Rosé Pine")),
    (ThemeChoice::Dracula, "dracula", Label::Name("Dracula")),
    (
        ThemeChoice::GruvboxDark,
        "gruvbox-dark",
        Label::Name("Gruvbox Dark"),
    ),
    (
        ThemeChoice::HamiMelonDark,
        "hami-melon-dark",
        Label::Name("Hami Melon Dark"),
    ),
    (ThemeChoice::AyuDark, "ayu-dark", Label::Name("Ayu Dark")),
];

const LIGHT_THEMES: Choices<ThemeChoice> = &[
    (
        ThemeChoice::RosePineDawn,
        "rose-pine-dawn",
        Label::Name("Rosé Pine Dawn"),
    ),
    (ThemeChoice::Alucard, "alucard", Label::Name("Alucard")),
    (
        ThemeChoice::GruvboxLight,
        "gruvbox-light",
        Label::Name("Gruvbox Light"),
    ),
    (
        ThemeChoice::HamiMelonLight,
        "hami-melon-light",
        Label::Name("Hami Melon Light"),
    ),
    (ThemeChoice::AyuLight, "ayu-light", Label::Name("Ayu Light")),
];

const APPEARANCES: Choices<Appearance> = &[
    (
        Appearance::System,
        "system",
        Label::Key("choice.system_appearance"),
    ),
    (Appearance::Dark, "dark", Label::Key("choice.dark")),
    (Appearance::Light, "light", Label::Key("choice.light")),
];

const CURSOR_SHAPES: Choices<CursorShapeSetting> = &[
    (
        CursorShapeSetting::Block,
        "block",
        Label::Key("choice.block"),
    ),
    (CursorShapeSetting::Beam, "beam", Label::Key("choice.beam")),
    (
        CursorShapeSetting::Underline,
        "underline",
        Label::Key("choice.underline"),
    ),
];

const SCROLLBARS: Choices<ScrollbarVisibility> = &[
    (ScrollbarVisibility::Auto, "auto", Label::Key("choice.auto")),
    (
        ScrollbarVisibility::Always,
        "always",
        Label::Key("choice.always"),
    ),
    (
        ScrollbarVisibility::Never,
        "never",
        Label::Key("choice.never"),
    ),
];

const BELL_MODES: Choices<BellMode> = &[
    (BellMode::Sound, "sound", Label::Key("choice.sound")),
    (BellMode::Flash, "flash", Label::Key("choice.flash")),
    (
        BellMode::SoundAndFlash,
        "sound-and-flash",
        Label::Key("choice.sound_and_flash"),
    ),
    (BellMode::Off, "off", Label::Key("choice.off")),
];

const BOLD_TEXT: Choices<BoldText> = &[
    (
        BoldText::BoldFont,
        "bold-font",
        Label::Key("choice.bold_font"),
    ),
    (
        BoldText::BrightColors,
        "bright-colors",
        Label::Key("choice.bright_colors"),
    ),
    (
        BoldText::BoldFontAndBrightColors,
        "bold-font-and-bright-colors",
        Label::Key("choice.bold_font_and_bright_colors"),
    ),
    (BoldText::Normal, "normal", Label::Key("choice.normal")),
];

const NEW_TAB_DIRECTORIES: Choices<NewTabDirectory> = &[
    (
        NewTabDirectory::Current,
        "current",
        Label::Key("choice.current_tab_directory"),
    ),
    (NewTabDirectory::Home, "home", Label::Key("choice.home")),
];

pub(crate) const FONT_SIZE_RANGE: (f64, f64) = (8.0, 72.0);
const PADDING_RANGE: (f64, f64) = (0.0, 48.0);
/// APCA Lc: 0 is off, 45 (the default, as in Zed) suits body text, 75 and up
/// is high contrast.
const CONTRAST_RANGE: (f64, f64) = (0.0, 100.0);
/// gpui-kit always draws a 1px border, so it can't go below that.
const BORDER_WIDTH_RANGE: (f64, f64) = (1.0, 10.0);
const LINE_HEIGHT_RANGE: (f64, f64) = (1.0, 2.0);
const SCROLLBACK_RANGE: (f64, f64) = (0.0, 50_000.0);
/// In percent. Below ~10% the window becomes hard to find.
const OPACITY_RANGE: (f64, f64) = (10.0, 100.0);

pub struct SettingsView {
    focus_handle: FocusHandle,
    settings: AppSettings,
}

impl SettingsView {
    pub fn new(cx: &mut Context<Self>, settings: AppSettings) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            settings,
        }
    }

    pub fn current_settings(&self) -> AppSettings {
        self.settings.clone()
    }
}

/// Getter/setter pair bound to one `AppSettings` field. The setter only
/// notifies (and so only re-applies settings) when the value changes.
fn bind<T: PartialEq + 'static>(
    view: &Entity<SettingsView>,
    get: fn(&AppSettings) -> T,
    set: fn(&mut AppSettings, T),
) -> (impl Fn(&App) -> T + 'static, impl Fn(T, &mut App) + 'static) {
    let reader = view.clone();
    let writer = view.clone();
    (
        move |cx: &App| get(&reader.read(cx).settings),
        move |value: T, cx: &mut App| {
            writer.update(cx, |view, cx| {
                if get(&view.settings) != value {
                    set(&mut view.settings, value);
                    cx.notify();
                }
            })
        },
    )
}

fn switch(
    view: &Entity<SettingsView>,
    get: fn(&AppSettings) -> bool,
    set: fn(&mut AppSettings, bool),
) -> SettingField<bool> {
    let (read, write) = bind(view, get, set);
    SettingField::switch(read, write).default_value(get(&AppSettings::default()))
}

fn text(
    view: &Entity<SettingsView>,
    get: fn(&AppSettings) -> SharedString,
    set: fn(&mut AppSettings, SharedString),
) -> SettingField<SharedString> {
    let (read, write) = bind(view, get, set);
    SettingField::input(read, write).default_value(get(&AppSettings::default()))
}

/// A number field; values are clamped to `range` before `set` sees them.
fn number(
    view: &Entity<SettingsView>,
    (min, max): (f64, f64),
    step: f64,
    get: fn(&AppSettings) -> f64,
    set: fn(&mut AppSettings, f64),
) -> SettingField<f64> {
    let (read, write) = bind(view, get, set);
    SettingField::number_input(
        NumberFieldOptions { min, max, step },
        read,
        move |value: f64, cx: &mut App| write(value.clamp(min, max), cx),
    )
    .default_value(get(&AppSettings::default()))
}

fn dropdown<T: Copy + PartialEq + 'static>(
    view: &Entity<SettingsView>,
    choices: Choices<T>,
    get: fn(&AppSettings) -> T,
    set: fn(&mut AppSettings, T),
) -> SettingField<SharedString> {
    let fallback = get(&AppSettings::default());
    let key_of = move |value: T| {
        choices
            .iter()
            .find(|(v, ..)| *v == value)
            .map_or(choices[0].1, |(_, key, _)| key)
    };
    let (read, write) = bind(view, get, set);
    SettingField::dropdown(
        choices
            .iter()
            .map(|(_, key, label)| ((*key).into(), label.text()))
            .collect(),
        move |cx: &App| key_of(read(cx)).into(),
        move |key: SharedString, cx: &mut App| {
            let value = choices
                .iter()
                .find(|(_, k, _)| *k == key.as_str())
                .map_or(fallback, |(v, ..)| *v);
            write(value, cx);
        },
    )
    .default_value(key_of(fallback).to_string())
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();

        let font_family = text(&view, |s| s.font_family.clone(), |s, v| s.font_family = v);
        let font_size = number(
            &view,
            FONT_SIZE_RANGE,
            1.0,
            |s| s.font_size as f64,
            |s, v| s.font_size = v as f32,
        );
        let cursor_blink = switch(&view, |s| s.cursor_blink, |s, v| s.cursor_blink = v);
        let opacity = number(
            &view,
            OPACITY_RANGE,
            5.0,
            |s| (s.background_opacity as f64 * 100.0).round(),
            |s, v| s.background_opacity = (v / 100.0) as f32,
        );
        let blur = switch(&view, |s| s.background_blur, |s, v| s.background_blur = v);
        let padding = number(
            &view,
            PADDING_RANGE,
            2.0,
            |s| s.padding as f64,
            |s, v| s.padding = v.round() as f32,
        );
        let line_height = number(
            &view,
            LINE_HEIGHT_RANGE,
            0.05,
            // Rounded to the 0.05 step so float noise doesn't show in the field.
            |s| (s.line_height as f64 * 20.0).round() / 20.0,
            |s, v| s.line_height = v as f32,
        );
        let ignore_title = switch(
            &view,
            |s| s.ignore_program_title,
            |s, v| s.ignore_program_title = v,
        );
        let tint_root = switch(&view, |s| s.tint_root, |s, v| s.tint_root = v);
        let tint_ssh = switch(&view, |s| s.tint_ssh, |s, v| s.tint_ssh = v);
        let minimum_contrast = number(
            &view,
            CONTRAST_RANGE,
            5.0,
            |s| s.minimum_contrast as f64,
            |s, v| s.minimum_contrast = v.round() as f32,
        );
        let border_width = number(
            &view,
            BORDER_WIDTH_RANGE,
            1.0,
            |s| s.border_width as f64,
            |s, v| s.border_width = v.round() as f32,
        );
        let border_when_maximized = switch(
            &view,
            |s| s.border_when_maximized,
            |s, v| s.border_when_maximized = v,
        );
        let tabs_in_title_bar = switch(
            &view,
            |s| s.tabs_in_title_bar,
            |s, v| s.tabs_in_title_bar = v,
        );
        let command = text(&view, |s| s.command.clone(), |s, v| s.command = v);
        let login_shell = switch(&view, |s| s.login_shell, |s, v| s.login_shell = v);
        let scrollback = number(
            &view,
            SCROLLBACK_RANGE,
            1000.0,
            |s| s.scrollback as f64,
            |s, v| s.scrollback = v.round() as usize,
        );

        v_flex().size_full().track_focus(&self.focus_handle).child(
            SettingsPanel::new("app-settings")
                .with_group_variant(GroupBoxVariant::Outline)
                .pages(vec![SettingPage::new(tr("settings.page.terminal"))
                    .group(
                        SettingGroup::new()
                            .title(tr("settings.group.font"))
                            .items(vec![
                                SettingItem::new(tr("settings.font_family"), font_family),
                                SettingItem::new(tr("settings.font_size"), font_size),
                                SettingItem::new(
                                    tr("settings.bold_text"),
                                    dropdown(
                                        &view,
                                        BOLD_TEXT,
                                        |s| s.bold_text,
                                        |s, v| s.bold_text = v,
                                    ),
                                ),
                            ]),
                    )
                    .group(
                        SettingGroup::new()
                            .title(tr("settings.group.theme"))
                            .items(vec![
                                SettingItem::new(
                                    tr("settings.appearance"),
                                    dropdown(
                                        &view,
                                        APPEARANCES,
                                        |s| s.appearance,
                                        |s, v| s.appearance = v,
                                    ),
                                ),
                                SettingItem::new(
                                    tr("settings.dark_theme"),
                                    dropdown(
                                        &view,
                                        DARK_THEMES,
                                        |s| s.dark_theme,
                                        |s, v| s.dark_theme = v,
                                    ),
                                ),
                                SettingItem::new(
                                    tr("settings.light_theme"),
                                    dropdown(
                                        &view,
                                        LIGHT_THEMES,
                                        |s| s.light_theme,
                                        |s, v| s.light_theme = v,
                                    ),
                                ),
                                SettingItem::new(tr("settings.background_opacity"), opacity),
                                SettingItem::new(tr("settings.background_blur"), blur),
                                SettingItem::new(tr("settings.minimum_contrast"), minimum_contrast),
                            ]),
                    )
                    .group(
                        SettingGroup::new()
                            .title(tr("settings.group.layout"))
                            .items(vec![
                                SettingItem::new(tr("settings.padding"), padding),
                                SettingItem::new(tr("settings.line_height"), line_height),
                                SettingItem::new(
                                    tr("settings.tabs_in_title_bar"),
                                    tabs_in_title_bar,
                                ),
                                SettingItem::new(tr("settings.border_width"), border_width),
                                SettingItem::new(
                                    tr("settings.border_when_maximized"),
                                    border_when_maximized,
                                ),
                            ]),
                    )
                    .group(
                        SettingGroup::new()
                            .title(tr("settings.group.cursor"))
                            .items(vec![
                                SettingItem::new(tr("settings.cursor_blink"), cursor_blink),
                                SettingItem::new(
                                    tr("settings.cursor_shape"),
                                    dropdown(
                                        &view,
                                        CURSOR_SHAPES,
                                        |s| s.cursor_shape,
                                        |s, v| s.cursor_shape = v,
                                    ),
                                ),
                            ]),
                    )
                    .group(
                        SettingGroup::new()
                            .title(tr("settings.group.history"))
                            .items(vec![
                                SettingItem::new(tr("settings.scrollback"), scrollback),
                                SettingItem::new(
                                    tr("settings.scrollbar"),
                                    dropdown(
                                        &view,
                                        SCROLLBARS,
                                        |s| s.scrollbar,
                                        |s, v| s.scrollbar = v,
                                    ),
                                ),
                            ]),
                    )
                    .group(
                        SettingGroup::new()
                            .title(tr("settings.group.shell"))
                            .items(vec![
                                SettingItem::new(tr("settings.command"), command),
                                SettingItem::new(tr("settings.login_shell"), login_shell),
                                SettingItem::new(
                                    tr("settings.new_tab_directory"),
                                    dropdown(
                                        &view,
                                        NEW_TAB_DIRECTORIES,
                                        |s| s.new_tab_directory,
                                        |s, v| s.new_tab_directory = v,
                                    ),
                                ),
                            ]),
                    )
                    .group(
                        SettingGroup::new()
                            .title(tr("settings.group.behavior"))
                            .items(vec![
                                SettingItem::new(
                                    tr("settings.language"),
                                    dropdown(
                                        &view,
                                        LANGUAGES,
                                        |s| s.language,
                                        |s, v| s.language = v,
                                    ),
                                ),
                                SettingItem::new(
                                    tr("settings.bell"),
                                    dropdown(&view, BELL_MODES, |s| s.bell, |s, v| s.bell = v),
                                ),
                                SettingItem::new(tr("settings.ignore_program_title"), ignore_title),
                                SettingItem::new(tr("settings.tint_root"), tint_root),
                                SettingItem::new(tr("settings.tint_ssh"), tint_ssh),
                            ]),
                    )]),
        )
    }
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_follows_the_appearance() {
        let settings = |appearance| AppSettings {
            appearance,
            dark_theme: ThemeChoice::Dracula,
            light_theme: ThemeChoice::Alucard,
            ..Default::default()
        };
        assert_eq!(
            settings(Appearance::System).theme(true),
            ThemeChoice::Dracula
        );
        assert_eq!(
            settings(Appearance::System).theme(false),
            ThemeChoice::Alucard
        );
        assert_eq!(
            settings(Appearance::Dark).theme(false),
            ThemeChoice::Dracula
        );
        assert_eq!(
            settings(Appearance::Light).theme(true),
            ThemeChoice::Alucard
        );
    }

    #[test]
    fn older_theme_setting_becomes_the_dark_theme() {
        let loaded: AppSettings = toml::from_str(r#"theme = "gruvbox-dark""#).unwrap();
        assert_eq!(loaded.dark_theme, ThemeChoice::GruvboxDark);
        assert_eq!(loaded.light_theme, AppSettings::default().light_theme);
    }

    #[test]
    fn theme_lists_hold_only_their_kind() {
        assert!(DARK_THEMES.iter().all(|(theme, ..)| theme.is_dark()));
        assert!(LIGHT_THEMES.iter().all(|(theme, ..)| !theme.is_dark()));
        assert!(AppSettings::default().dark_theme.is_dark());
        assert!(!AppSettings::default().light_theme.is_dark());
    }
}
