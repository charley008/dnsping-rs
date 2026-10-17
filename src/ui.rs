use crate::ui_theme::{self, Palette};
use dnsping::{
    config,
    engine::{self, Event},
    model::*,
};
use gpui::{prelude::*, *};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    *,
};
use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

#[derive(IntoElement)]
struct Caption(SharedString);

impl RenderOnce for Caption {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .text_size(px(12.))
            .text_color(rgb(Palette::current(cx).muted))
            .child(self.0)
    }
}

fn label(text: impl Into<SharedString>) -> Caption {
    Caption(text.into())
}
fn panel(colors: Palette) -> Div {
    div()
        .bg(rgb(colors.surface))
        .border_1()
        .border_color(rgb(colors.border))
        .rounded(px(14.))
}
fn icon(name: IconName) -> Icon {
    Icon::new(name).size(px(17.))
}
fn number(value: Option<f64>) -> String {
    value
        .map(|v| format!("{v:.1}"))
        .unwrap_or_else(|| "—".into())
}

pub struct Dashboard {
    settings: Settings,
    snapshot: Option<Settings>,
    domain: Entity<InputState>,
    port: Entity<InputState>,
    samples: Entity<InputState>,
    timeout: Entity<InputState>,
    custom_name: Entity<InputState>,
    custom_ip: Entity<InputState>,
    adding: bool,
    editing_server: Option<usize>,
    rows: Vec<ResultRow>,
    runtime: Arc<tokio::runtime::Runtime>,
    receiver: Option<Receiver<Event>>,
    cancel: Option<CancellationToken>,
    running: bool,
    completed: bool,
    started: Option<Instant>,
    elapsed: f64,
    run_id: u64,
    server_scroll: ScrollHandle,
    result_scroll: ScrollHandle,
    message: String,
    error: bool,
    _subscriptions: Vec<Subscription>,
}

