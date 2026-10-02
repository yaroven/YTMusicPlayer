//! Chromecast output: [`CastPlayer`] is a [`Playback`] adapter that hands
//! the stream URL to a Cast device's Default Media Receiver, which fetches
//! and plays it itself. The session swaps it in for the local player.
//!
//! One task owns the TLS connection: commands come in over a channel,
//! media status updates the shared [`PlayerStatus`] (position interpolated
//! between updates), and the end of a track starts the preloaded one.

mod channel;
mod discover;

use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

pub use discover::discover;

use crate::audio::player::{Load, PlayState, Playback, PlayerEvent, PlayerStatus};
use channel::{NS_CONNECTION, NS_HEARTBEAT, NS_MEDIA, NS_RECEIVER, RECEIVER, Reader, Writer};

/// Google's Default Media Receiver.
const APP_ID: &str = "CC1AD845";
const HEARTBEAT: Duration = Duration::from_secs(5);
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(15);

/// A Cast device found on the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    pub addr: SocketAddr,
}

enum Command {
    Load(Load),
    Preload(Load),
    CancelPreload,
    SetPaused(Option<bool>),
    Stop,
    SeekTo(Duration),
    SetVolume(f32),
}

/// Last reported state, for interpolating the position.
struct Shared {
    state: PlayState,
    position: Duration,
    at: Instant,
    duration: Option<Duration>,
    volume: f32,
}

impl Shared {
    fn position(&self) -> Duration {
        match self.state {
            PlayState::Playing => self.position + self.at.elapsed(),
            _ => self.position,
        }
    }

    fn set_position(&mut self, position: Duration) {
        self.position = position;
        self.at = Instant::now();
    }
}

pub struct CastPlayer {
    tx: UnboundedSender<Command>,
    shared: Arc<Mutex<Shared>>,
    alive: Arc<AtomicBool>,
    pub device: Device,
}

impl CastPlayer {
    /// Connects, starts the media receiver app and returns the player.
    /// Its events go to `events`.
    pub async fn connect(
        device: Device,
        volume: f32,
        events: UnboundedSender<PlayerEvent>,
    ) -> Result<Self> {
        let (mut reader, mut writer) = channel::connect(device.addr).await?;
        channel::send(
            &mut writer,
            RECEIVER,
            NS_CONNECTION,
            &json!({"type": "CONNECT"}),
        )
        .await?;
        channel::send(
            &mut writer,
            RECEIVER,
            NS_RECEIVER,
            &json!({"type": "LAUNCH", "appId": APP_ID, "requestId": 1}),
        )
        .await?;
        let (transport, session) =
            tokio::time::timeout(LAUNCH_TIMEOUT, wait_for_app(&mut reader, &mut writer))
                .await
                .context("the device didn't start the media receiver")??;
        channel::send(
            &mut writer,
            &transport,
            NS_CONNECTION,
            &json!({"type": "CONNECT"}),
        )
        .await?;

        let shared = Arc::new(Mutex::new(Shared {
            state: PlayState::Idle,
            position: Duration::ZERO,
            at: Instant::now(),
            duration: None,
            volume,
        }));
        let alive = Arc::new(AtomicBool::new(true));
        let (tx, rx) = unbounded_channel();
        let link = Link {
            reader,
            writer,
            transport,
            session,
            shared: shared.clone(),
            events,
            request: 10,
            media_session: None,
            load_request: None,
            current: None,
            next: None,
            seek_on_load: None,
        };
        let flag = alive.clone();
        tokio::spawn(async move {
            if let Err(err) = link.run(rx).await {
                tracing::warn!("cast: {err:#}");
            }
            flag.store(false, Ordering::Relaxed);
        });
        let player = Self {
            tx,
            shared,
            alive,
            device,
        };
        player.set_volume(volume);
        Ok(player)
    }

    /// Turns false once the connection drops (device off, app closed).
    pub fn alive(&self) -> Arc<AtomicBool> {
        self.alive.clone()
    }

    fn shared(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }
}

