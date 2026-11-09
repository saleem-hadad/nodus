//! The dock of tools and the colour palette (INKUI-001, INKUI-004).

use super::*;

impl BoardCanvas {
    /// INKUI-001: the dock, 16px from the left, its bottom 58px up.
    pub(crate) fn render_ink_dock(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = theme::tokens(cx);
        let tool = self.overlays.ink.tool;
        let color = self.overlays.ink.color(cx);
        let palette = self.overlays.ink.palette;
        let empty = self.ink_strokes().is_empty();
        let me = cx.entity().downgrade();

        let mut children: Vec<AnyElement> = Vec::new();
        for (each, name, key) in TOOLS {
            let me = me.clone();
            let glyph = move |fg: Rgba| match each {
                InkTool::Select => ov_icon(OvIcon::MousePointer2, 15., 2., fg),
                InkTool::Pen => icon(Icon::Pencil, 15., 2., fg),
                InkTool::Arrow => ov_icon(OvIcon::MoveUpRight, 15., 2., fg),
                InkTool::Text => ov_icon(OvIcon::Type, 15., 2., fg),
                InkTool::Eraser => ov_icon(OvIcon::Eraser, 15., 2., fg),
            };
            let title = match key {
                Some(k) => format!("{name} ({k})"),
                None => name.to_owned(),
            };
            children.push(tool_button(
                ("ink-tool", each as usize),
                &t,
                tool == each,
                false,
                title,
                move |el, fg| el.child(glyph(fg)),
                move |window, cx| {
                    if let Some(c) = me.upgrade() {
                        c.update(cx, |c, cx| c.ink_set_tool(each, Some(window), cx));
                    }
                },
                window,
                cx,
            ));
        }
        children.push(div().w(px(16.)).h(px(1.)).my(px(3.)).bg(t.separator).into_any_element());

        // The colour button and its palette.
        let toggle = me.clone();
        let dot = ink_rgba(&t, color);
        let color_button = tool_button(
            "ink-color",
            &t,
            false,
            false,
            "Ink colour".to_owned(),
            move |el, _| {
                el.child(div().size(px(14.)).rounded_full().bg(dot).shadow(vec![inset_shadow(
                    0.,
                    0.,
                    0.,
                    0.5,
                    theme::tokens::rgba(0, 0, 0, 0.25),
                )]))
            },
            move |_, cx| {
                if let Some(c) = toggle.upgrade() {
                    c.update(cx, |c, cx| {
                        c.overlays.ink.palette = !c.overlays.ink.palette;
                        cx.notify();
                    });
                }
            },
            window,
            cx,
        );
        let palette_el = palette.then(|| self.render_ink_palette(&t, color, window, cx));
        let area = me.clone();
        children.push(
            div()
                .relative()
                .child(color_button)
                .children(palette_el)
                .child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, _, cx| {
                            if let Some(c) = area.upgrade() {
                                c.update(cx, |c, _| {
                                    c.overlays.ink.color_area.clear();
                                    c.overlays.ink.color_area.push(bounds);
                                    // The palette: to the right, 168×34 around the button's middle.
                                    c.overlays.ink.color_area.push(Bounds::new(
                                        point(bounds.origin.x + px(38.), bounds.origin.y - px(3.)),
                                        gpui::size(px(168.), px(34.)),
                                    ));
                                });
                            }
                        },
                    )
                    .absolute()
                    .size_full(),
                )
                .into_any_element(),
        );

        let clear = me.clone();
        children.push(tool_button(
            "ink-clear",
            &t,
            false,
            empty,
            "Clear all ink".to_owned(),
            move |el, fg| el.child(icon(Icon::Trash2, 15., 2., fg)),
            move |window, cx| {
                if let Some(c) = clear.upgrade() {
                    c.update(cx, |c, cx| c.ink_clear_all(window, cx));
                }
            },
            window,
            cx,
        ));

        div()
            .id("ink-dock")
            .absolute()
            .left(px(16.))
            .bottom(px(58.))
            .occlude()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(2.))
            .p(px(3.))
            .rounded(px(11.))
            .map(|d| frosted(d, crate::board::hud::rgba_mix(t.bg_elevated, 0.86), 20., 1.6))
            .shadow(t.md())
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                // Pressing the dock never moves the keyboard.
                window.prevent_default();
                cx.stop_propagation();
            })
            .children(children)
            .into_any_element()
    }

    /// INKUI-004: the palette, to the right of the colour button.
    pub(crate) fn render_ink_palette(
        &mut self,
        t: &Tokens,
        current: InkColor,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let rise = enter(window, cx, "ink-palette-in", DUR_FAST, motion::ease_out);
        let me = cx.entity().downgrade();
        let swatches = InkColor::ALL.iter().map(|&c| {
            let me = me.clone();
            let swatch = div()
                .id(("ink-swatch", c as usize))
                .relative()
                .size(px(18.))
                .flex_none()
                .rounded_full()
                .bg(ink_rgba(t, c))
                .shadow(vec![inset_shadow(0., 0., 0., 0.5, theme::tokens::rgba(0, 0, 0, 0.25))])
                .tooltip(native_tooltip(color_name(c).into()))
                .on_click(move |_, _, cx| {
                    if let Some(canvas) = me.upgrade() {
                        canvas.update(cx, |canvas, cx| canvas.ink_pick_color(c, cx));
                    }
                });
            // The current one: a 2px gap, then a 1.5px accent ring, drawn
            // as circles (a spread shadow keeps the box's corners).
            div()
                .relative()
                .size(px(18.))
                .flex_none()
                .when(c == current, |d| {
                    d.child(circle(-3.5, 25., t.accent))
                        .child(circle(-2., 22., t.bg_elevated))
                })
                .child(swatch)
        });
        div()
            .absolute()
            .left(px(38.))
            .top(px(-3.))
            .occlude()
            .flex()
            .gap(px(8.))
            .py(px(8.))
            .px(px(10.))
            .rounded(px(12.))
            .map(|d| frosted(d, crate::board::hud::rgba_mix(t.bg_elevated, 0.92), 24., 1.8))
            .shadow(t.lg())
            .group_opacity(rise)
            .translate(px(-4. * (1. - rise)), px(0.))
            .children(swatches)
            .into_any_element()
    }
}

