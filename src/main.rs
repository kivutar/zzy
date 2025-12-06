mod colors;
mod terminal;

use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::{
    h_flex,
    tab::{Tab, TabBar},
    v_flex, Icon, IconName, Root, Theme, ThemeMode, TitleBar,
};
use terminal::TerminalView;

pub struct Example {
    terminal: Entity<TerminalView>,
    focused_once: bool,
}

impl Render for Example {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Give focus to the terminal on first render so typing works immediately.
        if !self.focused_once {
            cx.focus_view(&self.terminal, window);
            self.focused_once = true;
        }

        v_flex()
            .size_full()
            .child(
                // Render custom title bar on top of Root view.
                TitleBar::new().child(
                    h_flex()
                        .w_full()
                        .pr_2()
                        .justify_between()
                        .child("term")
                        .child(IconName::Settings2),
                ),
            )
            .child(
                TabBar::new("tabs")
                    .selected_index(0)
                    .on_click(|selected_index, _, _| {
                        println!("Tab {} selected", selected_index);
                    })
                    .child(
                        Tab::new()
                            .prefix(Icon::new(IconName::SquareTerminal))
                            .label("term ― bash")
                            .px_2()
                            .suffix(IconName::Close),
                    ),
            )
            .child(
                v_flex()
                    .id("window-body")
                    .bg(cx.theme().secondary)
                    .flex_1()
                    .w_full()
                    .h_full()
                    .p_2()
                    .items_start()
                    .justify_start()
                    .overflow_hidden()
                    .child(AnyView::from(self.terminal.clone())),
            )
    }
}

fn main() {
    let app = Application::new().with_assets(gpui_component_assets::Assets);

    app.run(move |cx| {
        gpui_component::init(cx);
        Theme::global_mut(cx).mode = ThemeMode::Dark;
        Theme::change(ThemeMode::Dark, None, cx);
        // Override the monospace font after applying the theme so it doesn't get reset.
        Theme::global_mut(cx).mono_font_family = "Adwaita Mono".into();

        let window_options = WindowOptions {
            // Setup GPUI to use custom title bar and client-side decorations for Wayland/X11.
            titlebar: Some(TitleBar::title_bar_options()),
            window_decorations: Some(WindowDecorations::Client),
            window_background: WindowBackgroundAppearance::Transparent,
            window_bounds: Some(WindowBounds::centered(size(px(800.), px(600.)), cx)),
            ..Default::default()
        };

        cx.spawn(async move |cx| {
            cx.open_window(window_options, |window, cx| {
                let view = cx.new(|cx| {
                    let terminal = cx.new(|cx| TerminalView::new(cx));
                    Example {
                        terminal,
                        focused_once: false,
                    }
                });
                cx.new(|cx| Root::new(view, window, cx))
            })?;

            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });
}
