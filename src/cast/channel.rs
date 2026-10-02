//! The Cast v2 wire format: TLS to port 8009 (devices use self-signed
//! certificates), each message a 4-byte big-endian length plus a
//! `CastMessage` protobuf whose payload is JSON. Only the string-payload
//! fields are encoded/decoded, by hand — no protobuf dependency.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf},
    net::TcpStream,
};
use tokio_rustls::{
    TlsConnector,
    client::TlsStream,
    rustls::{
        self, DigitallySignedStruct, SignatureScheme,
        client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
        pki_types::{CertificateDer, ServerName, UnixTime},
    },
};

pub const NS_CONNECTION: &str = "urn:x-cast:com.google.cast.tp.connection";
pub const NS_HEARTBEAT: &str = "urn:x-cast:com.google.cast.tp.heartbeat";
pub const NS_RECEIVER: &str = "urn:x-cast:com.google.cast.receiver";
pub const NS_MEDIA: &str = "urn:x-cast:com.google.cast.media";
pub const SENDER: &str = "sender-0";
pub const RECEIVER: &str = "receiver-0";
/// Largest message accepted from a device.
const MAX_MESSAGE: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub source: String,
    pub destination: String,
    pub namespace: String,
    pub payload: String,
}

impl Message {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.payload).unwrap_or(Value::Null)
    }

    pub fn kind(&self) -> String {
        self.json()["type"].as_str().unwrap_or("").to_owned()
    }
}

pub type Reader = ReadHalf<TlsStream<TcpStream>>;
pub type Writer = WriteHalf<TlsStream<TcpStream>>;

pub async fn connect(addr: SocketAddr) -> Result<(Reader, Writer)> {
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(AnyCertificate))
    .with_no_client_auth();
    let tcp = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(addr))
        .await
        .context("timed out connecting")??;
    let name = ServerName::IpAddress(addr.ip().into());
    let tls = TlsConnector::from(Arc::new(config))
        .connect(name, tcp)
        .await
        .context("TLS handshake")?;
    Ok(tokio::io::split(tls))
}

pub async fn send(
    w: &mut Writer,
    destination: &str,
    namespace: &str,
    payload: &Value,
) -> Result<()> {
    let body = encode(SENDER, destination, namespace, &payload.to_string());
    let mut frame = (body.len() as u32).to_be_bytes().to_vec();
    frame.extend_from_slice(&body);
    w.write_all(&frame).await?;
    w.flush().await?;
    Ok(())
}

pub async fn receive(r: &mut Reader) -> Result<Message> {
    let len = r.read_u32().await? as usize;
    if len > MAX_MESSAGE {
        bail!("message too large ({len} bytes)");
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await?;
    decode(&body)
}

fn varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn string_field(out: &mut Vec<u8>, field: u8, s: &str) {
    out.push((field << 3) | 2);
    varint(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

fn encode(source: &str, destination: &str, namespace: &str, payload: &str) -> Vec<u8> {
    let mut out = vec![0x08, 0x00]; // protocol_version = CASTV2_1_0
    string_field(&mut out, 2, source);
    string_field(&mut out, 3, destination);
    string_field(&mut out, 4, namespace);
    out.extend_from_slice(&[0x28, 0x00]); // payload_type = STRING
    string_field(&mut out, 6, payload);
    out
}

fn decode(mut b: &[u8]) -> Result<Message> {
    fn read_varint(b: &mut &[u8]) -> Result<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let (&byte, rest) = b.split_first().context("truncated varint")?;
            *b = rest;
            v |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(v);
            }
        }
        bail!("varint too long")
    }
    let mut m = Message {
        source: String::new(),
        destination: String::new(),
        namespace: String::new(),
        payload: String::new(),
    };
    while !b.is_empty() {
        let key = read_varint(&mut b)?;
        let (field, wire) = (key >> 3, key & 7);
        match wire {
            0 => {
                read_varint(&mut b)?;
            }
            2 => {
                let len = read_varint(&mut b)? as usize;
                anyhow::ensure!(len <= b.len(), "truncated field");
                let (value, rest) = b.split_at(len);
                b = rest;
                let text = || String::from_utf8_lossy(value).into_owned();
                match field {
                    2 => m.source = text(),
                    3 => m.destination = text(),
                    4 => m.namespace = text(),
                    6 => m.payload = text(),
                    _ => {}
                }
            }
            1 => b = b.get(8..).context("truncated")?,
            5 => b = b.get(4..).context("truncated")?,
            _ => bail!("unknown wire type {wire}"),
        }
    }
    Ok(m)
}

/// Cast devices present self-signed certificates; the connection is to an
/// address the user picked on their own network.
#[derive(Debug)]
struct AnyCertificate;

impl ServerCertVerifier for AnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
pub fn tests_encode(source: &str, destination: &str, namespace: &str, payload: &str) -> Vec<u8> {
    encode(source, destination, namespace, payload)
}

#[cfg(test)]
pub fn tests_decode(body: &[u8]) -> Message {
    decode(body).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip() {
        let payload = "x".repeat(300); // multi-byte length varint
        let body = encode(SENDER, RECEIVER, NS_RECEIVER, &payload);
        let m = decode(&body).unwrap();
        assert_eq!(m.source, SENDER);
        assert_eq!(m.destination, RECEIVER);
        assert_eq!(m.namespace, NS_RECEIVER);
        assert_eq!(m.payload, payload);
    }
}
