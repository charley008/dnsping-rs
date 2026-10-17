use crate::model::QueryKind;
use anyhow::{Context, Result, bail, ensure};
use hickory_proto::{
    op::{Message, MessageType, Query, ResponseCode},
    rr::{Name, RData, RecordType},
};
use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
};

#[derive(Clone, Debug)]
pub struct Resolution {
    pub ip: IpAddr,
    pub elapsed_ms: f64,
}

pub async fn resolve(
    domain: &str,
    server: SocketAddr,
    kind: QueryKind,
    timeout: Duration,
) -> Result<Resolution> {
    let start = Instant::now();
    let ip = tokio::time::timeout(timeout, resolve_inner(domain, server, kind))
        .await
        .context("DNS 查询超时")??;
    Ok(Resolution {
        ip,
        elapsed_ms: start.elapsed().as_secs_f64() * 1000.,
    })
}

async fn exchange(query: &Message, server: SocketAddr) -> Result<Message> {
    let bytes = query.to_vec()?;
    let socket = UdpSocket::bind(if server.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    })
    .await?;
    socket
        .connect(server)
        .await
        .context("无法连接 DNS 服务器")?;
    socket.send(&bytes).await?;
    let mut buffer = vec![0; 65535];
    let message = loop {
        let len = socket.recv(&mut buffer).await?;
        let response = Message::from_vec(&buffer[..len]).context("DNS 响应格式错误")?;
        if response.metadata.id != query.metadata.id {
            continue;
        }
        validate_response(&response, query)?;
        break response;
    };
    if !message.metadata.truncation {
        return Ok(message);
    }
    let mut stream = TcpStream::connect(server)
        .await
        .context("DNS TCP 回退失败")?;
    stream.write_u16(bytes.len() as u16).await?;
    stream.write_all(&bytes).await?;
    let len = stream.read_u16().await? as usize;
    ensure!(len >= 12, "DNS TCP 响应长度无效");
    buffer.resize(len, 0);
    stream.read_exact(&mut buffer).await?;
    let response = Message::from_vec(&buffer)?;
    validate_response(&response, query)?;
    ensure!(!response.metadata.truncation, "DNS TCP 响应仍被截断");
    Ok(response)
}

fn validate_response(response: &Message, query: &Message) -> Result<()> {
    ensure!(
        response.metadata.id == query.metadata.id
            && response.metadata.message_type == MessageType::Response
            && response.metadata.op_code == query.metadata.op_code
            && response.queries == query.queries,
        "DNS 响应与请求不匹配"
    );
    Ok(())
}

