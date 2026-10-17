use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, net::IpAddr};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueryKind {
    #[default]
    Ipv4,
    Ipv6,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProbeKind {
    Tcp,
    #[default]
    Icmp,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DnsServer {
    pub name: String,
    pub address: IpAddr,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub custom: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub domain: String,
    pub query: QueryKind,
    pub probe: ProbeKind,
    pub port: u16,
    pub samples: u32,
    pub timeout_ms: u64,
    pub dark_theme: bool,
    pub servers: Vec<DnsServer>,
}

impl Default for Settings {
    fn default() -> Self {
        let servers = [
            ("本地运营商", "202.103.224.68", true),
            ("腾讯 DNS", "119.29.29.29", true),
            ("阿里 DNS", "223.5.5.5", true),
            ("字节跳动", "180.184.1.1", true),
            ("114DNS", "114.114.114.114", true),
        ]
        .into_iter()
        .map(|(name, address, enabled)| DnsServer {
            name: name.into(),
            address: address.parse().unwrap(),
            enabled,
            custom: false,
        })
        .collect();
        Self {
            domain: "account.microsoft.com".into(),
            query: QueryKind::Ipv4,
            probe: ProbeKind::Icmp,
            port: 80,
            samples: 5,
            timeout_ms: 2000,
            dark_theme: true,
            servers,
        }
    }
}

impl Settings {
    /// Create or edit a provider without replacing its selection or built-in identity.
    pub fn save_server(&mut self, index: Option<usize>, name: &str, address: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 24 {
            bail!("服务商名称需要 1–24 个字符");
        }
        let address: IpAddr = address
            .trim()
            .parse()
            .map_err(|_| anyhow::anyhow!("请输入有效的 DNS IP 地址"))?;
        if address.is_unspecified() || address.is_multicast() {
            bail!("不能使用未指定地址或组播地址");
        }
        if index.is_some_and(|i| i >= self.servers.len()) {
            bail!("要编辑的 DNS 已不存在，请重新选择");
        }
        if self
            .servers
            .iter()
            .enumerate()
            .any(|(i, s)| Some(i) != index && s.address == address)
        {
            bail!("这个 DNS 地址已经在列表中");
        }
        if let Some(index) = index {
            self.servers[index].name = name.into();
            self.servers[index].address = address;
        } else {
            if self.servers.len() >= 32 {
                bail!("最多支持 32 个 DNS 服务器");
            }
            self.servers.push(DnsServer {
                name: name.into(),
                address,
                enabled: true,
                custom: true,
            });
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<String> {
        let domain = normalize_domain(&self.domain)?;
        if self.port == 0 && self.probe == ProbeKind::Tcp {
            bail!("端口范围为 1–65535");
        }
        if !(1..=30).contains(&self.samples) {
            bail!("测试次数范围为 1–30");
        }
        if !(200..=10000).contains(&self.timeout_ms) {
            bail!("超时范围为 200–10000 ms");
        }
        if !self.servers.iter().any(|s| s.enabled) {
            bail!("请至少选择一个 DNS 服务器");
        }
        if self.servers.len() > 32 {
            bail!("最多支持 32 个 DNS 服务器");
        }
        if self
            .servers
            .iter()
            .any(|s| s.address.is_unspecified() || s.address.is_multicast())
        {
            bail!("DNS 地址不能是未指定地址或组播地址");
        }
        Ok(domain)
    }
}

pub fn normalize_domain(input: &str) -> Result<String> {
    let input = input.trim().trim_end_matches('.');
    if input.is_empty() {
        bail!("请输入要查询的域名");
    }
    if input.contains(['/', ':', '\\']) || input.chars().any(char::is_whitespace) {
        bail!("请输入纯域名，例如 www.example.com，不要包含协议、端口或路径");
    }
    if input.parse::<IpAddr>().is_ok() {
        bail!("请输入域名，而不是 IP 地址");
    }
    let ascii = idna::domain_to_ascii(input).map_err(|_| anyhow::anyhow!("域名格式不正确"))?;
    if ascii.len() > 253
        || ascii.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
    {
        bail!("域名格式不正确");
    }
    Ok(ascii.to_ascii_lowercase())
}

#[derive(Clone, Debug)]
pub enum RowStatus {
    Resolving,
    Waiting,
    Probing,
    Complete,
    Failed(String),
    Cancelled,
}

#[derive(Clone, Debug, Default)]
pub struct ProbeStats {
    pub samples: Vec<Result<f64, String>>,
}

impl ProbeStats {
    pub fn successes(&self) -> usize {
        self.samples.iter().filter(|s| s.is_ok()).count()
    }
    pub fn average(&self) -> Option<f64> {
        let count = self.successes();
        (count > 0).then(|| {
            self.samples
                .iter()
                .filter_map(|s| s.as_ref().ok())
                .sum::<f64>()
                / count as f64
        })
    }
    pub fn minimum(&self) -> Option<f64> {
        self.samples
            .iter()
            .filter_map(|s| s.as_ref().ok())
            .copied()
            .reduce(f64::min)
    }
    pub fn jitter(&self) -> Option<f64> {
        let values: Vec<f64> = self
            .samples
            .iter()
            .filter_map(|s| s.as_ref().ok())
            .copied()
            .collect();
        let pairs: Vec<_> = values.windows(2).collect();
        (!pairs.is_empty())
            .then(|| pairs.iter().map(|s| (s[1] - s[0]).abs()).sum::<f64>() / pairs.len() as f64)
    }
    pub fn last_error(&self) -> Option<&str> {
        self.samples
            .iter()
            .rev()
            .find_map(|s| s.as_ref().err().map(String::as_str))
    }
}

#[derive(Clone, Debug)]
pub struct ResultRow {
    pub server: DnsServer,
    pub ip: Option<IpAddr>,
    pub dns_ms: Option<f64>,
    pub stats: ProbeStats,
    pub status: RowStatus,
}

impl ResultRow {
    pub fn new(server: DnsServer) -> Self {
        Self {
            server,
            ip: None,
            dns_ms: None,
            stats: ProbeStats::default(),
            status: RowStatus::Resolving,
        }
    }
}

pub fn compare_rows(a: &ResultRow, b: &ResultRow) -> Ordering {
    // Cross multiplication avoids rounding percentages; zero successes are never candidates.
    let left = a.stats.successes() * b.stats.samples.len().max(1);
    let right = b.stats.successes() * a.stats.samples.len().max(1);
    right.cmp(&left).then_with(|| {
        a.stats
            .average()
            .unwrap_or(f64::INFINITY)
            .total_cmp(&b.stats.average().unwrap_or(f64::INFINITY))
    })
}

pub fn best_row(rows: &[ResultRow]) -> Option<usize> {
    rows.iter()
        .enumerate()
        .filter(|(_, r)| matches!(r.status, RowStatus::Complete) && r.stats.successes() > 0)
        .min_by(|(_, a), (_, b)| compare_rows(a, b))
        .map(|(i, _)| i)
}

pub fn csv_report(settings: &Settings, rows: &[ResultRow]) -> String {
    fn cell(s: &str) -> String {
        // Also neutralize spreadsheet formulas in custom server names.
        let safe = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) {
            format!("'{s}")
        } else {
            s.to_owned()
        };
        format!("\"{}\"", safe.replace('"', "\"\""))
    }
    let mut out = String::from(
        "\u{feff}域名,DNS 服务商,DNS 地址,首个 IP,解析耗时 ms,测速方式,端口,平均延迟 ms,最小延迟 ms,成功次数,已测次数,计划次数,状态\r\n",
    );
    for r in rows {
        let status = match &r.status {
            RowStatus::Complete => "完成".to_owned(),
            RowStatus::Failed(e) => e.clone(),
            RowStatus::Cancelled => "已取消".into(),
            _ => "进行中".into(),
        };
        let fields = [
            settings.domain.clone(),
            r.server.name.clone(),
            r.server.address.to_string(),
            r.ip.map(|ip| ip.to_string()).unwrap_or_default(),
            r.dns_ms.map(|v| format!("{v:.2}")).unwrap_or_default(),
            if settings.probe == ProbeKind::Tcp {
                "TCP"
            } else {
                "ICMP"
            }
            .into(),
            if settings.probe == ProbeKind::Tcp {
                settings.port.to_string()
            } else {
                String::new()
            },
            r.stats
                .average()
                .map(|v| format!("{v:.2}"))
                .unwrap_or_default(),
            r.stats
                .minimum()
                .map(|v| format!("{v:.2}"))
                .unwrap_or_default(),
            r.stats.successes().to_string(),
            r.stats.samples.len().to_string(),
            settings.samples.to_string(),
            status,
        ];
        out.push_str(&fields.iter().map(|s| cell(s)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_builtin_and_custom_preserves_selection_and_identity() {
        let mut settings = Settings::default();
        settings.servers[0].enabled = false;
        settings
            .save_server(Some(0), "运营商备用", "202.103.224.69")
            .unwrap();
        assert_eq!(settings.servers.len(), 5);
        assert!(!settings.servers[0].enabled);
        assert!(!settings.servers[0].custom);
        assert_eq!(settings.servers[0].address.to_string(), "202.103.224.69");
        // Keeping one's own IP is valid when only changing the name.
        settings
            .save_server(Some(0), "本地 DNS", "202.103.224.69")
            .unwrap();
        settings.save_server(None, "自定义", "8.8.4.4").unwrap();
        settings.servers[5].enabled = false;
        settings
            .save_server(Some(5), "IPv6 DNS", "2606:4700:4700::1111")
            .unwrap();
        assert_eq!(settings.servers.len(), 6);
        assert!(settings.servers[5].custom);
        assert!(!settings.servers[5].enabled);
        assert!(settings.servers[5].address.is_ipv6());
    }

    #[test]
    fn invalid_edits_do_not_change_any_provider() {
        let mut settings = Settings::default();
        let before = serde_json::to_value(&settings).unwrap();
        for (index, name, address) in [
            (Some(0), "重复", "223.5.5.5"),
            (Some(0), "", "8.8.4.4"),
            (Some(0), "错误", "https://1.1.1.1"),
            (Some(0), "错误", "::"),
            (Some(99), "不存在", "8.8.4.4"),
        ] {
            assert!(settings.save_server(index, name, address).is_err());
            assert_eq!(serde_json::to_value(&settings).unwrap(), before);
        }
    }

    #[test]
    fn full_provider_list_still_allows_edits() {
        let mut settings = Settings::default();
        while settings.servers.len() < 32 {
            let n = settings.servers.len();
            settings
                .save_server(None, &format!("测试 {n}"), &format!("192.0.2.{n}"))
                .unwrap();
        }
        settings
            .save_server(Some(0), "编辑已有服务商", "8.8.4.4")
            .unwrap();
        assert!(
            settings
                .save_server(None, "超出上限", "192.0.2.100")
                .is_err()
        );
        assert_eq!(settings.servers.len(), 32);
    }

    #[test]
    fn pre_theme_config_keeps_existing_provider_edits() {
        let mut settings = Settings {
            domain: "example.com".into(),
            ..Default::default()
        };
        settings
            .save_server(Some(0), "本地 DNS", "202.103.224.69")
            .unwrap();
        let mut json = serde_json::to_value(&settings).unwrap();
        json.as_object_mut().unwrap().remove("dark_theme");
        let loaded: Settings = serde_json::from_value(json).unwrap();
        assert!(loaded.dark_theme);
        assert_eq!(loaded.domain, "example.com");
        assert_eq!(loaded.servers[0].name, "本地 DNS");
        assert_eq!(loaded.servers[0].address.to_string(), "202.103.224.69");
    }
    #[test]
    fn domain_validation() {
        assert_eq!(
            normalize_domain(" WWW.Example.COM. ").unwrap(),
            "www.example.com"
        );
        assert!(normalize_domain("中文.cn").unwrap().starts_with("xn--"));
        for s in [
            "",
            "https://example.com",
            "a..com",
            "a.com:80",
            "1.1.1.1",
            "-a.com",
            "a_.com",
        ] {
            assert!(normalize_domain(s).is_err(), "{s}");
        }
    }
    fn row(samples: Vec<Result<f64, String>>) -> ResultRow {
        let mut r = ResultRow::new(Settings::default().servers.remove(0));
        r.stats.samples = samples;
        r.status = RowStatus::Complete;
        r
    }
    #[test]
    fn reliability_precedes_latency_and_failures_never_win() {
        let rows = vec![
            row(vec![Ok(5.), Err("timeout".into())]),
            row(vec![Ok(100.), Ok(120.)]),
            row(vec![Err("timeout".into())]),
        ];
        assert_eq!(best_row(&rows), Some(1));
        assert_eq!(best_row(&[row(vec![Err("timeout".into())])]), None);
        assert_eq!(best_row(&[row(vec![Ok(30.)]), row(vec![Ok(10.)])]), Some(1));
        assert_eq!(best_row(&[row(vec![Ok(10.)]), row(vec![Ok(10.)])]), Some(0));
    }
    #[test]
    fn csv_escapes_custom_names_and_formulas() {
        let mut r = row(vec![Ok(20.)]);
        r.server.name = "=SUM(1,2)\"".into();
        let csv = csv_report(&Settings::default(), &[r]);
        assert!(csv.contains("\"'=SUM(1,2)\"\"\""));
        assert!(csv.starts_with('\u{feff}'));
    }
}
