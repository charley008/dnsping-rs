use crate::{dns, model::*, probe};
use std::{
    collections::BTreeMap,
    net::{IpAddr, SocketAddr},
    sync::mpsc::Sender,
    time::Duration,
};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub enum Event {
    Resolved {
        row: usize,
        result: Result<dns::Resolution, String>,
    },
    Sample {
        ip: IpAddr,
        result: Result<f64, String>,
    },
    Finished,
    Cancelled,
}

pub async fn run(settings: Settings, sender: Sender<Event>, cancel: CancellationToken) {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => { let _ = sender.send(Event::Cancelled); },
        _ = run_inner(settings, &sender) => { let _ = sender.send(Event::Finished); },
    }
}

async fn run_inner(settings: Settings, sender: &Sender<Event>) {
    let timeout = Duration::from_millis(settings.timeout_ms);
    let mut queries = JoinSet::new();
    for (row, server) in settings.servers.iter().filter(|s| s.enabled).enumerate() {
        let domain = settings.domain.clone();
        let server = server.address;
        let kind = settings.query;
        queries.spawn(async move {
            (
                row,
                dns::resolve(&domain, SocketAddr::new(server, 53), kind, timeout)
                    .await
                    .map_err(|e| format!("{e:#}")),
            )
        });
    }
    let mut addresses = BTreeMap::new();
    while let Some(result) = queries.join_next().await {
        if let Ok((row, resolution)) = result {
            if let Ok(r) = &resolution {
                addresses.insert(r.ip, ());
            }
            let _ = sender.send(Event::Resolved {
                row,
                result: resolution,
            });
        }
    }
    let mut probes = JoinSet::new();
    for (ip, _) in addresses {
        let sender = sender.clone();
        let count = settings.samples;
        let kind = settings.probe;
        let port = settings.port;
        probes.spawn(async move {
            for index in 0..count {
                let result = probe::sample(ip, kind, port, timeout)
                    .await
                    .map_err(|e| format!("{e:#}"));
                if sender.send(Event::Sample { ip, result }).is_err() {
                    return;
                }
                if index + 1 < count {
                    tokio::time::sleep(Duration::from_millis(180)).await;
                }
            }
        });
    }
    while probes.join_next().await.is_some() {}
}

pub fn apply_event(rows: &mut [ResultRow], event: &Event, count: u32) {
    match event {
        Event::Resolved { row, result } => {
            if let Some(row) = rows.get_mut(*row) {
                match result {
                    Ok(r) => {
                        row.ip = Some(r.ip);
                        row.dns_ms = Some(r.elapsed_ms);
                        row.status = RowStatus::Waiting;
                    }
                    Err(e) => row.status = RowStatus::Failed(e.clone()),
                }
            }
        }
        Event::Sample { ip, result } => {
            for row in rows.iter_mut().filter(|r| r.ip == Some(*ip)) {
                row.stats.samples.push(result.clone());
                row.status = if row.stats.samples.len() >= count as usize {
                    if row.stats.successes() == 0 {
                        RowStatus::Failed(row.stats.last_error().unwrap_or("所有测速均失败").into())
                    } else {
                        RowStatus::Complete
                    }
                } else {
                    RowStatus::Probing
                };
            }
        }
        Event::Cancelled => {
            for row in rows {
                if !matches!(row.status, RowStatus::Complete | RowStatus::Failed(_)) {
                    row.status = RowStatus::Cancelled;
                }
            }
        }
        Event::Finished => {
            for row in rows {
                if !matches!(row.status, RowStatus::Complete | RowStatus::Failed(_)) {
                    row.status = RowStatus::Failed("任务未完成".into());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn already_cancelled_run_returns_without_results() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        let (tx, rx) = std::sync::mpsc::channel();
        run(Settings::default(), tx, cancel).await;
        assert!(matches!(rx.recv().unwrap(), Event::Cancelled));
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn duplicate_ips_share_samples_and_cancellation_keeps_completed_rows() {
        let mut rows: Vec<_> = Settings::default()
            .servers
            .into_iter()
            .take(2)
            .map(ResultRow::new)
            .collect();
        let ip = "1.2.3.4".parse().unwrap();
        for row in &mut rows {
            row.ip = Some(ip);
        }
        apply_event(
            &mut rows,
            &Event::Sample {
                ip,
                result: Ok(12.),
            },
            1,
        );
        apply_event(&mut rows, &Event::Cancelled, 1);
        assert!(
            rows.iter()
                .all(|r| matches!(r.status, RowStatus::Complete) && r.stats.average() == Some(12.))
        );
    }
}
