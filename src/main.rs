use gpui::*;
use gpui_component::{
    tab::{Tab, TabBar},
    h_flex, v_flex, Root, TitleBar,
    Theme, ThemeMode, IconName
};

pub struct Example;
impl Render for Example {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(
                // Render custom title bar on top of Root view.
                TitleBar::new().child(
                    h_flex()
                        .w_full()
                        .pr_2()
                        .justify_between()
                        .child("Terminal")
                        .child(IconName::Settings2),
                ),
            )
            .child(
                TabBar::new("tabs")
                    .selected_index(0)
                    .on_click(|selected_index, _, _| {
                        println!("Tab {} selected", selected_index);
                    })
                    .child(Tab::new().label("/bin/bash ~")
                        .px_2()
                        .suffix(IconName::Close)
                    )
            )
            .child(
                div()
                    .id("window-body")
                    .p_5()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .child("Hello, World!")
            )
    }
}

fn main() {
    let app = Application::new().with_assets(gpui_component_assets::Assets);

    app.run(move |cx| {
        gpui_component::init(cx);
        Theme::global_mut(cx).mode = ThemeMode::Dark;
        Theme::change(ThemeMode::Dark, None, cx);

        let window_options = WindowOptions {
            // Setup GPUI to use custom title bar
            titlebar: Some(TitleBar::title_bar_options()),
            window_bounds: Some(WindowBounds::centered(size(px(800.), px(600.)), cx)),
            ..Default::default()
        };

        cx.spawn(async move |cx| {
            cx.open_window(window_options, |window, cx| {
                let view = cx.new(|_| Example);
                cx.new(|cx| Root::new(view, window, cx))
            })?;

            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });
}
