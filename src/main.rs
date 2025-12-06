mod colors;
mod terminal;

use gpui::*;
use gpui_component::ActiveTheme;
use gpui_component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    tab::{Tab, TabBar},
    v_flex, Icon, IconName, Root, Sizable, Theme, ThemeMode, TitleBar,
};
use terminal::TerminalView;

pub struct ZedTerm {
    tabs: Vec<TerminalTab>,
    selected_index: usize,
    focus_pending: bool,
    keystroke_sub: Option<Subscription>,
}

struct TerminalTab {
    terminal: Entity<TerminalView>,
}

impl ZedTerm {
    fn new(cx: &mut Context<Self>) -> Self {
        let mut zedterm = Self {
            tabs: Vec::new(),
            selected_index: 0,
            focus_pending: true,
            keystroke_sub: None,
        };

        let view = cx.entity().downgrade();
        zedterm.keystroke_sub = Some(cx.intercept_keystrokes(move |event, window, cx| {
            if event.keystroke.modifiers.control
                && event.keystroke.modifiers.shift
                && event.keystroke.key.eq_ignore_ascii_case("t")
            {
                if let Some(view) = view.upgrade() {
                    let _ = view.update(cx, |zedterm, cx| {
                        zedterm.add_tab(cx);
                        if let Some(terminal) = zedterm.selected_terminal() {
                            cx.focus_view(terminal, window);
                        }
                    });
                }
                cx.stop_propagation();
            }
        }));

        zedterm.add_tab(cx);
        zedterm
    }

    fn add_tab(&mut self, cx: &mut Context<Self>) {
        let terminal = cx.new(|cx| TerminalView::new(cx));
        cx.observe(&terminal, |_zedterm, _terminal, cx| {
            // Propagate terminal changes (e.g., title updates) to the tab bar.
            cx.notify();
        })
        .detach();
        self.tabs.push(TerminalTab { terminal });
        self.selected_index = self.tabs.len() - 1;
        self.focus_pending = true;
        cx.notify();
    }

    fn select_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.tabs.len() {
            self.selected_index = index;
            self.focus_pending = true;
            cx.notify();
        }
    }

    fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }

        let selected_before = self.selected_index;
        self.tabs.remove(index);

        if self.tabs.is_empty() {
            cx.quit();
            return;
        }

        if selected_before > index {
            self.selected_index = selected_before - 1;
        } else if selected_before >= self.tabs.len() {
            self.selected_index = self.tabs.len() - 1;
        }

        self.focus_pending = true;
        cx.notify();
    }

    fn selected_terminal(&self) -> Option<&Entity<TerminalView>> {
        self.tabs.get(self.selected_index).map(|tab| &tab.terminal)
    }
}

impl Render for ZedTerm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.tabs.is_empty() {
            return v_flex().child("No tabs");
        }

        if self.focus_pending {
            if let Some(terminal) = self.selected_terminal() {
                cx.focus_view(terminal, window);
            }
            self.focus_pending = false;
        }

        let view = cx.entity();
        v_flex()
            .size_full()
            .child(
                // Render custom title bar on top of Root view.
                TitleBar::new().child(
                    h_flex()
                        .w_full()
                        .pr_2()
                        .justify_between()
                        .child("zedterm")
                        .child(IconName::Settings2),
                ),
            )
            .child(
                TabBar::new("tabs")
                    .w_full()
                    .selected_index(self.selected_index)
                    .on_click({
                        let view = view.clone();
                        move |selected_index, window, cx| {
                            let view = view.clone();
                            let _ = view.update(cx, |zedterm, cx| {
                                zedterm.select_tab(*selected_index, cx);
                                if let Some(terminal) = zedterm.selected_terminal() {
                                    cx.focus_view(terminal, window);
                                }
                            });
                        }
                    })
                    .suffix({
                        let view = view.clone();
                        Button::new("add-tab")
                            .ghost()
                            .xsmall()
                            .mr_2()
                            .icon(IconName::Plus)
                            .tab_stop(false)
                            .on_click(move |_, window, cx| {
                                let view = view.clone();
                                let _ = view.update(cx, |zedterm, cx| {
                                    zedterm.add_tab(cx);
                                    if let Some(terminal) = zedterm.selected_terminal() {
                                        cx.focus_view(terminal, window);
                                    }
                                });
                            })
                    })
                    .children(self.tabs.iter().enumerate().map(|(index, tab)| {
                        let close_view = view.clone();
                        let label = tab.terminal.read(cx).tab_title();
                        Tab::new()
                            .prefix(Icon::new(IconName::SquareTerminal))
                            .label(label)
                            .px_2()
                            .suffix(
                                Button::new(("close-tab", index as u32))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Close)
                                    .tab_stop(false)
                                    .on_click(move |_, window, cx| {
                                        cx.stop_propagation();
                                        let view = close_view.clone();
                                        let _ = view.update(cx, |zedterm, cx| {
                                            zedterm.close_tab(index, cx);
                                            if let Some(terminal) = zedterm.selected_terminal() {
                                                cx.focus_view(terminal, window);
                                            }
                                        });
                                    }),
                            )
                    })),
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
                    .child(AnyView::from(
                        self.tabs[self.selected_index].terminal.clone(),
                    )),
            )
    }
}

fn main() {
    let app = Application::new().with_assets(gpui_component_assets::Assets);

    app.run(move |cx| {
        gpui_component::init(cx);
        Theme::global_mut(cx).mode = ThemeMode::Dark;
        Theme::change(ThemeMode::Dark, None, cx);
        let border_color = Theme::global(cx).border;
        Theme::global_mut(cx).tab_active = border_color;
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
                let view = cx.new(|cx| ZedTerm::new(cx));
                cx.new(|cx| Root::new(view, window, cx))
            })?;

            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });
}