impl Dashboard {
    pub fn new(
        runtime: Arc<tokio::runtime::Runtime>,
        settings: Settings,
        warning: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let domain = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("输入域名，例如 www.example.com")
                .default_value(settings.domain.clone())
        });
        let port =
            cx.new(|cx| InputState::new(window, cx).default_value(settings.port.to_string()));
        let samples =
            cx.new(|cx| InputState::new(window, cx).default_value(settings.samples.to_string()));
        let timeout =
            cx.new(|cx| InputState::new(window, cx).default_value(settings.timeout_ms.to_string()));
        let custom_name = cx.new(|cx| InputState::new(window, cx).placeholder("服务商名称"));
        let custom_ip = cx.new(|cx| InputState::new(window, cx).placeholder("IPv4 / IPv6 地址"));
        let mut subscriptions = vec![cx.subscribe_in(&domain, window, |this, _, event, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) && !this.running {
                this.start(cx);
            }
        })];
        for input in [&domain, &port, &samples, &timeout] {
            subscriptions.push(cx.subscribe(input, |this, _, event, cx| {
                if matches!(event, InputEvent::Blur) && !this.running {
                    this.persist_inputs(cx);
                }
            }));
        }
        subscriptions.push(cx.on_release(|this, cx| this.persist_inputs(cx)));
        subscriptions.push(
            cx.subscribe_in(&custom_ip, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) && this.adding && !this.running {
                    this.save_server(window, cx);
                }
            }),
        );
        Self {
            settings,
            snapshot: None,
            domain,
            port,
            samples,
            timeout,
            custom_name,
            custom_ip,
            adding: false,
            editing_server: None,
            rows: vec![],
            runtime,
            receiver: None,
            cancel: None,
            running: false,
            completed: false,
            started: None,
            elapsed: 0.,
            run_id: 0,
            server_scroll: ScrollHandle::new(),
            result_scroll: ScrollHandle::new(),
            message: warning
                .clone()
                .unwrap_or_else(|| "准备就绪 · 输入域名，比较不同 DNS 的解析与连接质量".into()),
            error: warning.is_some(),
            _subscriptions: subscriptions,
        }
    }

    fn read_inputs(&self, cx: &App) -> anyhow::Result<Settings> {
        use anyhow::Context as _;
        let mut s = self.settings.clone();
        s.domain = self.domain.read(cx).value().trim().to_owned();
        if s.probe == ProbeKind::Tcp {
            s.port = self
                .port
                .read(cx)
                .value()
                .parse()
                .context("端口必须是 1–65535 的整数")?;
        }
        s.samples = self
            .samples
            .read(cx)
            .value()
            .parse()
            .context("测试次数必须是 1–30 的整数")?;
        s.timeout_ms = self
            .timeout
            .read(cx)
            .value()
            .parse()
            .context("超时必须是 200–10000 的整数，单位 ms")?;
        Ok(s)
    }

    fn persist_inputs(&mut self, cx: &App) {
        if let Ok(s) = self.read_inputs(cx)
            && (1..=30).contains(&s.samples)
            && (200..=10000).contains(&s.timeout_ms)
            && s.port > 0
        {
            self.settings = s;
            self.persist();
        }
    }

    fn persist(&mut self) {
        if let Err(e) = config::save(&self.settings) {
            self.message = format!("设置保存失败：{e}");
            self.error = true;
        }
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        let result = self.read_inputs(cx).and_then(|mut s| {
            s.domain = s.validate()?;
            Ok(s)
        });
        let settings = match result {
            Ok(s) => s,
            Err(e) => {
                self.message = e.to_string();
                self.error = true;
                cx.notify();
                return;
            }
        };
        self.settings = settings.clone();
        self.persist();
        self.snapshot = Some(settings.clone());
        self.rows = settings
            .servers
            .iter()
            .filter(|s| s.enabled)
            .cloned()
            .map(ResultRow::new)
            .collect();
        self.completed = false;
        self.result_scroll.set_offset(point(px(0.), px(0.)));
        self.running = true;
        self.error = false;
        self.elapsed = 0.;
        self.started = Some(Instant::now());
        self.run_id += 1;
        let run_id = self.run_id;
        self.message = "正在向选定的 DNS 服务器查询…".into();
        let cancel = CancellationToken::new();
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        self.cancel = Some(cancel.clone());
        self.runtime.spawn(engine::run(settings, tx, cancel));
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(70))
                    .await;
                let keep = this
                    .update(cx, |this, cx| {
                        if this.run_id != run_id {
                            return false;
                        }
                        this.poll(cx);
                        this.running
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        if let Some(cancel) = self.cancel.take() {
            cancel.cancel();
        }
        // Drain no further messages after cancellation; the next run owns a fresh channel.
        self.receiver = None;
        engine::apply_event(&mut self.rows, &Event::Cancelled, self.settings.samples);
        self.running = false;
        self.completed = false;
        self.message = "测试已停止 · 已保留当前结果，可重新开始".into();
        cx.notify();
    }

    fn poll(&mut self, cx: &mut Context<Self>) {
        let events: Vec<_> = self
            .receiver
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        if let Some(start) = self.started {
            self.elapsed = start.elapsed().as_secs_f64();
        }
        for event in events {
            engine::apply_event(&mut self.rows, &event, self.settings.samples);
            match event {
                Event::Finished => {
                    self.running = false;
                    self.completed = true;
                    self.receiver = None;
                    self.cancel = None;
                    self.message = if best_row(&self.rows).is_some() {
                        format!(
                            "测试完成 · 用时 {:.1} 秒 · 按成功率、平均延迟推荐",
                            self.elapsed
                        )
                    } else {
                        "测试完成，但没有可用 IP · 请检查端口、网络或切换测速方式".into()
                    };
                }
                Event::Sample { .. } => {
                    self.message = "正在测速 · 相同 IP 共用一组样本，结果实时更新".into()
                }
                _ => {}
            }
        }
        cx.notify();
    }

    fn open_server_editor(
        &mut self,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.running {
            return;
        }
        let (name, address) = index
            .and_then(|i| self.settings.servers.get(i))
            .map(|s| (s.name.clone(), s.address.to_string()))
            .unwrap_or_default();
        self.editing_server = index;
        self.adding = true;
        self.custom_name
            .update(cx, |state, cx| state.set_value(name, window, cx));
        self.custom_ip
            .update(cx, |state, cx| state.set_value(address, window, cx));
        cx.notify();
    }

    fn save_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.running || !self.adding {
            return;
        }
        let name = self.custom_name.read(cx).value();
        let address = self.custom_ip.read(cx).value();
        match self
            .settings
            .save_server(self.editing_server, &name, &address)
        {
            Ok(()) => {
                self.message = if self.editing_server.is_some() {
                    "DNS 服务商已更新，下次测试使用新设置".into()
                } else {
                    "已添加 DNS 服务商".into()
                };
                self.error = false;
                self.adding = false;
                self.editing_server = None;
                self.custom_name
                    .update(cx, |s, cx| s.set_value("", window, cx));
                self.custom_ip
                    .update(cx, |s, cx| s.set_value("", window, cx));
                self.persist();
            }
            Err(error) => {
                self.message = error.to_string();
                self.error = true;
            }
        }
        cx.notify();
    }

    fn export(&mut self, cx: &mut Context<Self>) {
        let Some(settings) = &self.snapshot else {
            return;
        };
        let csv = csv_report(settings, &self.rows);
        let path = cx.prompt_for_new_path(&config::directory(), Some("DNSPing-results.csv"));
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(path))) = path.await {
                let result = std::fs::write(&path, csv);
                let _ = this.update(cx, |this, cx| {
                    match result {
                        Ok(_) => {
                            this.message = format!("已导出：{}", path.display());
                            this.error = false;
                        }
                        Err(e) => {
                            this.message = format!("导出失败：{e}");
                            this.error = true;
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn copy_ip(&mut self, ip: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(ip.clone()));
        self.message = format!("已复制 IP：{ip}");
        self.error = false;
        cx.notify();
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Palette::current(cx);
        let mut servers = div()
            .id("servers")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2();
        for (index, server) in self.settings.servers.iter().enumerate() {
            let color = colors.providers[index.min(5)];
            servers = servers.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_3()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(rgb(if server.enabled {
                        colors.border
                    } else {
                        colors.selected
                    }))
                    .bg(rgb(if server.enabled {
                        colors.selected
                    } else {
                        colors.unselected
                    }))
                    .child(
                        Checkbox::new(("dns", index))
                            .checked(server.enabled)
                            .disabled(self.running)
                            .on_click(cx.listener(move |this, enabled, _, cx| {
                                this.settings.servers[index].enabled = *enabled;
                                this.persist();
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_ellipsis()
                                    .text_color(rgb(color))
                                    .child(server.name.clone()),
                            )
                            .child(
                                div()
                                    .font_family("Consolas")
                                    .text_size(px(12.))
                                    .text_ellipsis()
                                    .text_color(rgb(colors.muted))
                                    .child(server.address.to_string()),
                            ),
                    )
                    .child(
                        Button::new(("edit-dns", index))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Settings2)
                            .label("编辑")
                            .tooltip(format!("编辑 {} ({})", server.name, server.address))
                            .disabled(self.running)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_server_editor(Some(index), window, cx)
                            })),
                    )
                    .when(server.custom, |el| {
                        el.child(
                            Button::new(("delete", index))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Close)
                                .tooltip("删除自定义 DNS")
                                .disabled(self.running || self.adding)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.settings.servers.remove(index);
                                    this.persist();
                                    cx.notify();
                                })),
                        )
                    }),
            );
        }
        div()
            .w(px(252.))
            .flex_none()
            .h_full()
            .bg(rgb(colors.sidebar))
            .border_r_1()
            .border_color(rgb(colors.border))
            .flex()
            .flex_col()
            .p_5()
            .gap_5()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .py_2()
                    .child(
                        div()
                            .size(px(40.))
                            .rounded(px(12.))
                            .bg(rgb(colors.winner))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(rgb(colors.accent))
                            .child(icon(IconName::Globe)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(22.))
                                    .font_weight(FontWeight::BOLD)
                                    .child("DNSPing"),
                            )
                            .child(label("找到更快的连接")),
                    ),
            )
            .child(div().h(px(1.)).bg(rgb(colors.border)))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("DNS 服务商"),
                    )
                    .child(label(format!(
                        "已选 {}",
                        self.settings.servers.iter().filter(|s| s.enabled).count()
                    ))),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(servers.h_full().track_scroll(&self.server_scroll))
                    .vertical_scrollbar(&self.server_scroll),
            )
            .when(self.adding, |el| {
                el.child(
                    panel(colors)
                        .p_3()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(if self.editing_server.is_some() {
                                    "编辑 DNS 服务商"
                                } else {
                                    "添加 DNS 服务商"
                                }),
                        )
                        .child(Input::new(&self.custom_name).small().disabled(self.running))
                        .child(Input::new(&self.custom_ip).small().disabled(self.running))
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(
                                    Button::new("save-dns")
                                        .primary()
                                        .small()
                                        .label(if self.editing_server.is_some() {
                                            "保存修改"
                                        } else {
                                            "添加"
                                        })
                                        .disabled(self.running)
                                        .on_click(cx.listener(Self::on_save_server)),
                                )
                                .child(
                                    Button::new("cancel-dns")
                                        .ghost()
                                        .small()
                                        .label("取消")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.adding = false;
                                            this.editing_server = None;
                                            cx.notify();
                                        })),
                                ),
                        ),
                )
            })
            .child(
                Button::new("add-dns")
                    .icon(IconName::Plus)
                    .label("添加自定义 DNS")
                    .w_full()
                    .disabled(self.running || self.adding)
                    .on_click(
                        cx.listener(|this, _, window, cx| {
                            this.open_server_editor(None, window, cx)
                        }),
                    ),
            )
            .child(
                div()
                    .p_3()
                    .rounded(px(10.))
                    .bg(rgb(colors.info_surface))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .items_center()
                            .text_color(rgb(colors.info_text))
                            .child(icon(IconName::Info))
                            .child("独立查询 · 真实测速"),
                    )
                    .child(label("每个 DNS 取首个 IP；相同 IP 自动合并测速。")),
            )
            .child(label(format!("WINDOWS  /  v{}", env!("CARGO_PKG_VERSION"))))
            .child(
                Button::new("config-file")
                    .ghost()
                    .small()
                    .icon(IconName::Folder)
                    .label("配置文件")
                    .tooltip(config::path().display().to_string())
                    .on_click(|_, _, cx| cx.reveal_path(&config::path())),
            )
    }

    fn on_save_server(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.save_server(window, cx);
    }

    fn controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Palette::current(cx);
        let ipv4 = self.settings.query == QueryKind::Ipv4;
        let tcp = self.settings.probe == ProbeKind::Tcp;
        panel(colors)
            .p_4()
            .flex_none()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.domain)
                                .large()
                                .disabled(self.running)
                                .prefix(icon(IconName::Search)),
                        ),
                    )
                    .child(
                        Button::new("run")
                            .large()
                            .w(px(130.))
                            .when(!self.running, |b| b.primary())
                            .when(self.running, |b| b.danger())
                            .icon(if self.running {
                                IconName::Close
                            } else {
                                IconName::ArrowRight
                            })
                            .label(if self.running {
                                "停止测试"
                            } else {
                                "开始测试"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.running {
                                    this.stop(cx);
                                } else {
                                    this.start(cx);
                                }
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(label("解析类型"))
                            .child(
                                div()
                                    .flex()
                                    .gap_1()
                                    .child(
                                        Button::new("ipv4")
                                            .small()
                                            .label("IPv4")
                                            .selected(ipv4)
                                            .when(ipv4, |b| b.primary())
                                            .disabled(self.running)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.settings.query = QueryKind::Ipv4;
                                                this.persist();
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new("ipv6")
                                            .small()
                                            .label("IPv6")
                                            .selected(!ipv4)
                                            .when(!ipv4, |b| b.primary())
                                            .disabled(self.running)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.settings.query = QueryKind::Ipv6;
                                                this.persist();
                                                cx.notify();
                                            })),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(label("测速方式"))
                            .child(
                                div()
                                    .flex()
                                    .gap_1()
                                    .child(
                                        Button::new("tcp")
                                            .small()
                                            .label("TCPing")
                                            .selected(tcp)
                                            .when(tcp, |b| b.primary())
                                            .disabled(self.running)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.settings.probe = ProbeKind::Tcp;
                                                this.persist();
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new("icmp")
                                            .small()
                                            .label("Ping")
                                            .selected(!tcp)
                                            .when(!tcp, |b| b.primary())
                                            .disabled(self.running)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.settings.probe = ProbeKind::Icmp;
                                                this.persist();
                                                cx.notify();
                                            })),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .w(px(74.))
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(label("端口"))
                            .child(
                                Input::new(&self.port)
                                    .small()
                                    .disabled(self.running || !tcp),
                            ),
                    )
                    .child(
                        div()
                            .w(px(70.))
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(label("次数"))
                            .child(Input::new(&self.samples).small().disabled(self.running)),
                    )
                    .child(
                        div()
                            .w(px(98.))
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(label("超时 / ms"))
                            .child(Input::new(&self.timeout).small().disabled(self.running)),
                    ),
            )
    }

    fn recommendation(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Palette::current(cx);
        let best = self
            .completed
            .then(|| best_row(&self.rows))
            .flatten()
            .map(|i| &self.rows[i]);
        let title = if best.is_some() {
            "本轮推荐 DNS"
        } else if self.running {
            "测试进行中"
        } else {
            "本轮推荐"
        };
        let headline = best
            .map(|r| format!("{} · {}", r.server.name, r.server.address))
            .unwrap_or_else(|| {
                if self.running {
                    "正在比较 DNS 与连接质量…".into()
                } else if self.completed {
                    "本轮没有可用结果".into()
                } else {
                    "选择 DNS，开始对比测试".into()
                }
            });
        let detail = best
            .map(|r| format!("首个 IP：{} · 优先成功率，再比较平均延迟", r.ip.unwrap()))
            .unwrap_or_else(|| {
                if self.completed {
                    "请检查网络、目标端口，或切换测速方式。".into()
                } else {
                    "对比解析速度与返回 IP 的连接质量，辅助选择 SmartDNS 上游。".into()
                }
            });
        let mut card = div()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(colors.accent_border))
            .bg(rgb(colors.accent_surface))
            .px_4()
            .py(px(10.))
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .child(
                div()
                    .size(px(30.))
                    .flex_none()
                    .rounded(px(9.))
                    .bg(rgb(colors.winner))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(colors.accent))
                    .child(icon(IconName::Star)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(px(11.))
                                    .text_color(rgb(colors.accent))
                                    .child(title),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_ellipsis()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(headline),
                            ),
                    )
                    .child(
                        div()
                            .text_ellipsis()
                            .text_size(px(11.))
                            .text_color(rgb(colors.muted))
                            .child(detail),
                    ),
            );
        if let Some(row) = best {
            let ip = row.ip.unwrap().to_string();
            let dns = row.server.address.to_string();
            card = card
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_col()
                        .items_end()
                        .child(
                            div()
                                .text_size(px(20.))
                                .font_family("Consolas")
                                .text_color(rgb(colors.accent))
                                .child(format!("{} ms", number(row.stats.average()))),
                        )
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(colors.muted))
                                .child("平均连接延迟"),
                        ),
                )
                .child(
                    Button::new("copy-best-dns")
                        .small()
                        .ghost()
                        .icon(IconName::Copy)
                        .label("复制 DNS")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(dns.clone()));
                            this.message = format!("已复制 DNS：{dns}");
                            this.error = false;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("copy-best")
                        .small()
                        .ghost()
                        .icon(IconName::Copy)
                        .label("IP")
                        .tooltip("复制推荐 DNS 返回的 IP")
                        .on_click(cx.listener(move |this, _, _, cx| this.copy_ip(ip.clone(), cx))),
                );
        }
        card
    }

    fn results(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Palette::current(cx);
        let best = if self.completed {
            best_row(&self.rows)
        } else {
            None
        };
        let snapshot = self.snapshot.as_ref().unwrap_or(&self.settings);
        let mut body = div()
            .id("result-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col();
        let mut indices: Vec<_> = (0..self.rows.len()).collect();
        if self.completed {
            indices.sort_by(|a, b| compare_rows(&self.rows[*a], &self.rows[*b]));
        }
        for index in indices {
            let row = &self.rows[index];
            let winner = best == Some(index);
            let (status, status_color) = match &row.status {
                RowStatus::Resolving => ("解析中".to_owned(), colors.pending),
                RowStatus::Waiting => ("等待测速".into(), colors.muted),
                RowStatus::Probing => (
                    format!("测速 {}/{}", row.stats.samples.len(), snapshot.samples),
                    colors.pending,
                ),
                RowStatus::Complete => {
                    (if winner { "最佳" } else { "已完成" }.into(), colors.accent)
                }
                RowStatus::Failed(_) => ("失败".into(), colors.error),
                RowStatus::Cancelled => ("已停止".into(), colors.muted),
            };
            let ip = row.ip.map(|ip| ip.to_string());
            let mut content = div()
                .flex_none()
                .flex()
                .flex_col()
                .border_b_1()
                .border_color(rgb(colors.divider))
                .bg(rgb(if winner {
                    colors.winner
                } else {
                    colors.surface
                }))
                .px_4()
                .py(px(5.))
                .gap(px(2.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .min_h(px(30.))
                        .child(
                            div()
                                .w(px(116.))
                                .flex_none()
                                .flex()
                                .flex_col()
                                .line_height(px(15.))
                                .gap(px(1.))
                                .child(row.server.name.clone())
                                .child(label(row.server.address.to_string())),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .font_family("Consolas")
                                .text_size(px(14.))
                                .text_ellipsis()
                                .child(ip.clone().unwrap_or_else(|| "—".into())),
                        )
                        .child(
                            div()
                                .w(px(72.))
                                .flex_none()
                                .font_family("Consolas")
                                .child(number(row.dns_ms)),
                        )
                        .child(
                            div()
                                .w(px(84.))
                                .flex_none()
                                .font_family("Consolas")
                                .text_color(rgb(if winner { colors.accent } else { colors.text }))
                                .child(number(row.stats.average())),
                        )
                        .child(div().w(px(64.)).flex_none().font_family("Consolas").child(
                            if row.stats.samples.is_empty() {
                                "—".into()
                            } else {
                                format!("{}/{}", row.stats.successes(), row.stats.samples.len())
                            },
                        ))
                        .child(
                            div()
                                .w(px(78.))
                                .flex_none()
                                .text_size(px(12.))
                                .text_color(rgb(status_color))
                                .child(status),
                        )
                        .child(
                            Button::new(("copy-row", index))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Copy)
                                .tooltip("复制 IP")
                                .disabled(ip.is_none())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(ip) = &ip {
                                        this.copy_ip(ip.clone(), cx);
                                    }
                                })),
                        ),
                );
            if let RowStatus::Failed(error) = &row.status {
                content = content.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(colors.error_detail))
                        .child(error.clone()),
                );
            } else if !row.stats.samples.is_empty() {
                let max = row
                    .stats
                    .samples
                    .iter()
                    .filter_map(|v| v.as_ref().ok())
                    .copied()
                    .fold(1., f64::max);
                content = content.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .pl(px(124.))
                        .child(div().flex().gap_1().items_end().h(px(10.)).children(
                            row.stats.samples.iter().map(|sample| {
                                div()
                                    .w(px(7.))
                                    .rounded(px(2.))
                                    .h(px(sample
                                        .as_ref()
                                        .map(|ms| (ms / max * 9.).max(2.) as f32)
                                        .unwrap_or(9.)))
                                    .bg(rgb(if sample.is_ok() {
                                        if winner { colors.accent } else { colors.bar }
                                    } else {
                                        colors.error
                                    }))
                            }),
                        ))
                        .child(label(format!(
                            "最小 {} ms   ·   抖动 {} ms",
                            number(row.stats.minimum()),
                            number(row.stats.jitter())
                        ))),
                );
            }
            body = body.child(content);
        }
        if self.rows.is_empty() {
            body = body.child(
                div()
                    .flex_1()
                    .min_h(px(190.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .child(
                        div()
                            .size(px(52.))
                            .rounded(px(16.))
                            .bg(rgb(colors.empty_surface))
                            .text_color(rgb(colors.empty_icon))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon(IconName::ChartPie)),
                    )
                    .child(
                        div()
                            .text_color(rgb(colors.text))
                            .child("你的下一次测试，从这里开始"),
                    )
                    .child(label("选择左侧 DNS，输入域名，按 Enter 或点击开始测试。")),
            );
        }
        panel(colors)
            .flex_1()
            .min_h(px(180.))
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_4()
                    .py_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .items_center()
                            .child("解析与测速结果")
                            .child(label(format!(
                                "{} 个 DNS · {} · {}",
                                self.rows.len(),
                                if snapshot.query == QueryKind::Ipv4 {
                                    "IPv4"
                                } else {
                                    "IPv6"
                                },
                                if snapshot.probe == ProbeKind::Tcp {
                                    format!("TCP :{}", snapshot.port)
                                } else {
                                    "ICMP".into()
                                }
                            ))),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(
                                Button::new("copy-report")
                                    .ghost()
                                    .small()
                                    .icon(IconName::Copy)
                                    .label("复制结果")
                                    .disabled(self.rows.is_empty())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(s) = &this.snapshot {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                csv_report(s, &this.rows),
                                            ));
                                            this.message = "已复制完整结果（CSV）".into();
                                            this.error = false;
                                            cx.notify();
                                        }
                                    })),
                            )
                            .child(
                                Button::new("export")
                                    .ghost()
                                    .small()
                                    .icon(IconName::ArrowDown)
                                    .label("导出 CSV")
                                    .disabled(self.rows.is_empty())
                                    .on_click(cx.listener(|this, _, _, cx| this.export(cx))),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .px_4()
                    .py_2()
                    .bg(rgb(colors.table_header))
                    .border_y_1()
                    .border_color(rgb(colors.border))
                    .child(div().w(px(116.)).flex_none().child(label("DNS 服务商")))
                    .child(div().flex_1().min_w_0().child(label("首个返回 IP")))
                    .child(div().w(px(72.)).flex_none().child(label("解析 / ms")))
                    .child(div().w(px(84.)).flex_none().child(label("平均 / ms")))
                    .child(div().w(px(64.)).flex_none().child(label("成功 / 已测")))
                    .child(div().w(px(78.)).flex_none().child(label("状态")))
                    .child(div().w(px(24.)).flex_none()),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(body.h_full().track_scroll(&self.result_scroll))
                    .vertical_scrollbar(&self.result_scroll),
            )
    }
}

