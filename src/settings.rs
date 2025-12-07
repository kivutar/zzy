use crate::terminal::DEFAULT_FONT_SIZE;
use gpui::{
    App, Context, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Render,
    Styled, Window,
};
use gpui_component::setting::{
    NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage,
    Settings as SettingsPanel,
};
use gpui_component::v_flex;

#[derive(Clone, Copy, Debug)]
pub struct AppSettings {
    pub font_size: f32,
    pub cursor_blink: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            font_size: DEFAULT_FONT_SIZE,
            cursor_blink: true,
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
        self.settings
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

        v_flex().p_neg_2().size_full().track_focus(&self.focus_handle).child(
            SettingsPanel::new("app-settings").pages(vec![SettingPage::new("Terminal").group(
                SettingGroup::new().title("Appearance").items(vec![
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
                                    let clamped = val.clamp(min_font_size, max_font_size);
                                    let _ = view.update(cx, |view, cx| {
                                        let clamped = clamped as f32;
                                        if (view.settings.font_size - clamped).abs() > f32::EPSILON
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
                ]),
            )]),
        )
    }
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
