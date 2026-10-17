#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ui;
mod ui_theme;

use gpui::*;
use gpui_component::Root;
use std::sync::Arc;

fn main() {
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(4)
            .build()
            .expect("创建网络运行时失败"),
    );
    Application::new()
        .with_assets(gpui_component_assets::Assets)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);
            let (settings, warning) = dnsping::config::load();
            ui_theme::apply(settings.dark_theme, cx);
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let display_size = cx
                .primary_display()
                .map(|d| d.bounds().size)
                .unwrap_or(size(px(1920.), px(1080.)));
            let initial_size = size(
                px(1280.).min(display_size.width - px(48.)).max(px(1080.)),
                px(880.).min(display_size.height - px(80.)).max(px(720.)),
            );
            let bounds = Bounds::centered(None, initial_size, cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(1080.), px(720.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("DNSPing · DNS 优选".into()),
                        appears_transparent: true,
                        ..Default::default()
                    }),
                    app_id: Some("DNSPing".into()),
                    ..Default::default()
                },
                move |window, cx| {
                    let view =
                        cx.new(|cx| ui::Dashboard::new(runtime, settings, warning, window, cx));
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .expect("创建 DNSPing 窗口失败");
            cx.activate(true);
        });
}