impl Drop for Dashboard {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.cancel();
        }
    }
}

impl Render for Dashboard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Palette::current(cx);
        let dns_done = self
            .rows
            .iter()
            .filter(|r| !matches!(r.status, RowStatus::Resolving))
            .count();
        let samples_done: usize = self.rows.iter().map(|r| r.stats.samples.len()).sum();
        let total = self.rows.len() * (self.settings.samples as usize + 1);
        let progress = if total == 0 {
            0.
        } else {
            (dns_done + samples_done) as f32 / total as f32
        };
        div().size_full().bg(rgb(colors.background)).text_color(rgb(colors.text)).font_family("Microsoft YaHei UI").text_size(px(13.))
            .flex().flex_col()
            .child(TitleBar::new().bg(rgb(colors.sidebar)).border_color(rgb(colors.border))
                .child(div().flex().items_center().gap_2().text_size(px(11.)).text_color(rgb(colors.muted))
                    .child(icon(IconName::Globe)).child("DNSPing").child(div().text_color(rgb(colors.subtle)).child("/")).child("网络连接实验室")))
            .child(div().flex().flex_1().min_h_0().child(self.sidebar(cx))
            .child(div().flex_1().min_w_0().h_full().flex().flex_col().p_4().gap_3()
                .child(div().flex().items_center().justify_between()
                    .child(div().flex().flex_col().gap_1()
                        .child(div().text_size(px(22.)).font_weight(FontWeight::BOLD).child("优选你的 DNS"))
                        .child(label("解析与连接质量对比 · 辅助选择 SmartDNS 上游")))
                    .child(div().flex().items_center().gap_3()
                        .child(Button::new("toggle-theme").small().ghost()
                            .icon(if self.settings.dark_theme { IconName::Sun } else { IconName::Moon })
                            .label(if self.settings.dark_theme { "浅色" } else { "深色" })
                            .tooltip("切换深色 / 浅色主题")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.settings.dark_theme = !this.settings.dark_theme;
                                ui_theme::apply(this.settings.dark_theme, cx);
                                this.persist(); window.refresh(); cx.notify();
                            })))
                        .child(div().flex().items_center().gap_2().px_3().py_2().rounded_full().bg(rgb(colors.accent_surface))
                        .child(div().size(px(6.)).rounded_full().bg(rgb(colors.accent)))
                        .child(div().text_color(rgb(colors.accent)).text_size(px(12.)).child(if self.running { "测试进行中" } else { "本机网络实测" })))))
                .child(self.controls(cx))
                .child(self.recommendation(cx))
                .when(self.running, |el| el.child(div().h(px(3.)).rounded_full().bg(rgb(colors.border)).child(div().h_full().w(relative(progress.max(0.02))).rounded_full().bg(rgb(colors.accent)))))
                .child(self.results(cx))
                .child(div().flex().items_center().gap_2().text_size(px(12.)).text_color(rgb(if self.error { colors.error } else { colors.muted }))
                    .child(icon(if self.error { IconName::TriangleAlert } else { IconName::Info }))
                    .child(div().flex_1().min_w_0().child(self.message.clone())))
                .child(div().text_size(px(11.)).text_color(rgb(colors.subtle)).child(if self.settings.probe == ProbeKind::Tcp {
                    "TCPing 测量端口建连耗时；推荐适用于本轮网络和端口，不代表业务响应速度。"
                } else { "Ping 使用 ICMP；部分服务器禁用 ICMP，无应答不一定表示网站不可访问。" }))))
    }
}