async fn resolve_inner(domain: &str, server: SocketAddr, kind: QueryKind) -> Result<IpAddr> {
    let record_type = match kind {
        QueryKind::Ipv4 => RecordType::A,
        QueryKind::Ipv6 => RecordType::AAAA,
    };
    let mut name = Name::from_ascii(format!("{domain}."))?;
    let mut seen = HashSet::new();
    for _ in 0..16 {
        ensure!(seen.insert(name.clone()), "DNS CNAME 存在循环");
        let mut query = Message::query();
        query.metadata.recursion_desired = true;
        query.add_query(Query::query(name.clone(), record_type));
        let response = exchange(&query, server).await?;
        match response.metadata.response_code {
            ResponseCode::NoError => {}
            ResponseCode::NXDomain => bail!("域名不存在 (NXDOMAIN)"),
            code => bail!("DNS 返回 {code}"),
        }
        let original = name.clone();
        let mut chain = HashSet::new();
        // Inspect names in the answer chain, retaining wire order for matching addresses.
        for _ in 0..16 {
            ensure!(chain.insert(name.clone()), "DNS CNAME 存在循环");
            if let Some(ip) =
                response
                    .answers
                    .iter()
                    .filter(|r| r.name == name)
                    .find_map(|r| match (kind, &r.data) {
                        (QueryKind::Ipv4, RData::A(a)) => Some(IpAddr::V4(a.0)),
                        (QueryKind::Ipv6, RData::AAAA(a)) => Some(IpAddr::V6(a.0)),
                        _ => None,
                    })
            {
                return Ok(ip);
            }
            match response
                .answers
                .iter()
                .filter(|r| r.name == name)
                .find_map(|r| {
                    if let RData::CNAME(cname) = &r.data {
                        Some(cname.0.clone())
                    } else {
                        None
                    }
                }) {
                Some(next) => name = next,
                None => break,
            }
        }
        if name == original {
            bail!("没有返回 {record_type} 地址");
        }
    }
    bail!("DNS CNAME 链过长")
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::{
        op::OpCode,
        rr::{
            Record,
            rdata::{A, AAAA, CNAME},
        },
    };

    fn response(q: &Message) -> Message {
        let mut r = Message::response(q.metadata.id, OpCode::Query);
        r.queries = q.queries.clone();
        r
    }
    #[tokio::test]
    async fn first_address_in_cname_chain_and_not_unrelated_answer() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = socket.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0; 2048];
            let (n, peer) = socket.recv_from(&mut buf).await.unwrap();
            let q = Message::from_vec(&buf[..n]).unwrap();
            let mut r = response(&q);
            r.add_answer(Record::from_rdata(
                Name::from_ascii("unrelated.test.").unwrap(),
                10,
                RData::A(A("9.9.9.9".parse().unwrap())),
            ));
            r.add_answer(Record::from_rdata(
                q.queries[0].name().clone(),
                10,
                RData::CNAME(CNAME(Name::from_ascii("target.test.").unwrap())),
            ));
            for ip in ["1.2.3.4", "5.6.7.8"] {
                r.add_answer(Record::from_rdata(
                    Name::from_ascii("target.test.").unwrap(),
                    10,
                    RData::A(A(ip.parse().unwrap())),
                ));
            }
            socket.send_to(&r.to_vec().unwrap(), peer).await.unwrap();
        });
        let r = resolve(
            "example.test",
            addr,
            QueryKind::Ipv4,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(r.ip.to_string(), "1.2.3.4");
    }
    #[tokio::test]
    async fn truncated_udp_falls_back_to_tcp_and_resolves_ipv6() {
        let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = tcp.local_addr().unwrap();
        let udp = UdpSocket::bind(addr).await.unwrap();
        tokio::spawn(async move {
            let mut buf = [0; 2048];
            let (n, peer) = udp.recv_from(&mut buf).await.unwrap();
            let q = Message::from_vec(&buf[..n]).unwrap();
            let mut r = response(&q);
            r.metadata.truncation = true;
            udp.send_to(&r.to_vec().unwrap(), peer).await.unwrap();
            let (mut stream, _) = tcp.accept().await.unwrap();
            let n = stream.read_u16().await.unwrap() as usize;
            stream.read_exact(&mut buf[..n]).await.unwrap();
            let q = Message::from_vec(&buf[..n]).unwrap();
            let mut r = response(&q);
            r.add_answer(Record::from_rdata(
                q.queries[0].name().clone(),
                10,
                RData::AAAA(AAAA("::1".parse().unwrap())),
            ));
            let bytes = r.to_vec().unwrap();
            stream.write_u16(bytes.len() as u16).await.unwrap();
            stream.write_all(&bytes).await.unwrap();
        });
        assert_eq!(
            resolve("test.local", addr, QueryKind::Ipv6, Duration::from_secs(1))
                .await
                .unwrap()
                .ip
                .to_string(),
            "::1"
        );
    }
    #[tokio::test]
    async fn timeout_is_bounded() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let r = resolve(
            "test.local",
            socket.local_addr().unwrap(),
            QueryKind::Ipv4,
            Duration::from_millis(20),
        )
        .await;
        assert!(r.unwrap_err().to_string().contains("超时"));
    }
}
