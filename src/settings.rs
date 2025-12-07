use crate::terminal::DEFAULT_FONT_SIZE;
use gpui::{
    div, px, AppContext, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement,
    Render, Styled, Window,
};
use gpui_component::StyledExt;
use gpui_component::{
    h_flex,
    slider::{Slider, SliderEvent, SliderScale, SliderState, SliderValue},
    switch::Switch,
    v_flex,
};

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
    font_size: f32,
    cursor_blink: bool,
    font_slider: Entity<SliderState>,
    focus_handle: FocusHandle,
}

impl SettingsView {
    pub fn new(cx: &mut Context<Self>, settings: AppSettings) -> Self {
        let font_slider = cx.new(|_| {
            SliderState::new()
                .min(8.0)
                .max(32.0)
                .step(1.0)
                .scale(SliderScale::Linear)
                .default_value(settings.font_size)
        });
        let focus_handle = cx.focus_handle();

        cx.subscribe(&font_slider, |view, _slider, event, cx| {
            if let SliderEvent::Change(SliderValue::Single(size)) = *event {
                view.font_size = size.clamp(8.0, 72.0);
                cx.notify();
            }
        })
        .detach();

        Self {
            font_size: settings.font_size,
            cursor_blink: settings.cursor_blink,
            font_slider,
            focus_handle,
        }
    }

    pub fn current_settings(&self) -> AppSettings {
        AppSettings {
            font_size: self.font_size,
            cursor_blink: self.cursor_blink,
        }
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let slider_state = self.font_slider.clone();
        let view = cx.entity().downgrade();
        div().paddings(px(12.0)).child(
            v_flex()
                .gap_4()
                .child(
                    v_flex().gap_2().child("Font size").child(
                        h_flex()
                            .items_center()
                            .gap_3()
                            .child(format!("{:.0}px", self.font_size))
                            .child(Slider::new(&slider_state)),
                    ),
                )
                .child(
                    v_flex().gap_2().child("Cursor blinking").child(
                        Switch::new("cursor-blink")
                            .checked(self.cursor_blink)
                            .on_click(move |checked, _window, app| {
                                if let Some(view) = view.upgrade() {
                                    let _ = view.update(app, |view, cx| {
                                        view.cursor_blink = *checked;
                                        cx.notify();
                                    });
                                }
                            })
                            .label("Enable blinking"),
                    ),
                ),
        )
    }
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
