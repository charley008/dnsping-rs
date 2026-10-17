use gpui::{App, px, rgb};
use gpui_component::{Theme, ThemeMode};

/// Shared semantic colors for every custom surface and status in the dashboard.
#[derive(Clone, Copy)]
pub struct Palette {
    pub background: u32,
    pub surface: u32,
    pub border: u32,
    pub text: u32,
    pub muted: u32,
    pub accent: u32,
    pub sidebar: u32,
    pub selected: u32,
    pub unselected: u32,
    pub accent_surface: u32,
    pub accent_border: u32,
    pub divider: u32,
    pub table_header: u32,
    pub winner: u32,
    pub info_surface: u32,
    pub info_text: u32,
    pub empty_surface: u32,
    pub empty_icon: u32,
    pub subtle: u32,
    pub error: u32,
    pub error_detail: u32,
    pub bar: u32,
    pub pending: u32,
}

impl Palette {
    pub fn current(cx: &App) -> Self {
        Self::new(Theme::global(cx).mode == ThemeMode::Dark)
    }

    pub fn new(dark: bool) -> Self {
        if dark {
            Self {
                background: 0x0b111b,
                surface: 0x111b29,
                border: 0x243246,
                text: 0xe8eef7,
                muted: 0x8b9bb0,
                accent: 0x5ce0c4,
                sidebar: 0x0e1724,
                selected: 0x172334,
                unselected: 0x101a28,
                accent_surface: 0x112a2b,
                accent_border: 0x2a514c,
                divider: 0x1c2a3c,
                table_header: 0x152031,
                winner: 0x142e2e,
                info_surface: 0x142532,
                info_text: 0x8dc7e9,
                empty_surface: 0x19293d,
                empty_icon: 0x82a9d7,
                subtle: 0x64778f,
                error: 0xf59898,
                error_detail: 0xca8f92,
                bar: 0x688fca,
                pending: 0x8bb5ff,
            }
        } else {
            Self {
                background: 0xf3f6fa,
                surface: 0xffffff,
                border: 0xd9e2ed,
                text: 0x1b2c40,
                muted: 0x60738b,
                accent: 0x007e69,
                sidebar: 0xecf1f7,
                selected: 0xe1ecf0,
                unselected: 0xf5f7fa,
                accent_surface: 0xe8f6f1,
                accent_border: 0xa9d9c9,
                divider: 0xe5ebf2,
                table_header: 0xf0f4f8,
                winner: 0xeaf7f2,
                info_surface: 0xe2edf8,
                info_text: 0x356c99,
                empty_surface: 0xe8eff8,
                empty_icon: 0x527ba6,
                subtle: 0x6c7e93,
                error: 0xb83b48,
                error_detail: 0x9d4653,
                bar: 0x497bb8,
                pending: 0x396bb0,
            }
        }
    }
}

pub fn apply(dark: bool, cx: &mut App) {
    Theme::change(
        if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        },
        None,
        cx,
    );
    let colors = Palette::new(dark);
    let theme = Theme::global_mut(cx);
    theme.font_family = "Microsoft YaHei UI".into();
    theme.font_size = px(14.);
    theme.mono_font_family = "Consolas".into();
    theme.radius = px(8.);
    theme.background = rgb(colors.background).into();
    theme.foreground = rgb(colors.text).into();
    theme.primary = rgb(colors.accent).into();
    theme.primary_hover = rgb(if dark { 0x7aead3 } else { 0x00927b }).into();
    theme.primary_active = rgb(if dark { 0x35c7aa } else { 0x006754 }).into();
    theme.primary_foreground = rgb(if dark { 0x08251f } else { 0xffffff }).into();
    theme.secondary = rgb(if dark { 0x1a283a } else { 0xe8eef5 }).into();
    theme.secondary_hover = rgb(if dark { 0x26384e } else { 0xdbe5f0 }).into();
    theme.secondary_foreground = rgb(colors.text).into();
    theme.border = rgb(colors.border).into();
    theme.input = rgb(colors.border).into();
    theme.muted = rgb(colors.selected).into();
    theme.muted_foreground = rgb(colors.muted).into();
    theme.caret = rgb(colors.accent).into();
    theme.ring = rgb(colors.accent).into();
}
