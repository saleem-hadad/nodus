//! One folder in the Finder-style grid (`FolderItem.tsx`; UI-230…UI-240):
//! icon, live badges, focus mark, label (or its rename field) and subtitle.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Div, ElementId, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use nodus_domain::{Project, ProjectStats, SessionStatus};

use crate::components::folder_icon::{folder_icon, project_glyph};
use crate::components::icon::{Icon, icon};
use crate::components::status_dot::status_dot;
use crate::components::tooltip::native_tooltip;
use crate::components::tween::transition;
use crate::theme::tokens::*;
use crate::theme::{self, fonts, motion};

/// What a folder shows.
pub struct FolderItem<'a> {
    pub project: &'a Project,
    pub stats: Option<&'a ProjectStats>,
    pub selected: bool,
    /// One of the projects in focus (UI-201).
    pub focused: bool,
    /// Shown while focusing only because a session in it waits (UI-200).
    pub dimmed: bool,
    /// The rename field, when renaming.
    pub rename: Option<AnyElement>,
    /// Popping in: 0 → 1 (UI-230), or 1.
    pub pop: f32,
    /// The pointer is pressed on it (UI-231).
    pub pressed: bool,
    /// A server project's badge (SRV-162); none for a local one.
    pub server: Option<ServerBadge>,
}

/// The badge on a server project's folder: its server's name for the
/// tooltip, dimmed while the server is away (SRV-162).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerBadge {
    pub name: String,
    pub away: bool,
}

impl ServerBadge {
    pub fn tooltip(&self) -> String {
        if self.away {
            format!("On {} · Reconnecting…", self.name)
        } else {
            format!("On {}", self.name)
        }
    }
}

/// The subtitle (UI-233): a blank line while stats load, else the counts.
pub fn subtitle(stats: Option<&ProjectStats>) -> String {
    let Some(stats) = stats else {
        return "\u{a0}".into();
    };
    if stats.total == 0 {
        return "No sessions".into();
    }
    let total = format!(
        "{} {}",
        stats.total,
        if stats.total == 1 { "session" } else { "sessions" }
    );
    if stats.active > 0 {
        format!("{total} · {} active", stats.active)
    } else {
        total
    }
}

