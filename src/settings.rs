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
    pub theme: ThemeChoice,
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
            theme: ThemeChoice::HamiMelonDark,
            background_opacity: 1.0,
            background_blur: true,
            command: SharedString::default(),
            login_shell: false,
            new_tab_directory: NewTabDirectory::Current,
            padding: 8.0,
            line_height: 1.0,
            bell: BellMode::Sound,
            ignore_program_title: false,
        }
    }
}

impl AppSettings {
    /// Clamp values (e.g. from a hand-edited file) to what the UI allows.
    pub fn sanitized(mut self) -> Self {
        let clamp =
            |v: f64, (min, max): (f64, f64)| if v.is_nan() { min } else { v.clamp(min, max) };
        self.font_size = clamp(self.font_size as f64, FONT_SIZE_RANGE) as f32;
        self.padding = clamp(self.padding as f64, PADDING_RANGE) as f32;
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

/// Dropdown entries: (value, key, label), in display order.
type Choices<T> = &'static [(T, &'static str, &'static str)];

const THEMES: Choices<ThemeChoice> = &[
    (ThemeChoice::RosePine, "rose-pine", "Rosé Pine"),
    (ThemeChoice::Dracula, "dracula", "Dracula"),
    (ThemeChoice::GruvboxDark, "gruvbox-dark", "Gruvbox Dark"),
    (
        ThemeChoice::HamiMelonDark,
        "hami-melon-dark",
        "Hami Melon Dark",
    ),
    (ThemeChoice::AyuDark, "ayu-dark", "Ayu Dark"),
];

const CURSOR_SHAPES: Choices<CursorShapeSetting> = &[
    (CursorShapeSetting::Block, "block", "Block"),
    (CursorShapeSetting::Beam, "beam", "Beam"),
    (CursorShapeSetting::Underline, "underline", "Underline"),
];

const SCROLLBARS: Choices<ScrollbarVisibility> = &[
    (ScrollbarVisibility::Auto, "auto", "Auto"),
    (ScrollbarVisibility::Always, "always", "Always"),
    (ScrollbarVisibility::Never, "never", "Never"),
];

const BELL_MODES: Choices<BellMode> = &[
    (BellMode::Sound, "sound", "Sound"),
    (BellMode::Flash, "flash", "Flash"),
    (
        BellMode::SoundAndFlash,
        "sound-and-flash",
        "Sound and Flash",
    ),
    (BellMode::Off, "off", "Off"),
];

const BOLD_TEXT: Choices<BoldText> = &[
    (BoldText::BoldFont, "bold-font", "Bold Font"),
    (BoldText::BrightColors, "bright-colors", "Bright Colors"),
    (
        BoldText::BoldFontAndBrightColors,
        "bold-font-and-bright-colors",
        "Bold Font and Bright Colors",
    ),
    (BoldText::Normal, "normal", "Normal"),
];

const NEW_TAB_DIRECTORIES: Choices<NewTabDirectory> = &[
    (
        NewTabDirectory::Current,
        "current",
        "Current Tab's Directory",
    ),
    (NewTabDirectory::Home, "home", "Home"),
];

pub(crate) const FONT_SIZE_RANGE: (f64, f64) = (8.0, 72.0);
const PADDING_RANGE: (f64, f64) = (0.0, 48.0);
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
            .map(|(_, key, label)| ((*key).into(), (*label).into()))
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
        let defaults = AppSettings::default();

        let font_family = {
            let (read, write) = bind(&view, |s| s.font_family.clone(), |s, v| s.font_family = v);
            SettingField::input(read, write).default_value(defaults.font_family.clone())
        };
        let font_size = {
            let (read, write) = bind(
                &view,
                |s| s.font_size as f64,
                |s, v| s.font_size = v.clamp(FONT_SIZE_RANGE.0, FONT_SIZE_RANGE.1) as f32,
            );
            let options = NumberFieldOptions {
                min: FONT_SIZE_RANGE.0,
                max: FONT_SIZE_RANGE.1,
                step: 1.0,
            };
            SettingField::number_input(options, read, write)
                .default_value(defaults.font_size as f64)
        };
        let cursor_blink = {
            let (read, write) = bind(&view, |s| s.cursor_blink, |s, v| s.cursor_blink = v);
            SettingField::switch(read, write).default_value(defaults.cursor_blink)
        };
        let opacity = {
            let (read, write) = bind(
                &view,
                |s| (s.background_opacity as f64 * 100.0).round(),
                |s, v| {
                    s.background_opacity =
                        (v.clamp(OPACITY_RANGE.0, OPACITY_RANGE.1) / 100.0) as f32
                },
            );
            let options = NumberFieldOptions {
                min: OPACITY_RANGE.0,
                max: OPACITY_RANGE.1,
                step: 5.0,
            };
            SettingField::number_input(options, read, write)
                .default_value(defaults.background_opacity as f64 * 100.0)
        };
        let blur = {
            let (read, write) = bind(&view, |s| s.background_blur, |s, v| s.background_blur = v);
            SettingField::switch(read, write).default_value(defaults.background_blur)
        };
        let padding = {
            let (read, write) = bind(
                &view,
                |s| s.padding as f64,
                |s, v| s.padding = v.clamp(PADDING_RANGE.0, PADDING_RANGE.1).round() as f32,
            );
            let options = NumberFieldOptions {
                min: PADDING_RANGE.0,
                max: PADDING_RANGE.1,
                step: 2.0,
            };
            SettingField::number_input(options, read, write).default_value(defaults.padding as f64)
        };
        let line_height = {
            let (read, write) = bind(
                &view,
                // Rounded to the 0.05 step so float noise doesn't show in the field.
                |s| (s.line_height as f64 * 20.0).round() / 20.0,
                |s, v| s.line_height = v.clamp(LINE_HEIGHT_RANGE.0, LINE_HEIGHT_RANGE.1) as f32,
            );
            let options = NumberFieldOptions {
                min: LINE_HEIGHT_RANGE.0,
                max: LINE_HEIGHT_RANGE.1,
                step: 0.05,
            };
            SettingField::number_input(options, read, write)
                .default_value(defaults.line_height as f64)
        };
        let ignore_title = {
            let (read, write) = bind(
                &view,
                |s| s.ignore_program_title,
                |s, v| s.ignore_program_title = v,
            );
            SettingField::switch(read, write).default_value(defaults.ignore_program_title)
        };
        let command = {
            let (read, write) = bind(&view, |s| s.command.clone(), |s, v| s.command = v);
            SettingField::input(read, write).default_value(defaults.command.clone())
        };
        let login_shell = {
            let (read, write) = bind(&view, |s| s.login_shell, |s, v| s.login_shell = v);
            SettingField::switch(read, write).default_value(defaults.login_shell)
        };
        let scrollback = {
            let (read, write) = bind(
                &view,
                |s| s.scrollback as f64,
                |s, v| {
                    s.scrollback = v.clamp(SCROLLBACK_RANGE.0, SCROLLBACK_RANGE.1).round() as usize
                },
            );
            let options = NumberFieldOptions {
                min: SCROLLBACK_RANGE.0,
                max: SCROLLBACK_RANGE.1,
                step: 1000.0,
            };
            SettingField::number_input(options, read, write)
                .default_value(defaults.scrollback as f64)
        };

        v_flex().size_full().track_focus(&self.focus_handle).child(
            SettingsPanel::new("app-settings")
                .with_group_variant(GroupBoxVariant::Outline)
                .pages(vec![SettingPage::new("Terminal")
                    .group(SettingGroup::new().title("Font").items(vec![
                        SettingItem::new("Font Family", font_family),
                        SettingItem::new("Font Size", font_size),
                        SettingItem::new(
                            "Bold Text",
                            dropdown(&view, BOLD_TEXT, |s| s.bold_text, |s, v| s.bold_text = v),
                        ),
                    ]))
                    .group(SettingGroup::new().title("Theme").items(vec![
                        SettingItem::new(
                            "Color Theme",
                            dropdown(&view, THEMES, |s| s.theme, |s, v| s.theme = v),
                        ),
                        SettingItem::new("Background Opacity (%)", opacity),
                        SettingItem::new("Background Blur", blur),
                    ]))
                    .group(SettingGroup::new().title("Layout").items(vec![
                        SettingItem::new("Padding (px)", padding),
                        SettingItem::new("Line Height", line_height),
                    ]))
                    .group(SettingGroup::new().title("Cursor").items(vec![
                        SettingItem::new("Cursor Blinking", cursor_blink),
                        SettingItem::new(
                            "Cursor Shape",
                            dropdown(
                                &view,
                                CURSOR_SHAPES,
                                |s| s.cursor_shape,
                                |s, v| s.cursor_shape = v,
                            ),
                        ),
                    ]))
                    .group(SettingGroup::new().title("History").items(vec![
                        SettingItem::new("Scrollback Lines", scrollback),
                        SettingItem::new(
                            "Scrollbar",
                            dropdown(&view, SCROLLBARS, |s| s.scrollbar, |s, v| s.scrollbar = v),
                        ),
                    ]))
                    .group(SettingGroup::new().title("Shell").items(vec![
                        SettingItem::new("Command", command),
                        SettingItem::new("Login Shell", login_shell),
                        SettingItem::new(
                            "New Tab Directory",
                            dropdown(
                                &view,
                                NEW_TAB_DIRECTORIES,
                                |s| s.new_tab_directory,
                                |s, v| s.new_tab_directory = v,
                            ),
                        ),
                    ]))
                    .group(SettingGroup::new().title("Behavior").items(vec![
                        SettingItem::new(
                            "Bell",
                            dropdown(&view, BELL_MODES, |s| s.bell, |s, v| s.bell = v),
                        ),
                        SettingItem::new("Ignore Program Titles", ignore_title),
                    ]))]),
        )
    }
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
