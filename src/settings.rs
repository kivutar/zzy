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
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            font_size: DEFAULT_FONT_SIZE,
            cursor_blink: true,
            cursor_shape: CursorShapeSetting::Block,
            font_family: DEFAULT_FONT_FAMILY.into(),
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
                        .group(
                            SettingGroup::new().title("Font").items(vec![
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
                            ]),
                        )
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
                            ]))]),
            )
    }
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
