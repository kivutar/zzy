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

pub const DEFAULT_FONT_FAMILY: &str = "Adwaita Mono";

#[derive(Clone, Debug)]
pub struct AppSettings {
    pub font_size: f32,
    pub cursor_blink: bool,
    pub cursor_shape: CursorShapeSetting,
    pub font_family: SharedString,
    pub ignore_bold: bool,
    pub scrollback: usize,
    pub scrollbar: ScrollbarVisibility,
    pub theme: ThemeChoice,
    /// Window background opacity, 0.0..=1.0.
    pub background_opacity: f32,
    pub background_blur: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            font_size: DEFAULT_FONT_SIZE,
            cursor_blink: true,
            cursor_shape: CursorShapeSetting::Block,
            font_family: DEFAULT_FONT_FAMILY.into(),
            ignore_bold: false,
            scrollback: 1000,
            scrollbar: ScrollbarVisibility::Auto,
            theme: ThemeChoice::HamiMelonDark,
            background_opacity: 1.0,
            background_blur: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShapeSetting {
    Block,
    Beam,
    Underline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollbarVisibility {
    Always,
    Auto,
    Never,
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

const FONT_SIZE_RANGE: (f64, f64) = (8.0, 72.0);
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
        let ignore_bold = {
            let (read, write) = bind(&view, |s| s.ignore_bold, |s, v| s.ignore_bold = v);
            SettingField::switch(read, write).default_value(defaults.ignore_bold)
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

        v_flex()
            .p_neg_2()
            .size_full()
            .track_focus(&self.focus_handle)
            .child(
                SettingsPanel::new("app-settings")
                    .with_group_variant(GroupBoxVariant::Outline)
                    .pages(vec![SettingPage::new("Terminal")
                        .group(SettingGroup::new().title("Font").items(vec![
                            SettingItem::new("Font Family", font_family),
                            SettingItem::new("Font Size", font_size),
                            SettingItem::new("Ignore Bold", ignore_bold),
                        ]))
                        .group(SettingGroup::new().title("Theme").items(vec![
                            SettingItem::new(
                                "Color Theme",
                                dropdown(&view, THEMES, |s| s.theme, |s, v| s.theme = v),
                            ),
                            SettingItem::new("Background Opacity (%)", opacity),
                            SettingItem::new("Background Blur", blur),
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
                                dropdown(
                                    &view,
                                    SCROLLBARS,
                                    |s| s.scrollbar,
                                    |s, v| s.scrollbar = v,
                                ),
                            ),
                        ]))]),
            )
    }
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
