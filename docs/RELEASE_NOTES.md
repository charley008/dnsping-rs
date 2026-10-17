Windows x64 便携版，下载 `DNSPing-windows-x64.zip`，解压后运行 `DNSPing.exe`。无需安装 Rust、Go 或 WebView2。

- 向多个 DNS 独立查询，比较解析耗时和首个返回 IP 的 TCP / ICMP 延迟，辅助选择 SmartDNS 上游。
- 内置与自定义 DNS 均可编辑名称和 IP，支持添加、删除自定义 DNS。
- 深色 / 浅色一键切换，自动记住主题；紧凑结果布局。
- 复制推荐 DNS / IP、取消测试、导出 CSV。
- 配置为程序同目录的 `settings.json`，可以关闭程序后用文本编辑器手动修改；首次使用会创建配置或迁移旧配置。

升级时关闭程序，再用新版文件覆盖原目录；发行 ZIP 不包含实际 `settings.json`，会保留你的配置。`settings.example.json` 仅作格式参考。

`DNSPing.exe` 是独立可执行文件；`DNSPing-source.zip` 是源码备份；`SHA256SUMS.txt` 用于校验 exe。