async fn wait_for_app(reader: &mut Reader, writer: &mut Writer) -> Result<(String, String)> {
    loop {
        let m = channel::receive(reader).await?;
        let v = m.json();
        match (m.namespace.as_str(), v["type"].as_str()) {
            (NS_HEARTBEAT, Some("PING")) => {
                channel::send(writer, &m.source, NS_HEARTBEAT, &json!({"type": "PONG"})).await?;
            }
            (NS_RECEIVER, Some("RECEIVER_STATUS")) => {
                let apps = v["status"]["applications"].as_array();
                if let Some(app) = apps
                    .into_iter()
                    .flatten()
                    .find(|a| a["appId"] == APP_ID && a["transportId"].is_string())
                {
                    return Ok((
                        app["transportId"].as_str().unwrap_or_default().to_owned(),
                        app["sessionId"].as_str().unwrap_or_default().to_owned(),
                    ));
                }
            }
            (NS_RECEIVER, Some("LAUNCH_ERROR")) => bail!("launch refused: {}", m.payload),
            _ => {}
        }
    }
}

impl Playback for CastPlayer {
    fn load(&self, track: Load) {
        {
            let mut s = self.shared();
            s.state = PlayState::Playing;
            s.duration = track.duration;
            s.set_position(Duration::ZERO);
        }
        let _ = self.tx.send(Command::Load(track));
    }
    fn preload(&self, track: Load) {
        let _ = self.tx.send(Command::Preload(track));
    }
    fn cancel_preload(&self) {
        let _ = self.tx.send(Command::CancelPreload);
    }
    // The receiver plays one item at a time: no overlap.
    fn set_crossfade(&self, _overlap: Duration) {}
    fn toggle_pause(&self) {
        let _ = self.tx.send(Command::SetPaused(None));
    }
    fn set_paused(&self, paused: bool) {
        let _ = self.tx.send(Command::SetPaused(Some(paused)));
    }
    fn stop(&self) {
        self.shared().state = PlayState::Idle;
        let _ = self.tx.send(Command::Stop);
    }
    fn seek_by(&self, secs: i64) {
        let now = self.shared().position();
        let target = if secs < 0 {
            now.saturating_sub(Duration::from_secs(secs.unsigned_abs()))
        } else {
            now + Duration::from_secs(secs as u64)
        };
        self.seek_to(target);
    }
    fn seek_to(&self, position: Duration) {
        self.shared().set_position(position);
        let _ = self.tx.send(Command::SeekTo(position));
    }
    fn set_volume(&self, volume: f32) {
        let volume = volume.clamp(0.0, 1.0);
        self.shared().volume = volume;
        let _ = self.tx.send(Command::SetVolume(volume));
    }
    fn status(&self) -> PlayerStatus {
        let s = self.shared();
        PlayerStatus {
            state: s.state,
            position: s.position(),
            duration: s.duration,
            volume: s.volume,
        }
    }
}

/// The connection task's state.
struct Link {
    reader: Reader,
    writer: Writer,
    /// The receiver app's transport id and session id.
    transport: String,
    session: String,
    shared: Arc<Mutex<Shared>>,
    events: UnboundedSender<PlayerEvent>,
    request: u64,
    media_session: Option<i64>,
    /// The LOAD whose answer names the new media session.
    load_request: Option<u64>,
    /// Generation of the loaded track.
    current: Option<u64>,
    next: Option<Load>,
    /// A seek asked for before the media session exists.
    seek_on_load: Option<Duration>,
}

