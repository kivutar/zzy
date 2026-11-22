mod colors;
mod config;
mod settings;
mod tab_bar;
mod terminal;
mod theme;

use colors::{theme_background, theme_foreground};
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    h_flex, v_flex, IconName, Root, Sizable, Theme, ThemeMode, TitleBar,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use settings::{AppSettings, NewTabDirectory, SettingsView};
use terminal::{TerminalBell, TerminalExited, TerminalView};

pub struct ZedTerm {
    tabs: Vec<TabContent>,
    selected_index: usize,
    focus_pending: bool,
    keystroke_sub: Option<Subscription>,
    settings: AppSettings,
    window_state: config::WindowState,
    _window_subscriptions: Vec<Subscription>,
    _save_window_state: Option<Task<()>>,
}

enum TabContent {
    Terminal(Entity<TerminalView>),
    Settings(Entity<SettingsView>),
}

impl ZedTerm {
    fn new(settings: AppSettings, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut zedterm = Self {
            tabs: Vec::new(),
            selected_index: 0,
            focus_pending: true,
            keystroke_sub: None,
            settings,
            window_state: window_state_of(window),
            _window_subscriptions: Vec::new(),
            _save_window_state: None,
        };

        // Remember the window size. Saved shortly after each resize rather than
        // on exit: closing through the title bar removes the window directly
        // (no should-close callback) and the app exits without quit hooks.
        zedterm._window_subscriptions =
            vec![cx.observe_window_bounds(window, |zedterm, window, cx| {
                let state = window_state_of(window);
                if state == zedterm.window_state {
                    return;
                }
                zedterm.window_state = state;
                // Replacing the task cancels the pending save: one write per resize.
                zedterm._save_window_state = Some(cx.spawn(async move |_, cx| {
                    cx.background_executor()
                        .timer(WINDOW_STATE_SAVE_DELAY)
                        .await;
                    config::save_window_state(&state);
                }));
            })];

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
                    view.update(cx, |zedterm, cx| {
                        zedterm.add_terminal_tab(window, cx);
                        zedterm.focus_selected(window, cx);
                    });
                }
                cx.stop_propagation();
            }
        }));

        zedterm.add_terminal_tab(window, cx);
        zedterm
    }

    fn add_terminal_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = self.settings.clone();
        let cwd = self.new_tab_directory(cx);
        let terminal = cx.new(move |cx| TerminalView::new(cx, &settings, cwd));
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
        cx.subscribe_in(&terminal, window, |_, _, _: &TerminalBell, window, _| {
            window.play_system_bell();
        })
        .detach();
        self.tabs.push(TabContent::Terminal(terminal));
        self.selected_index = self.tabs.len() - 1;
        self.focus_pending = true;
        cx.notify();
    }

    /// Where a new tab starts. The first tab (and the fallback) is zedterm's
    /// own working directory, so launching it from a shell opens there.
    fn new_tab_directory(&self, cx: &App) -> Option<std::path::PathBuf> {
        match self.settings.new_tab_directory {
            NewTabDirectory::Home => std::env::var_os("HOME").map(Into::into),
            // The selected tab, or the last terminal tab when Settings is selected.
            NewTabDirectory::Current => self
                .tabs
                .get(self.selected_index)
                .into_iter()
                .chain(self.tabs.iter().rev())
                .find_map(|tab| match tab {
                    TabContent::Terminal(term) => Some(term),
                    TabContent::Settings(_) => None,
                })
                .and_then(|term| term.read(cx).current_dir()),
        }
        .filter(|_| !self.tabs.is_empty())
    }

    fn ensure_settings_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        cx.observe_in(
            &settings_view,
            window,
            |zedterm, settings_view, window, cx| {
                let settings = settings_view.read(cx).current_settings();
                zedterm.apply_settings(settings, window, cx);
            },
        )
        .detach();

        self.tabs.push(TabContent::Settings(settings_view));
        self.selected_index = self.tabs.len() - 1;
        self.focus_pending = true;
        cx.notify();
    }

    /// Apply settings edited in the Settings tab: theme, window background and
    /// every open terminal; then save them.
    fn apply_settings(
        &mut self,
        settings: AppSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if settings.theme != self.settings.theme
            || settings.background_opacity != self.settings.background_opacity
        {
            apply_app_theme(&settings, cx);
        }
        window.set_background_appearance(background_appearance(&settings));
        for tab in &self.tabs {
            if let TabContent::Terminal(term) = tab {
                term.update(cx, |view, cx| view.apply_settings(&settings, window, cx));
            }
        }
        config::save_settings(&settings);
        self.settings = settings;
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
        if self.tabs.is_empty() {
            return v_flex().child("No tabs");
        }

        if self.focus_pending {
            self.focus_selected(window, cx);
            self.focus_pending = false;
        }

        let view = cx.entity();
        // Tint the window background for the selected tab while it runs as root
        // (red) or over SSH (green). Painted under all content, on top of Root's
        // background; default-colored terminal cells don't paint, so it shows through.
        let window_tint = self.context_color(&self.tabs[self.selected_index], cx);
        let header = v_flex()
            .w_full()
            .child(
                // Render custom title bar on top of Root view.
                // The window background (painted once by Root) shows through
                // the title bar, tabs and terminal so translucency is uniform.
                TitleBar::new().bg(transparent_black()).child(
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
                                        view.update(cx, |zedterm, cx| {
                                            zedterm.ensure_settings_tab(window, cx);
                                            zedterm.focus_selected(window, cx);
                                        });
                                    }
                                }),
                        ),
                ),
            )
            .child(self.render_tab_bar(cx));

        v_flex()
            .size_full()
            .when_some(window_tint, |this, color| this.bg(color.opacity(0.1)))
            .child(header)
            .child(
                v_flex()
                    .id("window-body")
                    .flex_1()
                    .w_full()
                    .h_full()
                    // Configurable padding for terminals; the Settings page is
                    // laid out edge to edge (as it was before padding existed).
                    .when(
                        matches!(self.tabs[self.selected_index], TabContent::Terminal(_)),
                        |this| this.p(px(self.settings.padding)),
                    )
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
fn apply_app_theme(settings: &AppSettings, cx: &mut App) {
    let choice = settings.theme;
    let opacity = settings.background_opacity;
    let bg: Hsla = theme_background(choice).into();
    let fg: Hsla = theme_foreground(choice).into();
    let border: Hsla = colors::theme_border(choice).into();
    // Root paints `background` over the whole window: it is the single
    // translucent layer. Surfaces drawn on top of it with the same color are
    // made transparent, otherwise stacked translucent layers would get darker.
    let window_bg = bg.opacity(opacity);
    let clear = transparent_black();
    Theme::update(cx, |theme| {
        theme.background = window_bg;
        theme.secondary = window_bg;
        theme.tab_bar = clear;
        theme.tab_bar_segmented = clear;
        theme.title_bar = clear;
        theme.sidebar = clear;
        // The active tab shows the plain window background; its side borders and
        // the gap in the line under the tab bar mark it (see the tab bar in render).
        theme.tab_active = clear;
        theme.foreground = fg;
        theme.secondary_foreground = fg;
        theme.tab_foreground = fg;
        theme.sidebar_foreground = fg;
        theme.sidebar_accent_foreground = fg;
        theme.sidebar_accent = border.opacity(opacity);
        theme.border = border;
        theme.input = border;
        theme.ring = border;
        theme.sidebar_border = border;
        theme.title_bar_border = border;
        theme.window_border = border;
        theme.table_row_border = border;
        theme.mono_font_family = settings::DEFAULT_FONT_FAMILY.into();
    });
}

const WINDOW_STATE_SAVE_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

fn window_state_of(window: &Window) -> config::WindowState {
    let (bounds, maximized) = match window.window_bounds() {
        WindowBounds::Windowed(bounds) => (bounds, false),
        WindowBounds::Maximized(bounds) => (bounds, true),
        // Don't reopen in fullscreen; keep the restore size.
        WindowBounds::Fullscreen(bounds) => (bounds, false),
    };
    config::WindowState {
        width: bounds.size.width.into(),
        height: bounds.size.height.into(),
        maximized,
    }
}

/// Blur what's behind the window when it is translucent (KDE/Wayland, macOS).
fn background_appearance(settings: &AppSettings) -> WindowBackgroundAppearance {
    if settings.background_blur && settings.background_opacity < 1.0 {
        WindowBackgroundAppearance::Blurred
    } else {
        WindowBackgroundAppearance::Transparent
    }
}

fn main() {
    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);

    app.run(move |cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        let settings = config::load_settings();
        let window_state = config::load_window_state();
        apply_app_theme(&settings, cx);

        let restore_bounds = Bounds::centered(
            None,
            size(px(window_state.width), px(window_state.height)),
            cx,
        );

        let window_options = WindowOptions {
            // Setup GPUI to use custom title bar and client-side decorations for Wayland/X11.
            titlebar: Some(TitlebarOptions {
                title: Some("zedterm".into()),
                ..TitleBar::title_bar_options()
            }),
            app_id: Some("zedterm".into()),
            window_decorations: Some(WindowDecorations::Client),
            window_background: background_appearance(&settings),
            window_bounds: Some(if window_state.maximized {
                WindowBounds::Maximized(restore_bounds)
            } else {
                WindowBounds::Windowed(restore_bounds)
            }),
            ..Default::default()
        };

        cx.spawn(async move |cx| {
            cx.open_window(window_options, |window, cx| {
                let view = cx.new(|cx| ZedTerm::new(settings, window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })?;

            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });
}