/// `.ink-tool`: 28×28, radius 8; hover, pressed and on looks.
#[allow(clippy::too_many_arguments)]
pub(crate) fn tool_button(
    id: impl Into<gpui::ElementId>,
    t: &Tokens,
    on: bool,
    disabled: bool,
    title: String,
    content: impl FnOnce(gpui::Stateful<gpui::Div>, Rgba) -> gpui::Stateful<gpui::Div>,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let id: gpui::ElementId = id.into();
    let hovered = window.use_keyed_state((id.clone(), "hover"), cx, |_, _| false);
    let hover = *hovered.read(cx) && !disabled;
    let (bg, fg) = if on {
        (t.bg_selected, t.accent)
    } else if hover {
        (t.bg_hover, t.text_primary)
    } else {
        (crate::components::button::transparent(), t.text_secondary)
    };
    let pressed_bg = t.bg_pressed;
    let el = div()
        .id(id)
        .size(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(8.))
        .bg(bg)
        .text_color(fg)
        .tooltip(native_tooltip(title.into()))
        .when(disabled, |d| d.opacity(0.35))
        .when(!disabled, |d| {
            d.on_hover(move |h, _, cx| {
                hovered.update(cx, |s, cx| {
                    *s = *h;
                    cx.notify();
                });
            })
            .when(!on, |d| d.active(move |s| s.bg(pressed_bg)))
            .on_click(move |_, window, cx| on_click(window, cx))
        });
    content(el, fg).into_any_element()
}

/// A filled circle `size` across, `inset` from its parent's corner.
pub(crate) fn circle(inset: f32, size: f32, color: Rgba) -> gpui::Div {
    div()
        .absolute()
        .left(px(inset))
        .top(px(inset))
        .size(px(size))
        .rounded_full()
        .bg(color)
}