impl Link {
    async fn run(mut self, mut commands: UnboundedReceiver<Command>) -> Result<()> {
        let mut heartbeat = tokio::time::interval(HEARTBEAT);
        let result = loop {
            tokio::select! {
                cmd = commands.recv() => match cmd {
                    Some(cmd) => {
                        if let Err(err) = self.command(cmd).await {
                            break Err(err);
                        }
                    }
                    // The session switched away: close the receiver app.
                    None => {
                        let session = self.session.clone();
                        let _ = self.receiver(json!({"type": "STOP", "sessionId": session})).await;
                        return Ok(());
                    }
                },
                m = channel::receive(&mut self.reader) => match m {
                    Ok(m) => {
                        if let Err(err) = self.message(m).await {
                            break Err(err);
                        }
                    }
                    Err(err) => break Err(err.context("connection lost")),
                },
                _ = heartbeat.tick() => {
                    if let Err(err) = channel::send(&mut self.writer, RECEIVER, NS_HEARTBEAT, &json!({"type": "PING"})).await {
                        break Err(err);
                    }
                }
            }
        };
        // Tell the session, so it can fall back to local playback.
        self.lock().state = PlayState::Idle;
        if let (Err(err), Some(generation)) = (&result, self.current) {
            let _ = self.events.send(PlayerEvent::Error {
                generation,
                message: format!("Chromecast: {err:#}"),
            });
        }
        result
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn next_request(&mut self) -> u64 {
        self.request += 1;
        self.request
    }

    async fn media(&mut self, mut payload: Value) -> Result<u64> {
        let id = self.next_request();
        payload["requestId"] = json!(id);
        channel::send(&mut self.writer, &self.transport, NS_MEDIA, &payload).await?;
        Ok(id)
    }

    async fn receiver(&mut self, mut payload: Value) -> Result<()> {
        payload["requestId"] = json!(self.next_request());
        channel::send(&mut self.writer, RECEIVER, NS_RECEIVER, &payload).await
    }

    async fn start(&mut self, load: Load) -> Result<()> {
        let generation = load.generation;
        self.current = Some(generation);
        self.media_session = None;
        self.lock().duration = load.duration;
        let Some(url) = load.url else {
            self.load_request = None;
            let _ = self.events.send(PlayerEvent::Error {
                generation,
                message: "this song can't be cast (no stream URL)".into(),
            });
            return Ok(());
        };
        let mut metadata = json!({"metadataType": 3});
        if let Some(t) = &load.track {
            metadata["title"] = json!(&*t.title);
            metadata["artist"] = json!(&*t.artist);
            metadata["images"] = json!([{
                "url": format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", t.video_id)
            }]);
        }
        let start = self.seek_on_load.take().unwrap_or_default();
        let mut media = json!({
            "contentId": url,
            "streamType": "BUFFERED",
            "contentType": "audio/mp4",
            "metadata": metadata,
        });
        if let Some(d) = load.duration {
            media["duration"] = json!(d.as_secs_f64());
        }
        let session = self.session.clone();
        let id = self
            .media(json!({
                "type": "LOAD",
                "sessionId": session,
                "autoplay": true,
                "currentTime": start.as_secs_f64(),
                "media": media,
            }))
            .await?;
        self.load_request = Some(id);
        Ok(())
    }

    async fn command(&mut self, cmd: Command) -> Result<()> {
        match cmd {
            Command::Load(load) => {
                self.next = None;
                self.start(load).await?;
            }
            Command::Preload(load) => self.next = Some(load),
            Command::CancelPreload => self.next = None,
            Command::SetPaused(paused) => {
                let Some(media) = self.media_session else {
                    return Ok(());
                };
                let pause = paused.unwrap_or(self.lock().state == PlayState::Playing);
                {
                    let mut s = self.lock();
                    let position = s.position();
                    s.set_position(position);
                    s.state = if pause {
                        PlayState::Paused
                    } else {
                        PlayState::Playing
                    };
                }
                let kind = if pause { "PAUSE" } else { "PLAY" };
                self.media(json!({"type": kind, "mediaSessionId": media}))
                    .await?;
            }
            Command::Stop => {
                self.current = None;
                self.next = None;
                if let Some(media) = self.media_session.take() {
                    self.media(json!({"type": "STOP", "mediaSessionId": media}))
                        .await?;
                }
            }
            Command::SeekTo(position) => match self.media_session {
                Some(media) => {
                    self.media(json!({
                        "type": "SEEK",
                        "mediaSessionId": media,
                        "currentTime": position.as_secs_f64(),
                    }))
                    .await?;
                }
                None => self.seek_on_load = Some(position),
            },
            Command::SetVolume(level) => {
                self.receiver(json!({"type": "SET_VOLUME", "volume": {"level": level}}))
                    .await?;
            }
        }
        Ok(())
    }

    async fn message(&mut self, m: channel::Message) -> Result<()> {
        let v = m.json();
        match (m.namespace.as_str(), v["type"].as_str().unwrap_or("")) {
            (NS_HEARTBEAT, "PING") => {
                channel::send(
                    &mut self.writer,
                    &m.source,
                    NS_HEARTBEAT,
                    &json!({"type": "PONG"}),
                )
                .await?;
            }
            (NS_CONNECTION, "CLOSE") if m.source == self.transport => {
                bail!("the receiver app was closed");
            }
            (NS_RECEIVER, "RECEIVER_STATUS") => {
                let running = v["status"]["applications"]
                    .as_array()
                    .is_some_and(|apps| apps.iter().any(|a| a["sessionId"] == *self.session));
                if !running {
                    bail!("the receiver app stopped");
                }
            }
            (NS_MEDIA, "MEDIA_STATUS") => self.media_status(&v).await?,
            (NS_MEDIA, "LOAD_FAILED" | "LOAD_CANCELLED" | "INVALID_REQUEST")
                if v["requestId"].as_u64() == self.load_request =>
            {
                if let Some(generation) = self.current {
                    let _ = self.events.send(PlayerEvent::Error {
                        generation,
                        message: format!("Chromecast couldn't play it ({})", m.kind()),
                    });
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn media_status(&mut self, v: &Value) -> Result<()> {
        let Some(status) = v["status"].as_array().and_then(|s| s.first()) else {
            return Ok(());
        };
        let id = status["mediaSessionId"].as_i64();
        if self.load_request.is_some() && v["requestId"].as_u64() == self.load_request {
            self.media_session = id;
            self.load_request = None;
            if let Some(position) = self.seek_on_load.take() {
                self.command(Command::SeekTo(position)).await?;
            }
        }
        if id.is_none() || id != self.media_session {
            return Ok(()); // an earlier item
        }
        let state = status["playerState"].as_str().unwrap_or("");
        {
            let mut s = self.lock();
            if let Some(t) = status["currentTime"].as_f64() {
                s.set_position(Duration::from_secs_f64(t.max(0.0)));
            }
            if let Some(d) = status["media"]["duration"].as_f64() {
                s.duration = Some(Duration::from_secs_f64(d.max(0.0)));
            }
            s.state = match state {
                "PLAYING" | "BUFFERING" => PlayState::Playing,
                "PAUSED" => PlayState::Paused,
                "IDLE" => PlayState::Idle,
                _ => s.state,
            };
        }
        if state != "IDLE" {
            return Ok(());
        }
        let Some(generation) = self.current else {
            return Ok(());
        };
        match status["idleReason"].as_str() {
            Some("FINISHED") => {
                self.media_session = None;
                match self.next.take() {
                    Some(next) => {
                        let started = next.generation;
                        self.start(next).await?;
                        {
                            let mut s = self.lock();
                            s.state = PlayState::Playing;
                            s.set_position(Duration::ZERO);
                        }
                        let _ = self.events.send(PlayerEvent::Started {
                            generation: started,
                        });
                    }
                    None => {
                        self.current = None;
                        let _ = self.events.send(PlayerEvent::Ended { generation });
                    }
                }
            }
            Some("ERROR") => {
                self.media_session = None;
                let _ = self.events.send(PlayerEvent::Error {
                    generation,
                    message: "Chromecast couldn't play the stream".into(),
                });
            }
            _ => {} // CANCELLED / INTERRUPTED: replaced by us
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::{io::AsyncWriteExt, net::TcpListener};
    use tokio_rustls::{
        TlsAcceptor,
        rustls::{
            self,
            pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
        },
    };

    use super::*;
    use crate::api::models::Track;

    /// A receiver that launches the app, accepts LOADs and reports each
    /// item finished right after it starts playing.
    async fn fake_receiver(listener: TcpListener, loads: UnboundedSender<Value>) {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(cert.cert.der().to_vec())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der())),
        )
        .unwrap();
        let (tcp, _) = listener.accept().await.unwrap();
        let tls = TlsAcceptor::from(Arc::new(config))
            .accept(tcp)
            .await
            .unwrap();
        let (mut r, mut w) = tokio::io::split(tls);
        let mut session = 0;
        loop {
            let Ok(len) = tokio::io::AsyncReadExt::read_u32(&mut r).await else {
                return;
            };
            let mut body = vec![0; len as usize];
            tokio::io::AsyncReadExt::read_exact(&mut r, &mut body)
                .await
                .unwrap();
            let m = channel::tests_decode(&body);
            let v = m.json();
            let reply = |ns: &str, src: &str, payload: Value| {
                let body = channel::tests_encode(src, "sender-0", ns, &payload.to_string());
                let mut frame = (body.len() as u32).to_be_bytes().to_vec();
                frame.extend_from_slice(&body);
                frame
            };
            let id = v["requestId"].clone();
            let out = match v["type"].as_str().unwrap_or("") {
                "LAUNCH" => vec![reply(
                    NS_RECEIVER,
                    RECEIVER,
                    json!({"type": "RECEIVER_STATUS", "requestId": id, "status": {"applications": [
                        {"appId": APP_ID, "transportId": "web-1", "sessionId": "s-1"}
                    ]}}),
                )],
                "LOAD" => {
                    session += 1;
                    loads.send(v.clone()).unwrap();
                    let playing = reply(
                        NS_MEDIA,
                        "web-1",
                        json!({"type": "MEDIA_STATUS", "requestId": id, "status": [
                            {"mediaSessionId": session, "playerState": "PLAYING", "currentTime": 0.0}
                        ]}),
                    );
                    w.write_all(&playing).await.unwrap();
                    // Let the sender queue its next track first.
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    vec![reply(
                        NS_MEDIA,
                        "web-1",
                        json!({"type": "MEDIA_STATUS", "requestId": 0, "status": [
                            {"mediaSessionId": session, "playerState": "IDLE", "idleReason": "FINISHED"}
                        ]}),
                    )]
                }
                _ => vec![],
            };
            for frame in out {
                w.write_all(&frame).await.unwrap();
            }
        }
    }

    fn load(generation: u64) -> Load {
        Load {
            media: None,
            url: Some(format!("https://example.com/{generation}.m4a")),
            track: Some(Track {
                video_id: "abcdefghijk".into(),
                title: "Song".into(),
                artist: "Band".into(),
                duration_secs: Some(100),
            }),
            duration: Some(Duration::from_secs(100)),
            gain: 1.0,
            generation,
        }
    }

    #[tokio::test]
    async fn plays_the_preload_when_the_device_finishes_a_track() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (loads_tx, mut loads) = unbounded_channel();
        tokio::spawn(fake_receiver(listener, loads_tx));

        let (events_tx, mut events) = unbounded_channel();
        let device = Device {
            name: "Fake".into(),
            addr,
        };
        let wait = Duration::from_secs(5);
        let player = tokio::time::timeout(wait, CastPlayer::connect(device, 0.5, events_tx))
            .await
            .unwrap()
            .unwrap();
        player.load(load(1));
        player.preload(load(2));

        let mut next_load = async || {
            tokio::time::timeout(wait, loads.recv())
                .await
                .unwrap()
                .unwrap()
        };
        let first = next_load().await;
        assert_eq!(first["media"]["contentId"], "https://example.com/1.m4a");
        assert_eq!(first["media"]["metadata"]["title"], "Song");
        let second = next_load().await;
        assert_eq!(second["media"]["contentId"], "https://example.com/2.m4a");

        let mut next_event = async || {
            tokio::time::timeout(wait, events.recv())
                .await
                .unwrap()
                .unwrap()
        };
        assert!(matches!(
            next_event().await,
            PlayerEvent::Started { generation: 2 }
        ));
        assert!(matches!(
            next_event().await,
            PlayerEvent::Ended { generation: 2 }
        ));
        assert_eq!(player.status().state, PlayState::Idle);
    }
}
