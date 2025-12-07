use crate::terminal::DEFAULT_FONT_SIZE;
use gpui::{
    App, Context, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, Styled, Window,
};
use gpui_component::group_box::GroupBoxVariant;
use gpui_component::setting::{
    NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage,
    Settings as SettingsPanel,
};
use gpui_component::v_flex;

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
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShapeSetting {
    Block,
    Beam,
    Underline,
}

impl CursorShapeSetting {
    pub fn as_str(&self) -> &'static str {
        match self {
            CursorShapeSetting::Block => "block",
            CursorShapeSetting::Beam => "beam",
            CursorShapeSetting::Underline => "underline",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollbarVisibility {
    Always,
    Auto,
    Never,
}

impl ScrollbarVisibility {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScrollbarVisibility::Always => "always",
            ScrollbarVisibility::Auto => "auto",
            ScrollbarVisibility::Never => "never",
        }
    }
}

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

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let default_settings = AppSettings::default();
        let min_font_size = 8.0;
        let max_font_size = 72.0;
        let font_size_options = NumberFieldOptions {
            min: min_font_size,
            max: max_font_size,
            step: 1.0,
        };
        let scrollbar_options = vec![
            ("auto".into(), "Auto".into()),
            ("always".into(), "Always".into()),
            ("never".into(), "Never".into()),
        ];
        let cursor_shape_options = vec![
            ("block".into(), "Block".into()),
            ("beam".into(), "Beam".into()),
            ("underline".into(), "Underline".into()),
        ];

        v_flex()
            .p_neg_2()
            .size_full()
            .track_focus(&self.focus_handle)
            .child(
                SettingsPanel::new("app-settings")
                    .with_group_variant(GroupBoxVariant::Outline)
                    .pages(vec![SettingPage::new("Terminal")
                        .group(SettingGroup::new().title("Font").items(vec![
                                SettingItem::new(
                                    "Font Family",
                                    SettingField::input(
                                        {
                                            let view = view.clone();
                                            move |cx: &App| {
                                                view.read(cx).settings.font_family.clone()
                                            }
                                        },
                                        {
                                            let view = view.clone();
                                            move |val: SharedString, cx: &mut App| {
                                                let _ = view.update(cx, |view, cx| {
                                                    if view.settings.font_family != val {
                                                        view.settings.font_family = val;
                                                        cx.notify();
                                                    }
                                                });
                                            }
                                        },
                                    )
                                    .default_value(default_settings.font_family.clone()),
                                ),
                            SettingItem::new(
                                "Font Size",
                                SettingField::number_input(
                                    font_size_options,
                                    {
                                            let view = view.clone();
                                            move |cx: &App| view.read(cx).settings.font_size as f64
                                        },
                                        {
                                            let view = view.clone();
                                            move |val: f64, cx: &mut App| {
                                                let clamped =
                                                    val.clamp(min_font_size, max_font_size);
                                                let _ = view.update(cx, |view, cx| {
                                                    let clamped = clamped as f32;
                                                    if (view.settings.font_size - clamped).abs()
                                                        > f32::EPSILON
                                                    {
                                                        view.settings.font_size = clamped;
                                                        cx.notify();
                                                    }
                                                });
                                            }
                                        },
                                    )
                                    .default_value(default_settings.font_size as f64),
                                ),
                                SettingItem::new(
                                    "Ignore Bold",
                                    SettingField::switch(
                                        {
                                            let view = view.clone();
                                            move |cx: &App| view.read(cx).settings.ignore_bold
                                        },
                                        {
                                            let view = view.clone();
                                            move |val: bool, cx: &mut App| {
                                                let _ = view.update(cx, |view, cx| {
                                                    if view.settings.ignore_bold != val {
                                                        view.settings.ignore_bold = val;
                                                        cx.notify();
                                                    }
                                                });
                                            }
                                        },
                                    )
                                    .default_value(default_settings.ignore_bold),
                                ),
                            ]))
                        .group(SettingGroup::new().title("Cursor").items(vec![
                                SettingItem::new(
                                    "Cursor Blinking",
                                    SettingField::switch(
                                        {
                                            let view = view.clone();
                                            move |cx: &App| view.read(cx).settings.cursor_blink
                                        },
                                        {
                                            let view = view.clone();
                                            move |val: bool, cx: &mut App| {
                                                let _ = view.update(cx, |view, cx| {
                                                    if view.settings.cursor_blink != val {
                                                        view.settings.cursor_blink = val;
                                                        cx.notify();
                                                    }
                                                });
                                            }
                                        },
                                    )
                                    .default_value(default_settings.cursor_blink),
                                ),
                                SettingItem::new(
                                    "Cursor Shape",
                                    SettingField::dropdown(
                                        cursor_shape_options,
                                        {
                                            let view = view.clone();
                                            move |cx: &App| {
                                                SharedString::from(
                                                    view.read(cx).settings.cursor_shape.as_str(),
                                                )
                                            }
                                        },
                                        {
                                            let view = view.clone();
                                            move |val: SharedString, cx: &mut App| {
                                                let shape = match val.as_str() {
                                                    "beam" => CursorShapeSetting::Beam,
                                                    "underline" => CursorShapeSetting::Underline,
                                                    _ => CursorShapeSetting::Block,
                                                };
                                                let _ = view.update(cx, |view, cx| {
                                                    if view.settings.cursor_shape != shape {
                                                        view.settings.cursor_shape = shape;
                                                        cx.notify();
                                                    }
                                                });
                                            }
                                        },
                                    )
                                    .default_value(
                                        default_settings.cursor_shape.as_str().to_string(),
                                    ),
                                ),
                            ]))
                        .group(SettingGroup::new().title("History").items(vec![
                            SettingItem::new(
                                "Scrollback Lines",
                                SettingField::number_input(
                                    NumberFieldOptions {
                                        min: 0.0,
                                        max: 50_000.0,
                                        step: 1000.0,
                                    },
                                    {
                                        let view = view.clone();
                                        move |cx: &App| view.read(cx).settings.scrollback as f64
                                    },
                                    {
                                        let view = view.clone();
                                        move |val: f64, cx: &mut App| {
                                            let clamped = val.clamp(0.0, 50_000.0);
                                            let lines = clamped.round() as usize;
                                            let _ = view.update(cx, |view, cx| {
                                                if view.settings.scrollback != lines {
                                                    view.settings.scrollback = lines;
                                                    cx.notify();
                                                }
                                            });
                                        }
                                    },
                                )
                                .default_value(default_settings.scrollback as f64),
                            ),
                            SettingItem::new(
                                "Scrollbar",
                                SettingField::dropdown(
                                    scrollbar_options,
                                    {
                                        let view = view.clone();
                                        move |cx: &App| {
                                            SharedString::from(
                                                view.read(cx).settings.scrollbar.as_str(),
                                            )
                                        }
                                    },
                                    {
                                        let view = view.clone();
                                        move |val: SharedString, cx: &mut App| {
                                            let visibility = match val.as_str() {
                                                "always" => ScrollbarVisibility::Always,
                                                "never" => ScrollbarVisibility::Never,
                                                _ => ScrollbarVisibility::Auto,
                                            };
                                            let _ = view.update(cx, |view, cx| {
                                                if view.settings.scrollbar != visibility {
                                                    view.settings.scrollbar = visibility;
                                                    cx.notify();
                                                }
                                            });
                                        }
                                    },
                                )
                                .default_value(default_settings.scrollbar.as_str().to_string()),
                            ),
                        ]))]),
            )
    }
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
