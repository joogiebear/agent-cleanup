//! Small styled building blocks shared by the pages.

use super::theme::*;
use agent_cleanup::model::Verdict;
use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, Sizable, Size, tooltip::Tooltip};
use gpui_kit::prelude::*;
use gpui_kit::*;

pub use gpui_kit::component::{h_flex, v_flex};

pub fn icon(name: IconName, size: f32, color: Rgba) -> Icon {
    Icon::new(name)
        .with_size(Size::Size(px(size)))
        .text_color(color)
}

pub fn verdict_icon(v: Verdict) -> IconName {
    match v {
        Verdict::SafeToRemove => IconName::CircleCheck,
        Verdict::UnsavedWork => IconName::TriangleAlert,
        Verdict::Unreadable => IconName::CircleQuestionMark,
        Verdict::StrayActive => IconName::Clock,
        Verdict::InProject => IconName::FolderHeart,
        Verdict::Checking => IconName::Hourglass,
    }
}

/// A rounded square icon in the left rail. Turns into the accent color when
/// it's the current page, Discord-style.
pub fn rail_button(
    id: &'static str,
    name: IconName,
    tip: &'static str,
    active: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .size(px(44.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(if active { 14. } else { 22. }))
        .cursor_pointer()
        .bg(if active { ACCENT } else { CARD })
        .hover(|s| {
            s.bg(if active { ACCENT } else { CARD_HOVER })
                .rounded(px(14.))
        })
        .child(icon(name, 20., if active { TEXT } else { MUTED }))
        .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
}

/// Colored dot plus label, for statuses.
pub fn status_pill(v: Verdict) -> Div {
    let c = verdict_color(v);
    h_flex()
        .flex_none()
        .gap_1p5()
        .px_2()
        .py(px(3.))
        .rounded_full()
        .bg(tint(c, 0.14))
        .text_color(c)
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .child(icon(verdict_icon(v), 13., c))
        .child(v.label())
}

/// Muted chip with an icon, for facts like size or age.
pub fn meta(name: IconName, text: impl Into<SharedString>) -> Div {
    h_flex()
        .flex_none()
        .gap_1()
        .text_color(FAINT)
        .text_size(px(12.))
        .child(icon(name, 13., FAINT))
        .child(text.into())
}

pub fn card() -> Div {
    div()
        .rounded(px(12.))
        .bg(CARD)
        .border_1()
        .border_color(BORDER)
}

pub fn page_header(title: &str, subtitle: &str) -> Div {
    v_flex()
        .gap_1()
        .child(
            div()
                .text_size(px(22.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.to_string()),
        )
        .child(
            div()
                .text_color(MUTED)
                .text_size(px(14.))
                .child(subtitle.to_string()),
        )
}

pub fn section_label(text: &str) -> Div {
    div()
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(FAINT)
        .child(text.to_uppercase())
}

/// An icon-only button that shows on cards, with a tooltip.
pub fn icon_action(
    id: impl Into<ElementId>,
    name: IconName,
    tip: impl Into<SharedString>,
    danger: bool,
) -> Stateful<Div> {
    let tip: SharedString = tip.into();
    let fg = if danger { RED } else { MUTED };
    div()
        .id(id)
        .size(px(30.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(8.))
        .cursor_pointer()
        .hover(move |s| s.bg(if danger { tint(RED, 0.15) } else { ELEVATED }))
        .child(icon(name, 16., fg))
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
}

/// A plain text button with an optional icon, styled to the palette.
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    kind: ButtonKind,
) -> Stateful<Div> {
    button_base(id, kind).child(label.into())
}

fn button_base(id: impl Into<ElementId>, kind: ButtonKind) -> Stateful<Div> {
    let (bg, hover, fg) = match kind {
        ButtonKind::Primary => (ACCENT, ACCENT_HOVER, hex_white()),
        ButtonKind::Secondary => (CARD_HOVER, ELEVATED, TEXT),
        ButtonKind::Danger => (
            RED,
            Rgba {
                r: 0.85,
                g: 0.33,
                b: 0.33,
                a: 1.0,
            },
            hex_white(),
        ),
        ButtonKind::Ghost => (tint(BG, 0.0), CARD_HOVER, MUTED),
    };
    h_flex()
        .id(id)
        .flex_none()
        .gap_2()
        .h(px(34.))
        .px_3p5()
        .justify_center()
        .rounded(px(8.))
        .cursor_pointer()
        .bg(bg)
        .hover(move |s| s.bg(hover))
        .text_color(fg)
        .text_size(px(14.))
        .font_weight(FontWeight::MEDIUM)
}

#[derive(Clone, Copy)]
pub enum ButtonKind {
    Primary,
    Secondary,
    Danger,
    Ghost,
}

pub fn hex_white() -> Rgba {
    Rgba {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    }
}

/// Same as [`button`] with a leading icon.
pub fn icon_button(
    id: impl Into<ElementId>,
    name: IconName,
    label: impl Into<SharedString>,
    kind: ButtonKind,
) -> Stateful<Div> {
    let fg = match kind {
        ButtonKind::Primary | ButtonKind::Danger => hex_white(),
        ButtonKind::Secondary => TEXT,
        ButtonKind::Ghost => MUTED,
    };
    let label = label.into();
    button_base(id, kind)
        .child(icon(name, 16., fg))
        .child(label)
}

/// A disabled-looking button that does nothing.
pub fn disabled(el: Stateful<Div>) -> Stateful<Div> {
    el.opacity(0.45).cursor_not_allowed()
}

pub trait Tip {
    fn tip(self, text: impl Into<SharedString>) -> Self;
}

impl Tip for Stateful<Div> {
    fn tip(self, text: impl Into<SharedString>) -> Self {
        let text: SharedString = text.into();
        self.tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx))
    }
}

