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
    net::IpAddr,
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

const BG: u32 = 0x0b111b;
const PANEL: u32 = 0x111b29;
const BORDER: u32 = 0x243246;
const TEXT: u32 = 0xe8eef7;
const MUTED: u32 = 0x8b9bb0;
const ACCENT: u32 = 0x5ce0c4;

fn label(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.))
        .text_color(rgb(MUTED))
        .child(text.into())
}
fn panel() -> Div {
    div()
        .bg(rgb(PANEL))
        .border_1()
        .border_color(rgb(BORDER))
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

pub fn theme(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    let theme = Theme::global_mut(cx);
    theme.font_family = "Microsoft YaHei UI".into();
    theme.font_size = px(14.);
    theme.mono_font_family = "Consolas".into();
    theme.radius = px(8.);
    theme.background = rgb(BG).into();
    theme.foreground = rgb(TEXT).into();
    theme.primary = rgb(ACCENT).into();
    theme.primary_hover = rgb(0x7aead3).into();
    theme.primary_active = rgb(0x35c7aa).into();
    theme.primary_foreground = rgb(0x08251f).into();
    theme.secondary = rgb(0x1a283a).into();
    theme.secondary_hover = rgb(0x26384e).into();
    theme.secondary_foreground = rgb(TEXT).into();
    theme.border = rgb(BORDER).into();
    theme.input = rgb(BORDER).into();
    theme.muted = rgb(0x182435).into();
    theme.muted_foreground = rgb(MUTED).into();
    theme.caret = rgb(ACCENT).into();
    theme.ring = rgb(ACCENT).into();
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (settings, warning) = config::load();
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

    fn add_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.custom_name.read(cx).value().trim().to_owned();
        let ip = self.custom_ip.read(cx).value().trim().parse::<IpAddr>();
        let error = match ip {
            _ if self.settings.servers.len() >= 32 => Some("最多支持 32 个 DNS 服务器"),
            _ if name.is_empty() || name.chars().count() > 24 => Some("服务商名称需要 1–24 个字符"),
            Err(_) => Some("请输入有效的 DNS IP 地址"),
            Ok(ip) if ip.is_unspecified() || ip.is_multicast() => {
                Some("不能使用未指定地址或组播地址")
            }
            Ok(ip) if self.settings.servers.iter().any(|s| s.address == ip) => {
                Some("这个 DNS 地址已经在列表中")
            }
            Ok(ip) => {
                self.settings.servers.push(DnsServer {
                    name,
                    address: ip,
                    enabled: true,
                    custom: true,
                });
                self.adding = false;
                self.custom_name
                    .update(cx, |s, cx| s.set_value("", window, cx));
                self.custom_ip
                    .update(cx, |s, cx| s.set_value("", window, cx));
                self.persist();
                None
            }
        };
        if let Some(error) = error {
            self.message = error.into();
            self.error = true;
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
        let mut servers = div()
            .id("servers")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2();
        for (index, server) in self.settings.servers.iter().enumerate() {
            let color = [0x70a7ff, 0xffad66, 0x71b7ff, 0xbb9aff, 0xf6bd70, 0x7fd4a5][index.min(5)];
            servers = servers.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .p_3()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(rgb(if server.enabled { BORDER } else { 0x172334 }))
                    .bg(rgb(if server.enabled { 0x172334 } else { 0x101a28 }))
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
                                    .text_color(rgb(color))
                                    .child(server.name.clone()),
                            )
                            .child(
                                div()
                                    .font_family("Consolas")
                                    .text_size(px(12.))
                                    .text_color(rgb(MUTED))
                                    .child(server.address.to_string()),
                            ),
                    )
                    .when(server.custom, |el| {
                        el.child(
                            Button::new(("delete", index))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Close)
                                .tooltip("删除自定义 DNS")
                                .disabled(self.running)
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
            .bg(rgb(0x0e1724))
            .border_r_1()
            .border_color(rgb(BORDER))
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
                            .bg(rgb(0x173a37))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(rgb(ACCENT))
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
            .child(div().h(px(1.)).bg(rgb(BORDER)))
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
                    panel()
                        .p_3()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(Input::new(&self.custom_name).small())
                        .child(Input::new(&self.custom_ip).small())
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(
                                    Button::new("save-dns")
                                        .primary()
                                        .small()
                                        .label("添加")
                                        .on_click(cx.listener(Self::on_add)),
                                )
                                .child(
                                    Button::new("cancel-dns")
                                        .ghost()
                                        .small()
                                        .label("取消")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.adding = false;
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
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.adding = true;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .p_3()
                    .rounded(px(10.))
                    .bg(rgb(0x142532))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .items_center()
                            .text_color(rgb(0x8dc7e9))
                            .child(icon(IconName::Info))
                            .child("独立查询 · 真实测速"),
                    )
                    .child(label("每个 DNS 取首个 IP；相同 IP 自动合并测速。")),
            )
            .child(label("WINDOWS  /  v0.1.0"))
    }

    fn on_add(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.add_server(window, cx);
    }

    fn controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ipv4 = self.settings.query == QueryKind::Ipv4;
        let tcp = self.settings.probe == ProbeKind::Tcp;
        panel()
            .p_5()
            .flex()
            .flex_col()
            .gap_4()
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
        let best = self
            .completed
            .then(|| best_row(&self.rows))
            .flatten()
            .map(|i| &self.rows[i]);
        let title = if best.is_some() {
            "本轮最佳 IP"
        } else if self.running {
            "正在寻找更好的连接"
        } else if self.completed {
            "本轮没有可用的 IP"
        } else {
            "让每一次连接，都有更好的起点"
        };
        let mut card = div()
            .rounded(px(14.))
            .border_1()
            .border_color(rgb(0x2a514c))
            .bg(rgb(0x112a2b))
            .p_5()
            .flex()
            .items_center()
            .gap_5()
            .min_h(px(128.))
            .child(
                div()
                    .size(px(44.))
                    .flex_none()
                    .rounded(px(13.))
                    .bg(rgb(0x1b403b))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(ACCENT))
                    .child(icon(IconName::Star)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(ACCENT))
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(
                                if best.is_some_and(|r| r.ip.is_some_and(|ip| ip.is_ipv6())) {
                                    20.
                                } else if best.is_some() {
                                    29.
                                } else {
                                    21.
                                },
                            ))
                            .font_weight(FontWeight::SEMIBOLD)
                            .when(best.is_some(), |el| el.font_family("Consolas"))
                            .child(
                                best.and_then(|r| r.ip)
                                    .map(|ip| ip.to_string())
                                    .unwrap_or_else(|| {
                                        if self.running {
                                            "解析与测速进行中…".into()
                                        } else if self.completed {
                                            "请检查网络或更换测试端口".into()
                                        } else {
                                            "比较 DNS，发现更快的 IP".into()
                                        }
                                    }),
                            ),
                    )
                    .child(label(
                        best.map(|r| {
                            format!("来自 {} · 优先成功率，再比较平均延迟", r.server.name)
                        })
                        .unwrap_or_else(|| "同一域名，不同 DNS，可能通向不同的网络路径。".into()),
                    )),
            );
        if let Some(row) = best {
            let ip = row.ip.unwrap().to_string();
            card = card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .items_end()
                    .child(
                        div()
                            .text_size(px(25.))
                            .font_family("Consolas")
                            .text_color(rgb(ACCENT))
                            .child(format!("{} ms", number(row.stats.average()))),
                    )
                    .child(label("平均连接延迟"))
                    .child(
                        Button::new("copy-best")
                            .small()
                            .ghost()
                            .icon(IconName::Copy)
                            .label("复制 IP")
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.copy_ip(ip.clone(), cx)),
                            ),
                    ),
            );
        }
        card
    }

    fn results(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                RowStatus::Resolving => ("解析中".to_owned(), 0x8bb5ff),
                RowStatus::Waiting => ("等待测速".into(), MUTED),
                RowStatus::Probing => (
                    format!("测速 {}/{}", row.stats.samples.len(), snapshot.samples),
                    0x8bb5ff,
                ),
                RowStatus::Complete => (if winner { "最佳" } else { "已完成" }.into(), ACCENT),
                RowStatus::Failed(_) => ("失败".into(), 0xf59898),
                RowStatus::Cancelled => ("已停止".into(), MUTED),
            };
            let ip = row.ip.map(|ip| ip.to_string());
            let mut content = div()
                .flex()
                .flex_col()
                .border_b_1()
                .border_color(rgb(0x1c2a3c))
                .bg(rgb(if winner { 0x142e2e } else { PANEL }))
                .px_4()
                .py_3()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .min_h(px(36.))
                        .child(
                            div()
                                .w(px(116.))
                                .flex_none()
                                .flex()
                                .flex_col()
                                .gap_1()
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
                                .text_color(rgb(if winner { ACCENT } else { TEXT }))
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
                        .text_color(rgb(0xca8f92))
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
                        .child(div().flex().gap_1().items_end().h(px(14.)).children(
                            row.stats.samples.iter().map(|sample| {
                                div()
                                    .w(px(7.))
                                    .rounded(px(2.))
                                    .h(px(sample
                                        .as_ref()
                                        .map(|ms| (ms / max * 13.).max(3.) as f32)
                                        .unwrap_or(13.)))
                                    .bg(rgb(if sample.is_ok() {
                                        if winner { ACCENT } else { 0x688fca }
                                    } else {
                                        0xca7979
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
                            .bg(rgb(0x19293d))
                            .text_color(rgb(0x82a9d7))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon(IconName::ChartPie)),
                    )
                    .child(
                        div()
                            .text_color(rgb(0xc0cde0))
                            .child("你的下一次测试，从这里开始"),
                    )
                    .child(label("选择左侧 DNS，输入域名，按 Enter 或点击开始测试。")),
            );
        }
        panel()
            .flex_1()
            .min_h(px(180.))
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_4()
                    .py_3()
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
                    .bg(rgb(0x152031))
                    .border_y_1()
                    .border_color(rgb(BORDER))
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
        div().size_full().bg(rgb(BG)).text_color(rgb(TEXT)).font_family("Microsoft YaHei UI").text_size(px(13.))
            .flex().flex_col()
            .child(TitleBar::new().bg(rgb(0x0e1724)).border_color(rgb(BORDER))
                .child(div().flex().items_center().gap_2().text_size(px(11.)).text_color(rgb(MUTED))
                    .child(icon(IconName::Globe)).child("DNSPing").child(div().text_color(rgb(0x40536c)).child("/")).child("网络连接实验室")))
            .child(div().flex().flex_1().min_h_0().child(self.sidebar(cx))
            .child(div().flex_1().min_w_0().h_full().flex().flex_col().p_6().gap_4()
                .child(div().flex().items_center().justify_between()
                    .child(div().flex().flex_col().gap_2()
                        .child(div().text_size(px(26.)).font_weight(FontWeight::BOLD).child("发现更快的 IP"))
                        .child(label("多 DNS 解析对比与网络延迟测试")))
                    .child(div().flex().items_center().gap_2().px_3().py_2().rounded_full().bg(rgb(0x172a2a))
                        .child(div().size(px(6.)).rounded_full().bg(rgb(ACCENT)))
                        .child(div().text_color(rgb(ACCENT)).text_size(px(12.)).child(if self.running { "测试进行中" } else { "本机网络实测" }))))
                .child(self.controls(cx))
                .child(self.recommendation(cx))
                .when(self.running, |el| el.child(div().h(px(3.)).rounded_full().bg(rgb(BORDER)).child(div().h_full().w(relative(progress.max(0.02))).rounded_full().bg(rgb(ACCENT)))))
                .child(self.results(cx))
                .child(div().flex().items_center().gap_2().text_size(px(12.)).text_color(rgb(if self.error { 0xf59898 } else { MUTED }))
                    .child(icon(if self.error { IconName::TriangleAlert } else { IconName::Info }))
                    .child(div().flex_1().min_w_0().child(self.message.clone())))
                .child(div().text_size(px(11.)).text_color(rgb(0x64778f)).child(if self.settings.probe == ProbeKind::Tcp {
                    "TCPing 测量端口建连耗时；推荐适用于本轮网络和端口，不代表业务响应速度。"
                } else { "Ping 使用 ICMP；部分服务器禁用 ICMP，无应答不一定表示网站不可访问。" }))))
    }
}
