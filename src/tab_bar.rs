//! The tab bar, with two workarounds for gpui-kit's `TabVariant::Tab` styling
//! that keep the window's translucency uniform:
//!
//! - gpui-kit paints inactive tab borders transparent (and its state styles
//!   override ours), so inactive tabs draw their own 1px separator.
//! - gpui-kit expects an opaque active tab to cover the line under the tab bar.
//!   Instead the bar is clipped 1px short (hiding its line) and we paint the
//!   line ourselves, leaving a gap under the active tab ("connected" tab).
//!
//! - The bar is shifted 1px left (clipped) so an active first tab's left
//!   border doesn't double the window border.
//!
//! If gpui-kit gains a theme color for inactive tab borders, most of this can go.

use crate::{TabContent, Zzy};
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    tab::{Tab, TabBar},
    v_flex, ActiveTheme, Icon, IconName, Sizable,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::{cell::Cell, rc::Rc};

/// gpui-kit's default `TabVariant::Tab` height and our horizontal tab padding,
/// used to place the separators and the line's gap on the tab edges.
pub(crate) const TAB_HEIGHT: f32 = 32.0;
const TAB_PADDING_X: f32 = 8.0;

/// Left and right edges for the line under the tabs, recorded during prepaint.
pub(crate) type LineExtent = Rc<Cell<Option<(Pixels, Pixels)>>>;

/// The payload of a tab being dragged to a new position.
#[derive(Clone)]
struct DraggedTab {
    index: usize,
    label: SharedString,
    icon: IconName,
}

/// What follows the cursor while a tab is dragged.
struct TabDragPreview(DraggedTab);

impl Render for TabDragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_2()
            .px_3()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .text_sm()
            .shadow_md()
            .child(Icon::new(self.0.icon.clone()))
            .child(self.0.label.clone())
    }
}

impl Zzy {
    /// Red while the tab's program runs as root, green during SSH (when the
    /// respective setting is on); `None` otherwise.
    pub(crate) fn context_color(&self, tab: &TabContent, cx: &App) -> Option<Hsla> {
        use crate::terminal::ProcessContext;
        match tab {
            TabContent::Terminal(term) => match term.read(cx).process_context() {
                ProcessContext::Root if self.settings.tint_root => ProcessContext::Root.color(),
                ProcessContext::Remote if self.settings.tint_ssh => ProcessContext::Remote.color(),
                _ => None,
            },
            TabContent::Settings(_) => None,
        }
    }

    /// The tab bar. In the title bar (`line_extent` set), the line under the
    /// tabs spans the whole title bar, whose edges are recorded there during
    /// prepaint, and tabs keep their presses from moving the window.
    pub(crate) fn render_tab_bar(
        &self,
        line_extent: Option<LineExtent>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let in_title_bar = line_extent.is_some();
        let view = cx.entity();
        let border_color = cx.theme().border;
        let drop_target = border_color.opacity(0.5);
        // Outer edges of the active tab, recorded during prepaint and read when
        // the line under the bar paints later in the same frame.
        let active_left: Rc<Cell<Option<Pixels>>> = Rc::default();
        let active_right: Rc<Cell<Option<Pixels>>> = Rc::default();
        let context_color = |tab: &TabContent, cx: &App| self.context_color(tab, cx);

        v_flex()
            .w_full()
            // Connected active tab: gpui-kit draws a full-width line under the tab
            // bar and expects an opaque active tab to cover it, which would break
            // translucency. Instead the bar is clipped 1px short (hiding its line)
            // and we draw the line ourselves, leaving a gap under the active tab.
            .child(
                div().h(px(TAB_HEIGHT - 1.)).overflow_hidden().child(
                    TabBar::new("tabs")
                        .w_full()
                        .flex_shrink_0()
                        // Shifted 1px left so the first tab's left border falls in
                        // the clip: when it's the active tab, its border would
                        // otherwise double the window border next to it.
                        .ml(px(-1.))
                        .selected_index(self.selected_index)
                        .on_click({
                            let view = view.clone();
                            move |selected_index, window, cx| {
                                // In the title bar, a double click on a tab must
                                // not reach the bar (which maximizes the window).
                                cx.stop_propagation();
                                let view = view.clone();
                                view.update(cx, |zzy, cx| {
                                    zzy.select_tab(*selected_index, cx);
                                    zzy.focus_selected(window, cx);
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
                                    view.update(cx, |zzy, cx| {
                                        zzy.add_terminal_tab(window, cx);
                                        zzy.focus_selected(window, cx);
                                    });
                                })
                        })
                        .children(self.tabs.iter().enumerate().map(|(index, tab)| {
                            let close_view = view.clone();
                            let (label, tab_icon) = match tab {
                                TabContent::Terminal(term) => {
                                    (term.read(cx).tab_title(), IconName::SquareTerminal)
                                }
                                TabContent::Settings(_) => (
                                    crate::i18n::tr("tab.settings").to_string(),
                                    IconName::Settings2,
                                ),
                            };
                            // Background tabs keep their own context color on the icon.
                            let icon = Icon::new(tab_icon.clone())
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
                                    view.update(cx, |zzy, cx| {
                                        zzy.close_tab(index, cx);
                                        zzy.focus_selected(window, cx);
                                    });
                                });
                            let dragged = DraggedTab {
                                index,
                                label: label.clone().into(),
                                icon: tab_icon,
                            };
                            Tab::new()
                                .prefix(icon)
                                .label(label)
                                .px(px(TAB_PADDING_X))
                                .cursor(CursorStyle::PointingHand)
                                // The title bar moves the window on a press and drag
                                // unless the press was claimed, as buttons do; a tab
                                // claims it so dragging reorders tabs instead.
                                .when(in_title_bar, |this| {
                                    this.on_mouse_down(MouseButton::Left, |_, window, _| {
                                        window.prevent_default()
                                    })
                                })
                                // Drag to reorder: the dropped tab takes this tab's place.
                                .on_drag(dragged, |dragged, _, _, cx| {
                                    cx.new(|_| TabDragPreview(dragged.clone()))
                                })
                                .drag_over::<DraggedTab>(move |style, dragged, _, _| {
                                    if dragged.index == index {
                                        style
                                    } else {
                                        style.bg(drop_target)
                                    }
                                })
                                .on_drop({
                                    let view = view.clone();
                                    move |dragged: &DraggedTab, window, cx| {
                                        view.update(cx, |zzy, cx| {
                                            zzy.move_tab(dragged.index, index, cx);
                                            zzy.focus_selected(window, cx);
                                        });
                                    }
                                })
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
                        let (left, right) = line_extent
                            .as_ref()
                            .and_then(|extent| extent.get())
                            .unwrap_or((bounds.left(), bounds.right()));
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
            )
    }
}
