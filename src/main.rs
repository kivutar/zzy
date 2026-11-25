// A GUI app: no console window next to it on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod colors;
mod config;
mod i18n;
mod settings;
mod tab_bar;
mod terminal;
mod theme;

use colors::{theme_background, theme_foreground};
use gpui_kit::component::{
    h_flex, tooltip::Tooltip, v_flex, window_corner_radii, ActiveTheme, Icon, IconName, Root,
    Sizable, Theme, ThemeMode, TitleBar, TITLE_BAR_HEIGHT,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use settings::{AppSettings, NewTabDirectory, SettingsView};
use terminal::{TerminalBell, TerminalExited, TerminalView};
use theme::ThemeChoice;

pub struct Zzy {
    tabs: Vec<TabContent>,
    selected_index: usize,
    focus_pending: bool,
    keystroke_sub: Option<Subscription>,
    settings: AppSettings,
    /// Whether the desktop prefers a dark appearance (for `Appearance::System`).
    system_dark: bool,
    window_state: config::WindowState,
    _window_subscriptions: Vec<Subscription>,
    _save_window_state: Option<Task<()>>,
}

enum TabContent {
    Terminal(Entity<TerminalView>),
    Settings(Entity<SettingsView>),
}

impl Zzy {
    fn new(settings: AppSettings, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut zzy = Self {
            tabs: Vec::new(),
            selected_index: 0,
            focus_pending: true,
            keystroke_sub: None,
            settings,
            system_dark: is_dark(window.appearance()),
            window_state: window_state_of(window),
            _window_subscriptions: Vec::new(),
            _save_window_state: None,
        };

        // Remember the window size. Saved shortly after each resize rather than
        // on exit: closing through the title bar removes the window directly
        // (no should-close callback) and the app exits without quit hooks.
        // The theme follows the desktop's light/dark preference when asked to.
        let appearance = cx.observe_window_appearance(window, |zzy, window, cx| {
            let system_dark = is_dark(window.appearance());
            if system_dark != zzy.system_dark {
                let old_theme = zzy.theme();
                zzy.system_dark = system_dark;
                if zzy.theme() != old_theme {
                    zzy.apply_theme(window, cx);
                }
            }
        });
        zzy._window_subscriptions = vec![
            appearance,
            cx.observe_window_bounds(window, |zzy, window, cx| {
                // The fullscreen button's icon follows the window state.
                cx.notify();
                let state = window_state_of(window);
                if state == zzy.window_state {
                    return;
                }
                zzy.window_state = state;
                // Replacing the task cancels the pending save: one write per resize.
                zzy._save_window_state = Some(cx.spawn(async move |_, cx| {
                    cx.background_executor()
                        .timer(WINDOW_STATE_SAVE_DELAY)
                        .await;
                    config::save_window_state(&state);
                }));
            }),
        ];

        let view = cx.entity().downgrade();
        zzy.keystroke_sub = Some(cx.intercept_keystrokes(move |event, window, cx| {
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
                    view.update(cx, |zzy, cx| {
                        zzy.add_terminal_tab(window, cx);
                        zzy.focus_selected(window, cx);
                    });
                }
                cx.stop_propagation();
            }
        }));

        // `main` themed the app before the window knew the desktop's preference.
        if zzy.theme() != zzy.settings.theme(true) {
            apply_app_theme(zzy.theme(), &zzy.settings, cx);
        }
        zzy.add_terminal_tab(window, cx);
        zzy
    }

    fn add_terminal_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = self.settings.clone();
        let theme = self.theme();
        let cwd = self.new_tab_directory(cx);
        let terminal = cx.new(|cx| TerminalView::new(window, cx, &settings, theme, cwd));
        cx.observe(&terminal, |_zzy, _terminal, cx| {
            // Propagate terminal changes (e.g., title updates) to the tab bar.
            cx.notify();
        })
        .detach();
        cx.subscribe(&terminal, |zzy, terminal, _: &TerminalExited, cx| {
            let index = zzy
                .tabs
                .iter()
                .position(|tab| matches!(tab, TabContent::Terminal(t) if *t == terminal));
            if let Some(index) = index {
                zzy.close_tab(index, cx);
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

    /// Where a new tab starts. The first tab (and the fallback) is zzy's
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
        cx.observe_in(&settings_view, window, |zzy, settings_view, window, cx| {
            let settings = settings_view.read(cx).current_settings();
            zzy.apply_settings(settings, window, cx);
        })
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
        let theme_changed = settings.theme(self.system_dark) != self.theme()
            || settings.background_opacity != self.settings.background_opacity
            || settings.ui_font_family != self.settings.ui_font_family
            || settings.ui_font_size != self.settings.ui_font_size
            || settings.window_radius != self.settings.window_radius;
        if theme_changed {
            apply_app_theme(settings.theme(self.system_dark), &settings, cx);
        }
        if settings.language != self.settings.language {
            i18n::apply(settings.language);
            // Every view shows translated text, not only this one.
            cx.refresh_windows();
        }
        window.set_background_appearance(background_appearance(&settings));
        for tab in &self.tabs {
            if let TabContent::Terminal(term) = tab {
                let theme = settings.theme(self.system_dark);
                term.update(cx, |view, cx| {
                    view.apply_settings(&settings, theme, window, cx)
                });
            }
        }
        config::save_settings(&settings);
        self.settings = settings;
        cx.notify();
    }

    /// The color theme in use: the settings' dark or light one.
    fn theme(&self) -> ThemeChoice {
        self.settings.theme(self.system_dark)
    }

    /// Re-theme the app and every terminal, after the desktop switched
    /// between light and dark.
    fn apply_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let theme = self.theme();
        apply_app_theme(theme, &self.settings, cx);
        let settings = self.settings.clone();
        for tab in &self.tabs {
            if let TabContent::Terminal(term) = tab {
                term.update(cx, |view, cx| {
                    view.apply_settings(&settings, theme, window, cx)
                });
            }
        }
        cx.notify();
    }

    /// Move the tab at `from` to position `to` (drag to reorder). The selected
    /// tab stays selected wherever it ends up.
    fn move_tab(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if from == to || from >= self.tabs.len() || to >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        self.selected_index = moved_index(self.selected_index, from, to);
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

impl Render for Zzy {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.tabs.is_empty() {
            return v_flex().child(i18n::tr("tab.none"));
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
        // Settings and fullscreen buttons, at the end of the title bar.
        let in_title_bar = self.settings.tabs_in_title_bar;
        let fullscreen = window.is_fullscreen();
        // Buttons at the end of the title bar, next to the window controls. With
        // the tabs in the title bar, "+" moves here from the tab bar.
        let title_buttons = h_flex()
            .h_full()
            .when(in_title_bar, |this| {
                let view = view.clone();
                this.child(title_bar_button(
                    "add-tab",
                    IconName::Plus,
                    i18n::tr("tab.new"),
                    move |window, cx| {
                        view.update(cx, |zzy, cx| {
                            zzy.add_terminal_tab(window, cx);
                            zzy.focus_selected(window, cx);
                        })
                    },
                    cx,
                ))
            })
            .child(title_bar_button(
                "open-settings",
                IconName::Settings2,
                i18n::tr("tab.settings"),
                {
                    let view = view.clone();
                    move |window, cx| {
                        view.update(cx, |zzy, cx| {
                            zzy.ensure_settings_tab(window, cx);
                            zzy.focus_selected(window, cx);
                        })
                    }
                },
                cx,
            ))
            .child(title_bar_button(
                "toggle-fullscreen",
                if fullscreen {
                    IconName::Minimize
                } else {
                    IconName::Maximize
                },
                if fullscreen {
                    i18n::tr("window.exit_fullscreen")
                } else {
                    i18n::tr("window.fullscreen")
                },
                |window, _| window.toggle_fullscreen(),
                cx,
            ));
        // Render custom title bar on top of Root view.
        // The window background (painted once by Root) shows through
        // the title bar, tabs and terminal so translucency is uniform.
        let title_bar = TitleBar::new().bg(transparent_black());
        let header = if in_title_bar {
            // The tabs' own line (with its gap under the active tab) replaces the
            // title bar's border, spanning the whole bar: its edges are recorded
            // here during prepaint, before the line paints.
            let extent = tab_bar::LineExtent::default();
            let record = extent.clone();
            v_flex()
                .w_full()
                .child(
                    canvas(
                        move |bounds, _, _| record.set(Some((bounds.left(), bounds.right()))),
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
                .child(
                    title_bar
                        .border_b(px(0.))
                        // As tall as the tabs, whose borders then reach the top
                        // edge, and no start padding: the first tab sits at the
                        // window edge as in the tab bar of its own.
                        .h(px(tab_bar::TAB_HEIGHT))
                        .pl_0()
                        .child(
                            h_flex()
                                .size_full()
                                // Tabs sit on the bar's bottom edge, like a tab bar.
                                .child(
                                    v_flex()
                                        .h_full()
                                        .flex_1()
                                        .min_w_0()
                                        .justify_end()
                                        .child(self.render_tab_bar(Some(extent), cx)),
                                )
                                .child(title_buttons),
                        ),
                )
        } else {
            v_flex()
                .w_full()
                .child(
                    title_bar.child(
                        h_flex()
                            .size_full()
                            .justify_between()
                            .child("zzy")
                            .child(title_buttons),
                    ),
                )
                .child(self.render_tab_bar(None, cx))
        };

        // The part of the window border we draw, inside gpui-kit's frame.
        let border = border_widths(&self.settings, window);
        // Rounded like the frame's corners, one pixel in (inside its line).
        let radii = window_corner_radii(window, cx);
        let inner = |radius: Pixels| (radius - px(1.)).max(px(0.));

        v_flex()
            .size_full()
            .when_some(window_tint, |this, color| this.bg(color.opacity(0.1)))
            .border_color(cx.theme().window_border)
            .border_t(border.top)
            .border_b(border.bottom)
            .border_l(border.left)
            .border_r(border.right)
            .rounded_tl(inner(radii.top_left))
            .rounded_tr(inner(radii.top_right))
            .rounded_bl(inner(radii.bottom_left))
            .rounded_br(inner(radii.bottom_right))
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

/// Where the tab at `index` ends up after the tab at `from` moves to `to`.
fn moved_index(index: usize, from: usize, to: usize) -> usize {
    if index == from {
        to
    } else if from < index && index <= to {
        index - 1
    } else if to <= index && index < from {
        index + 1
    } else {
        index
    }
}

/// Override the component theme with the terminal palette.
///
/// Must go through `Theme::update` so the render tokens (e.g. `tokens.tab_active`)
/// stay in sync with `colors`; it also refreshes all windows, so don't call it
/// from `render`.
fn apply_app_theme(choice: ThemeChoice, settings: &AppSettings, cx: &mut App) {
    // gpui-kit's own widgets (inputs, switches, menus) in the matching mode;
    // the colors below are then set on top of it.
    Theme::change(
        if choice.is_dark() {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        },
        None,
        cx,
    );
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
        // Inactive tabs are dimmed; the active (and hovered) tab uses full text color.
        theme.tab_foreground = fg.opacity(0.6);
        theme.tab_active_foreground = fg;
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
        // gpui-kit sizes its controls in rems of `font_size`: it scales them too.
        theme.font_family = settings.ui_font_family.clone();
        theme.font_size = px(settings.ui_font_size);
        // Rounds the window's free corners (gpui-kit squares tiled ones).
        theme.window_radius = px(settings.window_radius);
    });
}

fn is_dark(appearance: WindowAppearance) -> bool {
    matches!(
        appearance,
        WindowAppearance::Dark | WindowAppearance::VibrantDark
    )
}

/// How an edge of the window is framed before we draw anything.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
enum EdgeFrame {
    /// gpui-kit's fixed 1px border.
    #[default]
    Kit,
    /// Nothing: a tiled edge (maximized, snapped), or fullscreen.
    Bare,
    /// The window manager's frame (server-side decorations).
    WindowManager,
}

fn edge_frames(window: &Window) -> Edges<EdgeFrame> {
    let frame = |tiled: bool| {
        if tiled {
            EdgeFrame::Bare
        } else {
            EdgeFrame::Kit
        }
    };
    match window.window_decorations() {
        Decorations::Client { tiling } => Edges {
            top: frame(tiling.top),
            bottom: frame(tiling.bottom),
            left: frame(tiling.left),
            right: frame(tiling.right),
        },
        Decorations::Server if window.is_fullscreen() => Edges::all(EdgeFrame::Bare),
        Decorations::Server => Edges::all(EdgeFrame::WindowManager),
    }
}

/// The border we draw on each edge, in the theme's window border color, so
/// that every edge shows `border_width` in all: the rest of it inside
/// gpui-kit's 1px line, or all of it on bare edges when asked to.
fn border_widths(settings: &AppSettings, window: &Window) -> Edges<Pixels> {
    let frames = edge_frames(window);
    let width = |frame| px(border_width(settings, frame));
    Edges {
        top: width(frames.top),
        bottom: width(frames.bottom),
        left: width(frames.left),
        right: width(frames.right),
    }
}

fn border_width(settings: &AppSettings, frame: EdgeFrame) -> f32 {
    match frame {
        EdgeFrame::Kit => (settings.border_width - 1.0).max(0.0),
        EdgeFrame::Bare if settings.border_when_maximized => settings.border_width,
        EdgeFrame::Bare | EdgeFrame::WindowManager => 0.0,
    }
}

/// A title bar button drawn like gpui-kit's window controls (a full-height
/// square, small icon, secondary hover colors), so the buttons next to
/// minimize and maximize read as one set.
fn title_bar_button(
    id: &'static str,
    icon: IconName,
    tooltip: SharedString,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let (hover_bg, active_bg, hover_fg) = (
        theme.secondary_hover,
        theme.secondary_active,
        theme.secondary_foreground,
    );
    div()
        .id(id)
        .flex()
        .w(TITLE_BAR_HEIGHT)
        .h_full()
        .flex_shrink_0()
        .justify_center()
        .items_center()
        .text_color(theme.foreground)
        .hover(move |style| style.bg(hover_bg).text_color(hover_fg))
        .active(move |style| style.bg(active_bg).text_color(hover_fg))
        // Like the window controls: the press never moves the window, and
        // the click (or a double click) never reaches the title bar.
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.prevent_default();
            cx.stop_propagation();
        })
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_click(window, cx);
        })
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(Icon::new(icon).small())
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

rust_i18n::i18n!("locales", fallback = "en");

fn main() {
    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);

    app.run(move |cx| {
        gpui_kit::init(cx);
        let settings = config::load_settings();
        i18n::apply(settings.language);
        let window_state = config::load_window_state();
        // Assume dark until the window knows the desktop's preference; `Zzy::new`
        // re-themes if it turns out light.
        apply_app_theme(settings.theme(true), &settings, cx);

        let restore_bounds = Bounds::centered(
            None,
            size(px(window_state.width), px(window_state.height)),
            cx,
        );

        let window_options = WindowOptions {
            // Setup GPUI to use custom title bar and client-side decorations for Wayland/X11.
            titlebar: Some(TitlebarOptions {
                title: Some("zzy".into()),
                ..TitleBar::title_bar_options()
            }),
            app_id: Some("zzy".into()),
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
                let view = cx.new(|cx| Zzy::new(settings, window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })?;

            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });
}

#[cfg(test)]
mod tests {
    use super::{border_width, moved_index, AppSettings, EdgeFrame};

    #[test]
    fn border_adds_up_to_the_configured_width() {
        let settings = |border_width, border_when_maximized| AppSettings {
            border_width,
            border_when_maximized,
            ..Default::default()
        };
        // gpui-kit's 1px line plus ours.
        assert_eq!(border_width(&settings(1.0, false), EdgeFrame::Kit), 0.0);
        assert_eq!(border_width(&settings(4.0, false), EdgeFrame::Kit), 3.0);
        // Bare (maximized, fullscreen) edges only when asked.
        assert_eq!(border_width(&settings(4.0, false), EdgeFrame::Bare), 0.0);
        assert_eq!(border_width(&settings(4.0, true), EdgeFrame::Bare), 4.0);
        // Never on top of the window manager's frame.
        assert_eq!(
            border_width(&settings(4.0, true), EdgeFrame::WindowManager),
            0.0
        );
    }

    /// Simulates the move on a list and checks `moved_index` against it.
    #[test]
    fn moved_index_follows_every_tab() {
        for len in 1..6 {
            for from in 0..len {
                for to in 0..len {
                    let mut tabs: Vec<usize> = (0..len).collect();
                    let tab = tabs.remove(from);
                    tabs.insert(to, tab);
                    for index in 0..len {
                        let new_index = tabs.iter().position(|&t| t == index).unwrap();
                        assert_eq!(moved_index(index, from, to), new_index, "{from}->{to}");
                    }
                }
            }
        }
    }
}
