mod colors;
mod settings;
mod terminal;
mod theme;

use colors::{theme_background, theme_foreground};
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    tab::{Tab, TabBar},
    v_flex, Icon, IconName, Root, Sizable, Theme, ThemeMode, TitleBar,
};
use gpui_kit::*;
use settings::{AppSettings, SettingsView};
use terminal::{TerminalExited, TerminalView};

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
            let modifiers = &event.keystroke.modifiers;
            let key = &event.keystroke.key;

            if key.eq_ignore_ascii_case("f11")
                && !modifiers.control
                && !modifiers.alt
                && !modifiers.shift
                && !modifiers.platform
            {
                window.toggle_fullscreen();
                cx.stop_propagation();
                return;
            }

            if modifiers.control && modifiers.shift && key.eq_ignore_ascii_case("t") {
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
        cx.subscribe(&terminal, |zedterm, terminal, _: &TerminalExited, cx| {
            let index = zedterm
                .tabs
                .iter()
                .position(|tab| matches!(tab, TabContent::Terminal(t) if *t == terminal));
            if let Some(index) = index {
                zedterm.close_tab(index, cx);
            }
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
            if settings.theme != zedterm.settings.theme {
                apply_app_theme(settings.theme, cx);
            }
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

/// Override the component theme with the terminal palette.
///
/// Must go through `Theme::update` so the render tokens (e.g. `tokens.tab_active`)
/// stay in sync with `colors`; it also refreshes all windows, so don't call it
/// from `render`.
fn apply_app_theme(choice: theme::ThemeChoice, cx: &mut App) {
    let bg: Hsla = theme_background(choice).into();
    let fg: Hsla = theme_foreground(choice).into();
    let border: Hsla = colors::theme_border(choice).into();
    Theme::update(cx, |theme| {
        theme.background = bg;
        theme.secondary = bg;
        theme.foreground = fg;
        theme.secondary_foreground = fg;
        theme.tab_foreground = fg;
        theme.tab_bar = bg;
        theme.tab_bar_segmented = bg;
        theme.title_bar = bg;
        theme.border = border;
        theme.input = border;
        theme.ring = border;
        theme.sidebar = bg;
        theme.sidebar_foreground = fg;
        theme.sidebar_accent = border;
        theme.sidebar_accent_foreground = fg;
        theme.sidebar_border = border;
        theme.title_bar_border = border;
        theme.window_border = border;
        theme.tab_active = border;
        theme.table_row_border = border;
        theme.mono_font_family = "Adwaita Mono".into();
    });
}

fn main() {
    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);

    app.run(move |cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        apply_app_theme(AppSettings::default().theme, cx);

        let window_options = WindowOptions {
            // Setup GPUI to use custom title bar and client-side decorations for Wayland/X11.
            titlebar: Some(TitlebarOptions {
                title: Some("zedterm".into()),
                ..TitleBar::title_bar_options()
            }),
            app_id: Some("zedterm".into()),
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
