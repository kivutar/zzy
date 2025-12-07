mod colors;
mod settings;
mod terminal;
mod theme;

use colors::{theme_background, theme_foreground};
use gpui::*;
use gpui_component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    tab::{Tab, TabBar},
    v_flex, Icon, IconName, Root, Sizable, Theme, ThemeMode, TitleBar,
};
use settings::{AppSettings, SettingsView};
use terminal::TerminalView;

pub struct ZedTerm {
    tabs: Vec<TabContent>,
    selected_index: usize,
    focus_pending: bool,
    keystroke_sub: Option<Subscription>,
    settings: AppSettings,
    settings_dirty: bool,
}

enum TabContent {
    Terminal(Entity<TerminalView>),
    Settings(Entity<SettingsView>),
}

impl ZedTerm {
    fn new(cx: &mut Context<Self>) -> Self {
        let mut zedterm = Self {
            tabs: Vec::new(),
            selected_index: 0,
            focus_pending: true,
            keystroke_sub: None,
            settings: AppSettings::default(),
            settings_dirty: false,
        };

        let view = cx.entity().downgrade();
        zedterm.keystroke_sub = Some(cx.intercept_keystrokes(move |event, window, cx| {
            if event.keystroke.modifiers.control
                && event.keystroke.modifiers.shift
                && event.keystroke.key.eq_ignore_ascii_case("t")
            {
                if let Some(view) = view.upgrade() {
                    let _ = view.update(cx, |zedterm, cx| {
                        zedterm.add_terminal_tab(cx);
                        zedterm.focus_selected(window, cx);
                    });
                }
                cx.stop_propagation();
            }
        }));

        zedterm.add_terminal_tab(cx);
        zedterm
    }

    fn add_terminal_tab(&mut self, cx: &mut Context<Self>) {
        let settings = self.settings.clone();
        let terminal = cx.new(move |cx| TerminalView::new(cx, &settings));
        cx.observe(&terminal, |_zedterm, _terminal, cx| {
            // Propagate terminal changes (e.g., title updates) to the tab bar.
            cx.notify();
        })
        .detach();
        self.tabs.push(TabContent::Terminal(terminal));
        self.selected_index = self.tabs.len() - 1;
        self.focus_pending = true;
        cx.notify();
    }

    fn ensure_settings_tab(&mut self, cx: &mut Context<Self>) {
        if let Some(idx) = self
            .tabs
            .iter()
            .position(|tab| matches!(tab, TabContent::Settings(_)))
        {
            self.selected_index = idx;
            self.focus_pending = true;
            cx.notify();
            return;
        }

        let settings = self.settings.clone();
        let settings_view = cx.new(move |cx| SettingsView::new(cx, settings));
        cx.observe(&settings_view, |zedterm, settings_view, cx| {
            let settings = settings_view.read(cx).current_settings();
            zedterm.settings = settings;
            zedterm.settings_dirty = true;
            cx.notify();
        })
        .detach();

        self.tabs.push(TabContent::Settings(settings_view));
        self.selected_index = self.tabs.len() - 1;
        self.focus_pending = true;
        cx.notify();
    }

    fn apply_settings_to_terminals(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.settings_dirty {
            return;
        }

        for tab in &self.tabs {
            if let TabContent::Terminal(term) = tab {
                let settings = self.settings.clone();
                let _ = term.update(cx, |view, cx| view.apply_settings(&settings, window, cx));
            }
        }

        self.settings_dirty = false;
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

    fn focus_selected(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get(self.selected_index) {
            match tab {
                TabContent::Terminal(term) => cx.focus_view(term, window),
                TabContent::Settings(view) => cx.focus_view(view, window),
            }
        }
    }
}

impl Render for ZedTerm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.apply_settings_to_terminals(window, cx);

        if self.tabs.is_empty() {
            return v_flex().child("No tabs");
        }

        if self.focus_pending {
            self.focus_selected(window, cx);
            self.focus_pending = false;
        }

        let view = cx.entity();
        let app_bg = theme_background(self.settings.theme);
        let app_fg = theme_foreground(self.settings.theme);
        Theme::global_mut(cx).background = app_bg.into();
        Theme::global_mut(cx).secondary = app_bg.into();
        Theme::global_mut(cx).foreground = app_fg.into();
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
                        .child(
                            Button::new("open-settings")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Settings2)
                                .cursor(CursorStyle::PointingHand)
                                .on_click({
                                    let view = view.clone();
                                    move |_, window, cx| {
                                        let _ = view.update(cx, |zedterm, cx| {
                                            zedterm.ensure_settings_tab(cx);
                                            zedterm.focus_selected(window, cx);
                                        });
                                    }
                                }),
                        ),
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
                                zedterm.focus_selected(window, cx);
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
                            .cursor(CursorStyle::PointingHand)
                            .tab_stop(false)
                            .on_click(move |_, window, cx| {
                                let view = view.clone();
                                let _ = view.update(cx, |zedterm, cx| {
                                    zedterm.add_terminal_tab(cx);
                                    zedterm.focus_selected(window, cx);
                                });
                            })
                    })
                    .children(self.tabs.iter().enumerate().map(|(index, tab)| {
                        let close_view = view.clone();
                        let (label, icon) = match tab {
                            TabContent::Terminal(term) => {
                                (term.read(cx).tab_title(), IconName::SquareTerminal)
                            }
                            TabContent::Settings(_) => {
                                ("Settings".to_string(), IconName::Settings2)
                            }
                        };
                        Tab::new()
                            .prefix(Icon::new(icon))
                            .label(label)
                            .px_2()
                            .cursor(CursorStyle::PointingHand)
                            .suffix(
                                Button::new(("close-tab", index as u32))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Close)
                                    .cursor(CursorStyle::PointingHand)
                                    .tab_stop(false)
                                    .on_click(move |_, window, cx| {
                                        cx.stop_propagation();
                                        let view = close_view.clone();
                                        let _ = view.update(cx, |zedterm, cx| {
                                            zedterm.close_tab(index, cx);
                                            zedterm.focus_selected(window, cx);
                                        });
                                    }),
                            )
                    })),
            )
            .child(
                v_flex()
                    .id("window-body")
                    .bg(app_bg)
                    .flex_1()
                    .w_full()
                    .h_full()
                    .p_2()
                    .items_start()
                    .justify_start()
                    .overflow_hidden()
                    .child(match &self.tabs[self.selected_index] {
                        TabContent::Terminal(term) => AnyView::from(term.clone()),
                        TabContent::Settings(view) => AnyView::from(view.clone()),
                    }),
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
