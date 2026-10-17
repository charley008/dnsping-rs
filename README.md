# DNSPing

Windows 专用的 Rust + GPUI 桌面 DNS 对比测速工具，辅助选择 SmartDNS 上游。GPU 绘制界面，支持中文深色 / 浅色主题，不需要浏览器或 WebView2。

![DNSPing Windows 实际运行界面](docs/preview.jpg)

## 使用

从 [GitHub Releases](https://github.com/charley008/dnsping-rs/releases/latest) 下载 `DNSPing-windows-x64.zip`，解压后即可使用，无需编译环境。

运行打包后的 `DNSPing.exe`（本地打包位于 `dist/DNSPing.exe`）。选择左侧 DNS 服务商，输入纯域名，点击「开始测试」或在域名输入框按 Enter。

1. 每个 DNS 独立查询，取响应中对应域名 / CNAME 链上的第一个 A 或 AAAA 地址。
2. 对不同 IP 并发执行测速。同一个 IP 只测试一组样本，供各 DNS 行共用。
3. 整轮结束后，先按成功率降序，再按平均延迟升序推荐 IP。DNS 查询时间独立展示。

默认 IPv4、TCP 80 端口、5 次、2000 ms 超时。端口可改为 443；Ping 使用 Windows ICMP API，支持 IPv4 和 IPv6。可中途停止、复制 IP / 完整结果、通过系统「另存为」对话框导出 CSV。

内置 DNS：

| 名称 | 地址 | 默认启用 |
| --- | --- | --- |
| 本地运营商 | 202.103.224.68 | 是 |
| 阿里 DNS | 223.5.5.5 | 是 |
| 腾讯 DNS | 119.29.29.29 | 是 |
| 114 DNS | 114.114.114.114 | 是 |
| Cloudflare | 1.1.1.1 | 否 |
| Google | 8.8.8.8 | 否 |

每个服务商右侧都有「编辑」按钮，内置及自定义 DNS 均可修改名称和 IP 地址；表单会填入原值，点击「保存修改」更新原条目，点击「取消」放弃修改。编辑保留原来的勾选状态，并检查无效地址及重复 IP。支持添加、删除自定义 DNS；名称 1–24 字符，地址为 IPv4 或 IPv6，DNS 端口固定为 53。最多 32 个服务商。次数 1–30，超时 200–10000 ms。

右上角可一键切换深色 / 浅色主题，并记住上次选择。推荐区域采用紧凑横条，默认窗口可展示至少 5 条常规测速结果；更多结果可继续滚动查看。推荐 DNS 和其返回 IP 均可一键复制。

配置保存在 **`DNSPing.exe` 同目录的 `settings.json`**，包括域名、测试设置、主题和 DNS 列表。首次启动会创建配置；如果存在旧版 `%LOCALAPPDATA%\DNSPing\settings.json`，会迁移到程序目录并保留旧文件。左下角「配置文件」可定位该文件。应用不修改系统 DNS、hosts 或其他网络设置，不自动开始测试。

可以关闭程序后手动编辑 JSON，再重启生效；格式参考 [settings.example.json](settings.example.json)。名称与地址分别为 `name`、`address`，`enabled` 控制是否参与测试；`query` 可选 `Ipv4` / `Ipv6`，`probe` 可选 `Tcp` / `Icmp`，`dark_theme` 为 `true` / `false`。DNS 条目的 `enabled`、`custom` 字段可省略，省略时均为 `true`。支持 UTF-8 BOM；无效配置会提示，恢复保存前会备份为 `settings.invalid-*.json`。

程序目录需要可写。发行包包含默认配置 `settings.json`。升级时关闭程序，保留原有 `settings.json`，仅替换程序及其他文件，以免覆盖你的 DNS 配置。

## 如何理解结果

- 「解析」是 DNS 查询耗时；「平均」是成功测速样本的平均延迟；失败样本不会当作 0 ms。
- 「成功 / 已测」例如 4/5 表示 5 次采样中成功 4 次。采样条中红色表示失败；抖动是相邻成功样本的延迟差绝对值的平均值。
- 推荐针对本轮网络、协议、端口和 DNS 响应。TCP 连接测试不验证 HTTP 响应、TLS 证书或 SNI；ICMP 可能被目标服务器禁用。
- 取消的测试保留已有结果，但不产生最终推荐。全失败时也不会推荐一个虚假的“最快”地址。
- 多行数据可滚动查看；长 IPv6 地址可通过复制按钮取得完整值。
- CSV 使用 UTF-8 BOM，方便 Excel 正确显示中文，并处理自定义名称中的 CSV 引号和公式前缀。

## 构建

目标平台：Windows x64，GPU 渲染使用系统 Direct3D 11，文本使用 DirectWrite。

开发环境：稳定版 Rust MSVC 工具链、Visual Studio 2022 Build Tools 的 C++ 桌面开发组件及 Windows SDK。本次开发使用 Rust 1.99.0。依赖版本锁定在 `Cargo.lock`，GPUI 与组件库采用已验证兼容的 0.2.2 / 0.5.1 组合。

```powershell
cargo run --locked --bin dnsping
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --all -- --check
cargo build --locked --release --bin dnsping
```

`.cargo/config.toml` 静态链接 C 运行库，Release 程序隐藏控制台，图标资源已嵌入。产物位于 `target/release/dnsping.exe`。

执行 `scripts/package.ps1` 可生成 `dist/DNSPing-windows-x64.zip` 与 ZIP 的 `dist/SHA256SUMS.txt` 校验文件。ZIP 内仅包含 `DNSPing.exe`、`settings.json`、`LICENSE.txt` 和用于校验程序的 `SHA256SUMS.txt`。

## 无界面诊断

额外提供诊断入口，供复现与排查使用，使用相同的网络引擎：

```powershell
cargo run --locked --bin dnsping-check -- remoteassistance.support.services.microsoft.com
cargo run --locked --bin dnsping-check -- www.example.com --dns 223.5.5.5,119.29.29.29 --port 443 --samples 3
cargo run --locked --bin dnsping-check -- www.example.com --icmp --ipv6
```

支持 `--dns`、`--port`、`--samples`、`--timeout`、`--icmp`、`--ipv6`；输出 CSV 与最终推荐 IP。诊断入口不会写入图形界面配置。

架构与实现步骤见 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)，验证记录见 [docs/VALIDATION.md](docs/VALIDATION.md)。

本项目通过 Codex（GPT-6.1 SOL）辅助开发。
