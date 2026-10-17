//! Headless diagnostic runner for reproducing network behavior without a GUI.
use anyhow::{Context, Result, bail};
use dnsping::{
    engine::{self, Event},
    model::*,
};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let Some(domain) = args.next() else {
        println!(
            "Usage: dnsping-check DOMAIN [--dns IP,IP] [--port 80] [--samples 5] [--timeout 2000] [--icmp] [--ipv6]"
        );
        return Ok(());
    };
    let mut settings = Settings {
        domain,
        probe: ProbeKind::Tcp,
        ..Default::default()
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--icmp" => settings.probe = ProbeKind::Icmp,
            "--ipv6" => settings.query = QueryKind::Ipv6,
            "--port" => settings.port = args.next().context("missing port")?.parse()?,
            "--samples" => settings.samples = args.next().context("missing samples")?.parse()?,
            "--timeout" => settings.timeout_ms = args.next().context("missing timeout")?.parse()?,
            "--dns" => {
                settings.servers = args
                    .next()
                    .context("missing DNS addresses")?
                    .split(',')
                    .map(|ip| {
                        Ok(DnsServer {
                            name: ip.trim().into(),
                            address: ip.trim().parse()?,
                            enabled: true,
                            custom: true,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
            }
            _ => bail!("Unknown argument: {arg}"),
        }
    }
    settings.domain = settings.validate()?;
    let mut rows: Vec<_> = settings
        .servers
        .iter()
        .filter(|s| s.enabled)
        .cloned()
        .map(ResultRow::new)
        .collect();
    let (tx, rx) = std::sync::mpsc::channel();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(4)
        .build()?;
    runtime.spawn(engine::run(settings.clone(), tx, CancellationToken::new()));
    while let Ok(event) = rx.recv_timeout(Duration::from_secs(350)) {
        engine::apply_event(&mut rows, &event, settings.samples);
        if matches!(event, Event::Finished) {
            break;
        }
    }
    print!("{}", csv_report(&settings, &rows));
    if let Some(index) = best_row(&rows) {
        println!("Best IP: {}", rows[index].ip.unwrap());
    } else {
        bail!("No reachable IP found");
    }
    Ok(())
}
