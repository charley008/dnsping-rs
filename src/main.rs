#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod layout;
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
    let ui_runtime = runtime.clone();
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
            let work_area = layout::primary_work_area().unwrap_or_else(|| {
                cx.primary_display()
                    .map(|d| d.bounds())
                    .unwrap_or(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(1280.), px(800.)),
                    ))
            });
            let (bounds, minimum) = layout::window_geometry(work_area);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(minimum),
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
                        cx.new(|cx| ui::Dashboard::new(ui_runtime, settings, warning, window, cx));
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .expect("创建 DNSPing 窗口失败");
            cx.activate(true);
        });
    // Windows ICMP calls own their reply buffers until the OS completes them.
    // Do not block process exit waiting for those bounded background requests.
    if let Ok(runtime) = Arc::try_unwrap(runtime) {
        runtime.shutdown_background();
    }
}
