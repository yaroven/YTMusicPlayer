//! Finds Cast devices on the LAN: one mDNS question for
//! `_googlecast._tcp.local`, sent from an ephemeral port so devices answer
//! by unicast (RFC 6762 §6.7) — no multicast group to join, no daemon.

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};

use anyhow::Result;
use tokio::net::UdpSocket;

use super::Device;

const SERVICE: &str = "_googlecast._tcp.local";
const MDNS: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(224, 0, 0, 251)), 5353);

const TYPE_A: u16 = 1;
const TYPE_PTR: u16 = 12;
const TYPE_TXT: u16 = 16;
const TYPE_SRV: u16 = 33;

/// Asks twice (UDP may drop), collects answers for `wait`.
pub async fn discover(wait: Duration) -> Result<Vec<Device>> {
    browse(SERVICE, wait).await
}

async fn browse(service: &str, wait: Duration) -> Result<Vec<Device>> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
    let query = query(service);
    socket.send_to(&query, MDNS).await?;
    let mut records = Records {
        service: service.to_owned(),
        ..Records::default()
    };
    let mut buf = vec![0u8; 9000];
    let deadline = tokio::time::Instant::now() + wait;
    let mut asked_again = false;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            break;
        }
        if !asked_again && left < wait / 2 {
            asked_again = true;
            let _ = socket.send_to(&query, MDNS).await;
        }
        match tokio::time::timeout(left.min(wait / 2), socket.recv_from(&mut buf)).await {
            Ok(Ok((n, from))) => {
                if let Err(err) = records.parse(&buf[..n], from.ip()) {
                    tracing::debug!(%err, %from, "mDNS answer");
                }
            }
            Ok(Err(err)) => return Err(err.into()),
            Err(_) => {} // timeout slice
        }
    }
    Ok(records.devices())
}