/// A small coloured tag with the app's call on an item, like "Safe to clean".
pub fn verdict_tag(
    id: impl Into<ElementId>,
    color: Rgba,
    ic: IconName,
    text: impl Into<SharedString>,
    tip: impl Into<SharedString>,
) -> Stateful<Div> {
    h_flex()
        .id(id)
        .flex_none()
        .gap_1()
        .px_2()
        .py(px(2.))
        .rounded_full()
        .bg(tint(color, 0.14))
        .text_color(color)
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .child(icon(ic, 12., color))
        .child(text.into())
        .tip(tip)
}

/// The box at the top of a page that says what the app recommends: a
/// headline, why, optional tags, and an optional action button.
pub fn advice_banner(
    color: Rgba,
    ic: IconName,
    headline: impl Into<SharedString>,
    text: impl Into<SharedString>,
    tags: Option<Div>,
    action: Option<Stateful<Div>>,
) -> Div {
    h_flex()
        .gap_4()
        .px_5()
        .py_4()
        .rounded(px(12.))
        .bg(tint(color, 0.10))
        .border_1()
        .border_color(tint(color, 0.35))
        .child(icon(ic, 24., color))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    div()
                        .text_size(px(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(headline.into()),
                )
                .child(
                    div()
                        .text_color(MUTED)
                        .text_size(px(13.))
                        .child(text.into()),
                )
                .children(tags),
        )
        .children(action.map(|a| a.flex_none().h(px(40.)).px_5()))
}

/// "Copy prompt for your agent": the caller adds the click handler.
pub fn prompt_button(id: impl Into<ElementId>) -> Stateful<Div> {
    icon_button(id, IconName::Bot, "Copy prompt for your agent", ButtonKind::Ghost)
        .flex_none()
        .h(px(28.))
        .tip("Copies a request you can paste into Claude Code or Codex. It lists these items, says what to do with them, and tells the agent not to delete anything. It only includes the paths you see here.")
}
