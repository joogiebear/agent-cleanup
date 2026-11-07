//! Look and feel: a dark, minimal palette (near-black rail, charcoal
//! surfaces, one indigo accent) and Inter for text.

use agent_cleanup::model::Verdict;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Hsla, Rgba, px};

const fn hex(h: u32) -> Rgba {
    Rgba {
        r: ((h >> 16) & 0xff) as f32 / 255.0,
        g: ((h >> 8) & 0xff) as f32 / 255.0,
        b: (h & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}

pub const RAIL: Rgba = hex(0x16171a);
pub const SIDEBAR: Rgba = hex(0x1d1e22);
pub const BG: Rgba = hex(0x232428);
pub const CARD: Rgba = hex(0x2a2b30);
pub const CARD_HOVER: Rgba = hex(0x313238);
pub const ELEVATED: Rgba = hex(0x34353b);
pub const BORDER: Rgba = hex(0x36373d);
pub const TEXT: Rgba = hex(0xeceef2);
pub const MUTED: Rgba = hex(0xa4a8b3);
pub const FAINT: Rgba = hex(0x74788a);
pub const ACCENT: Rgba = hex(0x6c76fa);
pub const ACCENT_HOVER: Rgba = hex(0x5b64e6);
pub const GREEN: Rgba = hex(0x3ec486);
pub const ORANGE: Rgba = hex(0xf2a54a);
pub const BLUE: Rgba = hex(0x5aa9fa);
pub const RED: Rgba = hex(0xf06464);

/// `color` with its alpha replaced, for tinted backgrounds.
pub fn tint(color: Rgba, alpha: f32) -> Rgba {
    Rgba { a: alpha, ..color }
}

pub fn verdict_color(v: Verdict) -> Rgba {
    match v {
        Verdict::SafeToRemove => GREEN,
        Verdict::UnsavedWork => ORANGE,
        Verdict::Unreadable => RED,
        Verdict::StrayActive => BLUE,
        Verdict::InProject => MUTED,
        Verdict::Checking => FAINT,
    }
}

fn h(c: Rgba) -> Hsla {
    c.into()
}

/// Dark mode plus our palette on gpui-component's widgets.
pub fn install(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |t| {
        t.font_family = "Inter".into();
        t.font_size = px(14.);
        t.radius = px(8.);
        t.radius_lg = px(12.);
        t.shadow = true;
        t.focus_ring = false;

        t.background = h(BG);
        t.foreground = h(TEXT);
        t.border = h(BORDER);
        t.input = h(BORDER);
        t.ring = h(ACCENT);
        t.caret = h(TEXT);
        t.selection = h(tint(ACCENT, 0.45));
        t.muted = h(CARD);
        t.muted_foreground = h(MUTED);
        t.popover = h(ELEVATED);
        t.popover_foreground = h(TEXT);
        t.accent = h(CARD_HOVER);
        t.accent_foreground = h(TEXT);
        t.overlay = h(tint(hex(0x000000), 0.6));

        t.primary = h(ACCENT);
        t.primary_hover = h(ACCENT_HOVER);
        t.primary_active = h(ACCENT_HOVER);
        t.primary_foreground = h(hex(0xffffff));
        t.button_primary = h(ACCENT);
        t.button_primary_hover = h(ACCENT_HOVER);
        t.button_primary_active = h(ACCENT_HOVER);
        t.button_primary_foreground = h(hex(0xffffff));

        t.secondary = h(CARD_HOVER);
        t.secondary_hover = h(ELEVATED);
        t.secondary_active = h(ELEVATED);
        t.secondary_foreground = h(TEXT);
        t.button = h(CARD_HOVER);
        t.button_hover = h(ELEVATED);
        t.button_active = h(ELEVATED);
        t.button_foreground = h(TEXT);
        t.button_secondary = h(CARD_HOVER);
        t.button_secondary_hover = h(ELEVATED);
        t.button_secondary_active = h(ELEVATED);
        t.button_secondary_foreground = h(TEXT);

        t.danger = h(RED);
        t.danger_hover = h(hex(0xd85353));
        t.danger_active = h(hex(0xd85353));
        t.danger_foreground = h(hex(0xffffff));
        t.button_danger = h(RED);
        t.button_danger_hover = h(hex(0xd85353));
        t.button_danger_active = h(hex(0xd85353));
        t.button_danger_foreground = h(hex(0xffffff));
        t.success = h(GREEN);
        t.warning = h(ORANGE);
        t.info = h(BLUE);

        t.switch = h(ELEVATED);
        t.switch_thumb = h(hex(0xffffff));
        t.scrollbar = h(tint(BG, 0.0));
        t.scrollbar_thumb = h(tint(hex(0xffffff), 0.14));
        t.scrollbar_thumb_hover = h(tint(hex(0xffffff), 0.24));
        t.list_hover = h(CARD_HOVER);
        t.list_active = h(tint(ACCENT, 0.18));
        t.list_active_border = h(ACCENT);
        t.window_border = h(BORDER);
        t.title_bar = h(RAIL);
        t.title_bar_border = h(RAIL);
    });
}
