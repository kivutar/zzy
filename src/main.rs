mod colors;
mod config;
mod settings;
mod terminal;
mod theme;

use colors::{theme_background, theme_foreground};
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    tab::{Tab, TabBar},
    v_flex, ActiveTheme, Icon, IconName, Root, Sizable, Theme, ThemeMode, TitleBar,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use settings::{AppSettings, NewTabDirectory, SettingsView};
use std::{cell::Cell, rc::Rc};
use terminal::{ProcessContext, TerminalBell, TerminalExited, TerminalView};

pub struct ZedTerm {
    tabs: Vec<TabContent>,
    selected_index: usize,
    focus_pending: bool,
    keystroke_sub: Option<Subscription>,
    settings: AppSettings,
    settings_dirty: bool,
    window_state: config::WindowState,
    /// A terminal rang the bell; played from `render`, which has the window.
    bell_pending: bool,
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
            settings_dirty: false,
            window_state: window_state_of(window),
            bell_pending: false,
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
        cx.subscribe(&terminal, |zedterm, _, _: &TerminalBell, cx| {
            zedterm.bell_pending = true;
            cx.notify();
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
            if settings.theme != zedterm.settings.theme
                || settings.background_opacity != zedterm.settings.background_opacity
            {
                apply_app_theme(&settings, cx);
            }
            config::save_settings(&settings);
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

        window.set_background_appearance(background_appearance(&self.settings));

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
        if std::mem::take(&mut self.bell_pending) {
            window.play_system_bell();
        }

        if self.tabs.is_empty() {
            return v_flex().child("No tabs");
        }

        if self.focus_pending {
            self.focus_selected(window, cx);
            self.focus_pending = false;
        }

        let view = cx.entity();
        let context_color = |tab: &TabContent, cx: &App| match tab {
            TabContent::Terminal(term) => match term.read(cx).process_context() {
                ProcessContext::Root if self.settings.tint_root => ProcessContext::Root.color(),
                ProcessContext::Remote if self.settings.tint_ssh => ProcessContext::Remote.color(),
                _ => None,
            },
            TabContent::Settings(_) => None,
        };
        // Tint the window background for the selected tab while it runs as root
        // (red) or over SSH (green). Painted under all content, on top of Root's
        // background; default-colored terminal cells don't paint, so it shows through.
        let window_tint = context_color(&self.tabs[self.selected_index], cx);
        let border_color = cx.theme().border;
        let active_left: Rc<Cell<Option<Pixels>>> = Rc::default();
        let active_right: Rc<Cell<Option<Pixels>>> = Rc::default();
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
                                        let _ = view.update(cx, |zedterm, cx| {
                                            zedterm.ensure_settings_tab(cx);
                                            zedterm.focus_selected(window, cx);
                                        });
                                    }
                                }),
                        ),
                ),
            )
            // Connected active tab: gpui-kit draws a full-width line under the tab
            // bar and expects an opaque active tab to cover it, which would break
            // translucency. Instead the bar is clipped 1px short (hiding its line)
            // and we draw the line ourselves, leaving a gap under the active tab.
            .child(
                div().h(px(TAB_HEIGHT - 1.)).overflow_hidden().child(
                    TabBar::new("tabs")
                        .w_full()
                        .flex_shrink_0()
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
                            // Background tabs keep their own context color on the icon.
                            let icon = Icon::new(icon)
                                .when_some(context_color(tab, cx), |icon, color| {
                                    icon.text_color(color)
                                });
                            // The active tab reports its outer edges (content edge plus
                            // our padding and gpui-kit's 1px side border) so the line
                            // under the tab bar can leave a gap there. Recorded during
                            // prepaint, read when the line paints later in the frame.
                            let selected = index == self.selected_index;
                            let measure = |cell: &Rc<Cell<Option<Pixels>>>, right_edge: bool| {
                                let cell = cell.clone();
                                canvas(
                                    move |bounds, _, _| {
                                        let outset = px(TAB_PADDING_X + 1.);
                                        cell.set(Some(if right_edge {
                                            bounds.right() + outset
                                        } else {
                                            bounds.left() - outset
                                        }));
                                    },
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .size_full()
                            };
                            let icon = div()
                                .child(icon)
                                .when(selected, |this| this.child(measure(&active_left, false)));
                            // gpui-kit paints inactive tab borders transparent and its
                            // state styles override ours, so draw a 1px separator on the
                            // tab's right edge ourselves, inside the suffix (tab children
                            // go into a clipped inner box). Taffy positions absolute
                            // elements against their direct parent, so the suffix box is
                            // made as tall as the tab (`TAB_HEIGHT`) and the line
                            // reaches past our `TAB_PADDING_X` padding
                            // to the tab edge. Skipped next to the selected tab, whose
                            // own border already separates it.
                            let separator =
                                index != self.selected_index && index + 1 != self.selected_index;
                            let close_button = Button::new(("close-tab", index as u32))
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
                                });
                            Tab::new()
                                .prefix(icon)
                                .label(label)
                                .px(px(TAB_PADDING_X))
                                .cursor(CursorStyle::PointingHand)
                                .suffix(
                                    h_flex()
                                        .h(px(TAB_HEIGHT))
                                        .child(close_button)
                                        .when(selected, |this| {
                                            this.child(measure(&active_right, true))
                                        })
                                        .when(separator, |this| {
                                            this.child(
                                                div()
                                                    .absolute()
                                                    .top_0()
                                                    .bottom_0()
                                                    .right(px(-TAB_PADDING_X))
                                                    .w(px(1.))
                                                    .bg(border_color),
                                            )
                                        }),
                                )
                        })),
                ),
            )
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        let (left, right) = (bounds.left(), bounds.right());
                        let segments = match (active_left.get(), active_right.get()) {
                            // Each segment also covers the active tab's 1px side
                            // border column, closing the corner where they meet.
                            (Some(gap_start), Some(gap_end)) => vec![
                                (left, (gap_start + px(1.)).clamp(left, right)),
                                ((gap_end - px(1.)).clamp(left, right), right),
                            ],
                            _ => vec![(left, right)],
                        };
                        for (start, end) in segments {
                            if end > start {
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(start, bounds.top()),
                                        size(end - start, px(1.)),
                                    ),
                                    border_color,
                                ));
                            }
                        }
                    },
                )
                .w_full()
                .h(px(1.)),
            );

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
        theme.mono_font_family = "Adwaita Mono".into();
    });
}

/// gpui-kit's default `TabVariant::Tab` height and our horizontal tab padding,
/// used to place the inactive-tab separator on the tab's edge.
const TAB_HEIGHT: f32 = 32.0;
const TAB_PADDING_X: f32 = 8.0;

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
