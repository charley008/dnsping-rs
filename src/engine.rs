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
        samples: Vec<Result<f64, String>>,
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
    run_with_resolver(
        settings,
        sender,
        |domain, server, kind, timeout| async move {
            dns::resolve(&domain, SocketAddr::new(server, 53), kind, timeout).await
        },
    )
    .await;
}

async fn run_with_resolver<F, Fut>(settings: Settings, sender: &Sender<Event>, resolve: F)
where
    F: Fn(String, IpAddr, QueryKind, Duration) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<dns::Resolution>> + Send + 'static,
{
    let timeout = Duration::from_millis(settings.timeout_ms);
    enum Work {
        Resolved(usize, Result<dns::Resolution, String>),
        Sample(IpAddr, Result<f64, String>, u32),
    }
    let mut work = JoinSet::new();
    for (row, server) in settings.servers.iter().filter(|s| s.enabled).enumerate() {
        let domain = settings.domain.clone();
        let server = server.address;
        let kind = settings.query;
        let query = resolve(domain, server, kind, timeout);
        work.spawn(async move { Work::Resolved(row, query.await.map_err(|e| format!("{e:#}"))) });
    }
    let mut samples: BTreeMap<IpAddr, Vec<Result<f64, String>>> = BTreeMap::new();
    let spawn_sample = |work: &mut JoinSet<Work>, ip, index| {
        let kind = settings.probe;
        let port = settings.port;
        work.spawn(async move {
            if index > 0 {
                tokio::time::sleep(Duration::from_millis(180)).await;
            }
            Work::Sample(
                ip,
                probe::sample(ip, kind, port, timeout)
                    .await
                    .map_err(|e| format!("{e:#}")),
                index,
            )
        });
    };
    while let Some(result) = work.join_next().await {
        match result {
            Ok(Work::Resolved(row, result)) => {
                let previous = result
                    .as_ref()
                    .ok()
                    .map(|r| {
                        if let std::collections::btree_map::Entry::Vacant(entry) =
                            samples.entry(r.ip)
                        {
                            entry.insert(Vec::new());
                            spawn_sample(&mut work, r.ip, 0);
                        }
                        samples[&r.ip].clone()
                    })
                    .unwrap_or_default();
                if sender
                    .send(Event::Resolved {
                        row,
                        result,
                        samples: previous,
                    })
                    .is_err()
                {
                    return;
                }
            }
            Ok(Work::Sample(ip, result, index)) => {
                samples.get_mut(&ip).unwrap().push(result.clone());
                if sender.send(Event::Sample { ip, result }).is_err() {
                    return;
                }
                if index + 1 < settings.samples {
                    spawn_sample(&mut work, ip, index + 1);
                }
            }
            Err(_) => {}
        }
    }
}

fn sample_status(row: &ResultRow, count: u32) -> RowStatus {
    if row.stats.samples.len() >= count as usize {
        if row.stats.successes() == 0 {
            RowStatus::Failed(row.stats.last_error().unwrap_or("所有测速均失败").into())
        } else {
            RowStatus::Complete
        }
    } else if row.stats.samples.is_empty() {
        RowStatus::Waiting
    } else {
        RowStatus::Probing
    }
}

pub fn apply_event(rows: &mut [ResultRow], event: &Event, count: u32) {
    match event {
        Event::Resolved {
            row,
            result,
            samples,
        } => {
            if let Some(row) = rows.get_mut(*row) {
                match result {
                    Ok(r) => {
                        row.ip = Some(r.ip);
                        row.dns_ms = Some(r.elapsed_ms);
                        row.stats.samples = samples.clone();
                        row.status = sample_status(row, count);
                    }
                    Err(e) => row.status = RowStatus::Failed(e.clone()),
                }
            }
        }
        Event::Sample { ip, result } => {
            for row in rows.iter_mut().filter(|r| r.ip == Some(*ip)) {
                row.stats.samples.push(result.clone());
                row.status = sample_status(row, count);
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
    async fn probes_start_before_slow_dns_and_late_duplicate_gets_all_samples() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut settings = Settings {
            samples: 2,
            probe: ProbeKind::Tcp,
            port: listener.local_addr().unwrap().port(),
            ..Default::default()
        };
        settings.servers.truncate(2);
        let slow = settings.servers[1].address;
        let (tx, rx) = std::sync::mpsc::channel();
        tokio::time::timeout(
            Duration::from_secs(3),
            run_with_resolver(settings.clone(), &tx, move |_, server, _, _| async move {
                if server == slow {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                Ok(dns::Resolution {
                    ip: "127.0.0.1".parse().unwrap(),
                    elapsed_ms: 1.,
                })
            }),
        )
        .await
        .unwrap();
        let events: Vec<_> = rx.try_iter().collect();
        let first_sample = events
            .iter()
            .position(|event| matches!(event, Event::Sample { .. }))
            .unwrap();
        let late_dns = events
            .iter()
            .position(|event| matches!(event, Event::Resolved { row: 1, .. }))
            .unwrap();
        assert!(first_sample < late_dns);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::Sample { .. }))
                .count(),
            2
        );
        let mut rows: Vec<_> = settings.servers.into_iter().map(ResultRow::new).collect();
        for event in events {
            apply_event(&mut rows, &event, 2);
        }
        assert!(rows.iter().all(|row| row.stats.samples.len() == 2
            && row.stats.successes() == 2
            && matches!(row.status, RowStatus::Complete)));
    }
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