/// A PTR question with the "unicast response" bit.
fn query(service: &str) -> Vec<u8> {
    let mut q = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in service.split('.') {
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.push(0);
    q.extend_from_slice(&TYPE_PTR.to_be_bytes());
    q.extend_from_slice(&0x8001u16.to_be_bytes());
    q
}

#[derive(Default)]
struct Records {
    service: String,
    /// Service instances (from PTR answers).
    instances: Vec<String>,
    /// Instance -> (port, host).
    srv: HashMap<String, (u16, String)>,
    /// Instance -> friendly name (TXT `fn=`).
    names: HashMap<String, String>,
    /// Host -> address.
    hosts: HashMap<String, Ipv4Addr>,
    /// Instance -> address the answer came from (when no A record).
    senders: HashMap<String, IpAddr>,
}

impl Records {
    fn parse(&mut self, msg: &[u8], from: IpAddr) -> Result<()> {
        let mut r = Reader { msg, pos: 12 };
        let count = |i: usize| u16::from_be_bytes([msg[i], msg[i + 1]]) as usize;
        anyhow::ensure!(msg.len() >= 12, "short packet");
        let (questions, records) = (count(4), count(6) + count(8) + count(10));
        for _ in 0..questions {
            r.name()?;
            r.skip(4)?;
        }
        for _ in 0..records {
            let name = r.name()?;
            let kind = r.u16()?;
            r.skip(6)?; // class, TTL
            let len = r.u16()? as usize;
            let end = r.pos + len;
            anyhow::ensure!(end <= msg.len(), "record past the end");
            match kind {
                TYPE_PTR if name.eq_ignore_ascii_case(&self.service) => {
                    let instance = r.name()?;
                    self.senders.insert(instance.clone(), from);
                    if !self.instances.contains(&instance) {
                        self.instances.push(instance);
                    }
                }
                TYPE_SRV => {
                    r.skip(4)?; // priority, weight
                    let port = r.u16()?;
                    let host = r.name()?;
                    self.srv.insert(name, (port, host));
                }
                TYPE_TXT => {
                    let mut at = r.pos;
                    while at < end {
                        let n = msg[at] as usize;
                        let entry = msg.get(at + 1..at + 1 + n).unwrap_or_default();
                        if let Some(v) = entry.strip_prefix(b"fn=") {
                            self.names
                                .insert(name.clone(), String::from_utf8_lossy(v).into_owned());
                        }
                        at += 1 + n;
                    }
                }
                TYPE_A if len == 4 => {
                    let b = &msg[r.pos..end];
                    self.hosts
                        .insert(name, Ipv4Addr::new(b[0], b[1], b[2], b[3]));
                }
                _ => {}
            }
            r.pos = end;
        }
        Ok(())
    }

    fn devices(&self) -> Vec<Device> {
        let mut devices: Vec<Device> = Vec::new();
        for instance in &self.instances {
            let (port, host) = self
                .srv
                .get(instance)
                .cloned()
                .unwrap_or((8009, String::new()));
            let ip = match self.hosts.get(&host) {
                Some(ip) => IpAddr::V4(*ip),
                None => match self.senders.get(instance) {
                    Some(ip) => *ip,
                    None => continue,
                },
            };
            let name = self
                .names
                .get(instance)
                .cloned()
                .unwrap_or_else(|| instance.split('.').next().unwrap_or(instance).to_owned());
            let addr = SocketAddr::new(ip, port);
            if !devices.iter().any(|d| d.addr == addr) {
                devices.push(Device { name, addr });
            }
        }
        devices.sort_by(|a, b| a.name.cmp(&b.name));
        devices
    }
}

struct Reader<'a> {
    msg: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn skip(&mut self, n: usize) -> Result<()> {
        anyhow::ensure!(self.pos + n <= self.msg.len(), "truncated");
        self.pos += n;
        Ok(())
    }

    fn u16(&mut self) -> Result<u16> {
        anyhow::ensure!(self.pos + 2 <= self.msg.len(), "truncated");
        let v = u16::from_be_bytes([self.msg[self.pos], self.msg[self.pos + 1]]);
        self.pos += 2;
        Ok(v)
    }

    /// A domain name, following compression pointers.
    fn name(&mut self) -> Result<String> {
        let mut labels: Vec<String> = Vec::new();
        let mut at = self.pos;
        let mut jumped = false;
        for _ in 0..64 {
            let len = *self
                .msg
                .get(at)
                .ok_or_else(|| anyhow::anyhow!("truncated name"))? as usize;
            if len == 0 {
                if !jumped {
                    self.pos = at + 1;
                }
                return Ok(labels.join("."));
            }
            if len & 0xc0 == 0xc0 {
                let low = *self
                    .msg
                    .get(at + 1)
                    .ok_or_else(|| anyhow::anyhow!("truncated"))?;
                if !jumped {
                    self.pos = at + 2;
                }
                jumped = true;
                at = ((len & 0x3f) << 8) | low as usize;
                continue;
            }
            let label = self
                .msg
                .get(at + 1..at + 1 + len)
                .ok_or_else(|| anyhow::anyhow!("truncated label"))?;
            labels.push(String::from_utf8_lossy(label).into_owned());
            at += 1 + len;
        }
        anyhow::bail!("name too long or looping")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(out: &mut Vec<u8>, name: &str) {
        for label in name.split('.') {
            out.push(label.len() as u8);
            out.extend_from_slice(label.as_bytes());
        }
        out.push(0);
    }

    fn record(out: &mut Vec<u8>, owner: &str, kind: u16, data: &[u8]) {
        name(out, owner);
        out.extend_from_slice(&kind.to_be_bytes());
        out.extend_from_slice(&[0x80, 1, 0, 0, 0, 120]);
        out.extend_from_slice(&(data.len() as u16).to_be_bytes());
        out.extend_from_slice(data);
    }

    #[test]
    fn answer_with_srv_txt_and_a_records_becomes_a_device() {
        let instance = "Chromecast-abc._googlecast._tcp.local";
        let mut msg = vec![0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 3];
        let mut ptr = Vec::new();
        name(&mut ptr, instance);
        record(&mut msg, SERVICE, TYPE_PTR, &ptr);
        let mut txt = Vec::new();
        for entry in ["id=abc", "fn=Living Room"] {
            txt.push(entry.len() as u8);
            txt.extend_from_slice(entry.as_bytes());
        }
        record(&mut msg, instance, TYPE_TXT, &txt);
        let mut srv = vec![0, 0, 0, 0, 0x1f, 0x49];
        name(&mut srv, "abc.local");
        record(&mut msg, instance, TYPE_SRV, &srv);
        record(&mut msg, "abc.local", TYPE_A, &[192, 168, 1, 20]);

        let mut records = Records {
            service: SERVICE.into(),
            ..Records::default()
        };
        records.parse(&msg, "10.0.0.1".parse().unwrap()).unwrap();
        let devices = records.devices();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "Living Room");
        assert_eq!(devices[0].addr, "192.168.1.20:8009".parse().unwrap());
    }

    #[test]
    fn query_asks_for_unicast_ptr() {
        let q = query(SERVICE);
        assert_eq!(&q[q.len() - 4..], &[0, 12, 0x80, 1]);
    }

    /// Needs a LAN with AirPlay devices (the parser is the same).
    #[tokio::test]
    #[ignore]
    async fn finds_airplay_devices_live() {
        let devices = browse("_airplay._tcp.local", Duration::from_secs(3))
            .await
            .unwrap();
        println!("{devices:?}");
        assert!(!devices.is_empty());
    }
}