impl FolderItem<'_> {
    /// The item, 144px wide. `id` keys its hover state.
    pub fn render(self, id: &ElementId, hovered: &Entity<bool>, window: &mut Window, cx: &mut App) -> Div {
        let t = theme::tokens(cx);
        let p = self.project;
        let faded = p.archived == Some(true) || self.dimmed;
        let is_hovered = *hovered.read(cx);
        // UI-231: the drawing lifts 1px over 220ms when hovered.
        let lift = transition(
            window,
            cx,
            (id.clone(), "lift"),
            if is_hovered { -1.0 } else { 0.0 },
            DUR_MED,
            motion::ease_out,
        );
        // UI-236: faded folders at 42%, 70% while hovered or selected.
        let opacity = transition(
            window,
            cx,
            (id.clone(), "fade"),
            if !faded {
                1.0
            } else if is_hovered || self.selected {
                0.7
            } else {
                0.42
            },
            DUR_FAST,
            motion::ease,
        );
        let box_bg = transition(
            window,
            cx,
            (id.clone(), "box"),
            if self.selected {
                t.bg_pressed
            } else {
                crate::components::button::transparent()
            },
            DUR_FAST,
            motion::ease,
        );
        let label_color = if self.selected {
            t.text_on_accent
        } else if faded {
            t.text_secondary
        } else {
            t.text_primary
        };

        let mut folder = folder_icon(112.0, Some(p.color)).glyph(project_glyph(&p.name));
        if faded {
            folder = folder.saturate(0.4);
        }

        let badges = self.stats.filter(|s| s.working > 0 || s.waiting > 0).map(|s| {
            div()
                .absolute()
                .top(px(18.))
                .right(px(2.))
                .flex()
                .flex_col()
                .items_end()
                .gap(px(4.))
                .when(s.waiting > 0, |d| {
                    d.child(badge(
                        (id.clone(), "waiting"),
                        SessionStatus::Waiting,
                        s.waiting,
                        format!("{} waiting for you", s.waiting),
                        t,
                    ))
                })
                .when(s.working > 0, |d| {
                    d.child(badge(
                        (id.clone(), "working"),
                        SessionStatus::Working,
                        s.working,
                        format!("{} working", s.working),
                        t,
                    ))
                })
        });

        div()
            .w(px(144.))
            .flex()
            .flex_col()
            .items_center()
            .pt(px(2.))
            .px(px(4.))
            .pb(px(6.))
            .rounded(RADIUS_MD)
            .when(self.pop < 1.0, |d| {
                d.scale(0.6 + 0.4 * self.pop).group_opacity(self.pop)
            })
            .child(
                div()
                    .relative()
                    .when(self.pressed, |d| d.scale(0.97))
                    .w(px(124.))
                    .h(px(116.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(RADIUS_MD)
                    .bg(box_bg)
                    .child(div().translate(px(0.), px(lift)).opacity(opacity).child(folder))
                    .when(self.focused, |d| {
                        d.child(
                            div()
                                .id((id.clone(), "focus-mark"))
                                .absolute()
                                .top(px(18.))
                                .left(px(2.))
                                .size(px(20.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(10.))
                                .bg(t.bg_elevated)
                                .shadow(t.md())
                                .tooltip(native_tooltip("In focus".into()))
                                .child(icon(Icon::Target, 12., 2.4, t.accent)),
                        )
                    })
                    .children(badges)
                    .when_some(self.server, |d, server| {
                        d.child(
                            div()
                                .id((id.clone(), "server-badge"))
                                .absolute()
                                .bottom(px(16.))
                                .left(px(2.))
                                .size(px(20.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(10.))
                                .bg(t.bg_elevated)
                                .shadow(t.md())
                                .when(server.away, |d| d.opacity(0.45))
                                .tooltip(native_tooltip(server.tooltip().into()))
                                .child(icon(Icon::Server, 11., 2.4, t.text_secondary)),
                        )
                    }),
            )
            .map(|d| match self.rename {
                Some(field) => d.child(field),
                None => d.child(
                    div().mt(px(4.)).max_w(px(136.)).flex().justify_center().child(
                        div()
                            .px(px(6.))
                            .py(px(1.))
                            .rounded(px(5.))
                            .when(self.selected, |d| d.bg(t.accent))
                            .text_color(label_color)
                            .text_size(TEXT_MD)
                            .font_weight(FontWeight::MEDIUM)
                            .line_height(px(16.9))
                            .text_center()
                            .line_clamp(2)
                            .child(SharedString::from(p.name.clone())),
                    ),
                ),
            })
            .child(
                div()
                    .mt(px(2.))
                    .text_size(TEXT_XS)
                    .line_height(px(15.4))
                    .text_color(t.text_tertiary)
                    .font_features(fonts::tabular())
                    .whitespace_nowrap()
                    .child(subtitle(self.stats)),
            )
    }
}

/// A live badge pill (UI-234).
fn badge(id: impl Into<ElementId>, status: SessionStatus, count: u32, tooltip: String, t: Tokens) -> impl IntoElement {
    let id = id.into();
    let dot = status_dot((id.clone(), "dot"), status).badge();
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(5.))
        .h(px(20.))
        .pl(px(6.))
        .pr(px(7.))
        .rounded(px(10.))
        .bg(t.bg_elevated)
        .shadow(t.md())
        .text_size(TEXT_XS)
        .font_weight(FontWeight::SEMIBOLD)
        .font_features(fonts::tabular())
        .text_color(t.text_primary)
        .tooltip(native_tooltip(tooltip.into()))
        .child(dot)
        .child(count.to_string())
}

/// The rename field's frame (UI-237): 136px, 1px accent line in a 3px focus
/// band, centred text.
pub fn rename_frame(field: impl IntoElement, t: Tokens) -> Div {
    div()
        .mt(px(4.))
        .w(px(136.))
        .px(px(4.))
        .py(px(1.))
        .rounded(px(4.))
        .bg(t.bg_elevated)
        .shadow(vec![ring(4., t.focus_ring), ring(1., t.accent)])
        .text_size(TEXT_MD)
        .font_weight(FontWeight::MEDIUM)
        .line_height(px(16.9))
        .child(field)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtitles_the_counts() {
        let stats = |total, active| ProjectStats {
            total,
            active,
            ..ProjectStats::default()
        };
        assert_eq!(subtitle(None), "\u{a0}");
        assert_eq!(subtitle(Some(&stats(0, 0))), "No sessions");
        assert_eq!(subtitle(Some(&stats(1, 0))), "1 session");
        assert_eq!(subtitle(Some(&stats(5, 2))), "5 sessions · 2 active");
    }
}
