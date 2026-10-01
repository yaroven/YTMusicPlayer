//! yt-dlp discovery, provisioning and direct audio URL resolution.
//!
//! Lookup order: `yt-dlp` in `PATH`, then the app-managed copy in
//! `<data_dir>/bin`. If neither runs, [`YtDlp::ensure`] downloads the official
//! standalone binary for the current OS/arch from GitHub releases and verifies
//! it against the release's `SHA2-256SUMS` before making it executable.
//!
//! No JS runtime is passed by default: measured on 2026-10-01 (yt-dlp
//! 2026.08.19), resolving without one took ~6 s / ~94 MB peak and still
//! returned working AAC URLs. [`YtDlp::resolve_with_js`] is the fallback.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::Deserialize;
use tokio::process::Command;

use super::{
    install::{InstallError, download_verified},
    js_runtime::JsRuntime,
};

const RELEASE_BASE: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download";

/// First run of a PyInstaller one-file build unpacks itself, so be generous.
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(60);
const UPDATE_TIMEOUT: Duration = Duration::from_secs(120);

/// Prefer progressive AAC in MP4 (itag 140): symphonia (via rodio) decodes it,
/// but has no Opus decoder. `protocol^=http` excludes HLS/DASH manifests.
pub const DEFAULT_FORMAT: &str = "bestaudio[ext=m4a][protocol^=http]/bestaudio[acodec^=mp4a][protocol^=http]/bestaudio[protocol^=http]";

pub type Result<T, E = ExtractorError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum ExtractorError {
    #[error("yt-dlp not found in PATH or in {0}")]
    NotFound(PathBuf),
    #[error("no prebuilt yt-dlp for {os}/{arch}; install it with your package manager")]
    UnsupportedPlatform {
        os: &'static str,
        arch: &'static str,
    },
    #[error("invalid YouTube video id: {0:?}")]
    InvalidVideoId(String),
    #[error("failed to spawn {path}: {source}")]
    Spawn {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("yt-dlp timed out after {0:?}")]
    Timeout(Duration),
    #[error("video unavailable: {0}")]
    Unavailable(String),
    #[error("yt-dlp exited with {status}: {message}")]
    Failed { status: ExitStatus, message: String },
    #[error("failed to parse yt-dlp output: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("no direct HTTP URL for {0} (format selector matched nothing streamable)")]
    NoUrl(String),
    #[error("self-update is only supported for the app-managed yt-dlp binary")]
    NotManaged,
    #[error("download failed: {0}")]
    Download(#[from] reqwest::Error),
    #[error("{0} is missing from SHA2-256SUMS")]
    MissingChecksum(&'static str),
    #[error(transparent)]
    Install(#[from] InstallError),
    #[error("yt-dlp needs a JavaScript runtime and none could be found or installed")]
    NoJsRuntime,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Where the binary came from. Only a managed binary may self-update:
/// a system one belongs to the user's package manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinarySource {
    System,
    Managed,
}

/// A direct, time-limited audio URL plus what's needed to play it.
#[derive(Debug, Clone)]
pub struct AudioStream {
    pub video_id: String,
    pub title: Option<String>,
    pub url: String,
    pub format_id: Option<String>,
    /// Container extension, e.g. `m4a` or `webm`.
    pub ext: Option<String>,
    /// Codec, e.g. `mp4a.40.2` or `opus`.
    pub codec: Option<String>,
    pub bitrate_kbps: Option<f64>,
    pub sample_rate: Option<u32>,
    pub duration: Option<Duration>,
    pub content_length: Option<u64>,
    /// Headers yt-dlp expects on the media request (User-Agent etc.).
    pub http_headers: HashMap<String, String>,
    /// From the URL's `expire` param; googlevideo URLs live ~6h.
    pub expires_at: Option<SystemTime>,
}

impl AudioStream {
    /// True when the URL is still valid for at least `margin` from now.
    pub fn is_fresh(&self, margin: Duration) -> bool {
        self.expires_at
            .is_some_and(|exp| SystemTime::now() + margin < exp)
    }

    /// True for codecs our rodio/symphonia build can decode.
    pub fn is_decodable(&self) -> bool {
        matches!(
            self.ext.as_deref(),
            Some("m4a" | "mp4" | "mp3" | "ogg" | "flac" | "wav")
        ) && !self.codec.as_deref().is_some_and(|c| c.starts_with("opus"))
    }
}

#[derive(Debug, Clone)]
pub struct YtDlp {
    path: PathBuf,
    source: BinarySource,
    format: String,
    extra_args: Vec<String>,
}

impl YtDlp {
    pub fn new(path: impl Into<PathBuf>, source: BinarySource) -> Self {
        Self {
            path: path.into(),
            source,
            format: DEFAULT_FORMAT.to_owned(),
            extra_args: Vec::new(),
        }
    }

    /// Override the `-f` format selector.
    pub fn with_format(mut self, format: impl Into<String>) -> Self {
        self.format = format.into();
        self
    }

    /// Extra flags appended before the URL, e.g.
    /// `["--cookies-from-browser", "firefox"]` for age-restricted tracks.
    pub fn with_extra_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.extra_args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn source(&self) -> BinarySource {
        self.source
    }

    /// Path of the app-managed binary inside `managed_dir`.
    pub fn managed_path(managed_dir: &Path) -> PathBuf {
        managed_dir.join(executable_name())
    }

    /// Finds a *working* yt-dlp (one that answers `--version`).
    pub async fn locate(managed_dir: &Path) -> Option<Self> {
        let mut candidates = Vec::with_capacity(2);
        if let Ok(path) = which::which("yt-dlp") {
            candidates.push(Self::new(path, BinarySource::System));
        }
        let managed = Self::managed_path(managed_dir);
        if managed.is_file() {
            candidates.push(Self::new(managed, BinarySource::Managed));
        }

        for candidate in candidates {
            match candidate.version().await {
                Ok(version) => {
                    tracing::info!(path = %candidate.path.display(), %version, "using yt-dlp");
                    return Some(candidate);
                }
                Err(err) => {
                    tracing::warn!(path = %candidate.path.display(), %err, "yt-dlp candidate unusable");
                }
            }
        }
        None
    }

    /// [`locate`](Self::locate), falling back to downloading the managed binary.
    pub async fn ensure(client: &reqwest::Client, managed_dir: &Path) -> Result<Self> {
        if let Some(found) = Self::locate(managed_dir).await {
            return Ok(found);
        }
        tracing::info!(dir = %managed_dir.display(), "yt-dlp not found, downloading");
        let ytdlp = Self::download(client, managed_dir).await?;
        ytdlp.version().await?;
        Ok(ytdlp)
    }

    /// Downloads the latest official build into `managed_dir`, verifying SHA-256.
    pub async fn download(client: &reqwest::Client, managed_dir: &Path) -> Result<Self> {
        let asset = release_asset()?;
        tokio::fs::create_dir_all(managed_dir).await?;

        let sums = client
            .get(format!("{RELEASE_BASE}/SHA2-256SUMS"))
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let expected = find_checksum(&sums, asset).ok_or(ExtractorError::MissingChecksum(asset))?;

        let final_path = Self::managed_path(managed_dir);
        download_verified(
            client,
            &format!("{RELEASE_BASE}/{asset}"),
            expected,
            &final_path,
        )
        .await?;
        mark_updated(managed_dir).await;

        tracing::info!(path = %final_path.display(), asset, "yt-dlp installed");
        Ok(Self::new(final_path, BinarySource::Managed))
    }

    pub async fn version(&self) -> Result<String> {
        let stdout = self.run(&["--version"], PROBE_TIMEOUT).await?;
        Ok(stdout.trim().to_owned())
    }

    /// Runs `yt-dlp -U`. YouTube breaks extractors often; call this when
    /// [`resolve`](Self::resolve) starts failing with [`ExtractorError::Failed`].
    pub async fn update(&self) -> Result<String> {
        if self.source != BinarySource::Managed {
            return Err(ExtractorError::NotManaged);
        }
        let out = self.run(&["-U"], UPDATE_TIMEOUT).await?;
        if let Some(dir) = self.path.parent() {
            mark_updated(dir).await;
        }
        Ok(out)
    }

    /// Runs [`update`](Self::update) if the managed binary wasn't updated
    /// within `max_age`. Returns `Ok(false)` when nothing had to be done.
    pub async fn update_if_stale(&self, max_age: Duration) -> Result<bool> {
        if self.source != BinarySource::Managed {
            return Ok(false);
        }
        let stamp = self.path.with_file_name(UPDATE_STAMP);
        let fresh = tokio::fs::metadata(&stamp)
            .await
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age < max_age);
        if fresh {
            return Ok(false);
        }
        self.update().await?;
        Ok(true)
    }

    /// Resolves a video id into a direct, playable audio URL without any JS
    /// runtime. Cheapest path (one Python process); works for most tracks.
    pub async fn resolve(&self, video_id: &str) -> Result<AudioStream> {
        self.resolve_inner(video_id, None).await
    }

    /// Like [`resolve`](Self::resolve), but lets yt-dlp solve YouTube's JS
    /// challenges with `js`. Slower and uses more memory while it runs.
    pub async fn resolve_with_js(&self, video_id: &str, js: &JsRuntime) -> Result<AudioStream> {
        self.resolve_inner(video_id, Some(js)).await
    }

    async fn resolve_inner(&self, video_id: &str, js: Option<&JsRuntime>) -> Result<AudioStream> {
        validate_video_id(video_id)?;
        let page_url = format!("https://www.youtube.com/watch?v={video_id}");

        let mut args: Vec<&str> = vec![
            "--ignore-config", // user's config could change output/format
            "--no-playlist",
            "--no-warnings",
            "--no-progress",
            "--dump-single-json",
            "--format",
            &self.format,
            // Deterministic: never pick up a runtime implicitly from PATH.
            "--no-js-runtimes",
        ];
        let js_arg = js.map(JsRuntime::ytdlp_arg);
        if let Some(arg) = &js_arg {
            args.extend(["--js-runtimes", arg]);
        }
        args.extend(self.extra_args.iter().map(String::as_str));
        args.push("--"); // nothing after this is parsed as a flag
        args.push(&page_url);

        let stdout = self.run(&args, RESOLVE_TIMEOUT).await?;
        parse_stream(video_id, &stdout)
    }

    async fn run(&self, args: &[&str], timeout: Duration) -> Result<String> {
        let mut cmd = Command::new(&self.path);
        cmd.args(args)
            // Never let the child read the TUI's stdin.
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("PYTHONIOENCODING", "utf-8")
            .kill_on_drop(true);

        #[cfg(windows)]
        {
            // Don't flash a console window when spawned from a GUI-less parent.
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let child = cmd.spawn().map_err(|source| ExtractorError::Spawn {
            path: self.path.clone(),
            source,
        })?;

        // On timeout the future is dropped, and kill_on_drop reaps the child.
        let output = tokio::time::timeout(timeout, child.wait_with_output())
            .await
            .map_err(|_| ExtractorError::Timeout(timeout))??;

        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
        Err(classify_failure(
            output.status,
            &String::from_utf8_lossy(&output.stderr),
        ))
    }
}

/// Touched after every successful install/update; its mtime drives
/// [`YtDlp::update_if_stale`].
const UPDATE_STAMP: &str = ".yt-dlp-updated";

async fn mark_updated(managed_dir: &Path) {
    if let Err(err) = tokio::fs::write(managed_dir.join(UPDATE_STAMP), b"").await {
        tracing::warn!(%err, "failed to write yt-dlp update stamp");
    }
}

fn executable_name() -> &'static str {
    if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    }
}

/// Official standalone asset name for this build target.
fn release_asset() -> Result<&'static str> {
    use std::env::consts::{ARCH, OS};
    let musl = cfg!(target_env = "musl");
    Ok(match (OS, ARCH) {
        ("macos", "x86_64" | "aarch64") => "yt-dlp_macos", // universal2
        ("linux", "x86_64") if musl => "yt-dlp_musllinux",
        ("linux", "aarch64") if musl => "yt-dlp_musllinux_aarch64",
        ("linux", "x86_64") => "yt-dlp_linux",
        ("linux", "aarch64") => "yt-dlp_linux_aarch64",
        ("windows", "x86_64") => "yt-dlp.exe",
        ("windows", "x86") => "yt-dlp_x86.exe",
        ("windows", "aarch64") => "yt-dlp_arm64.exe",
        (os, arch) => return Err(ExtractorError::UnsupportedPlatform { os, arch }),
    })
}

/// Parses `sha256sum` output (`<hex>  <name>` or `<hex> *<name>`).
fn find_checksum<'a>(sums: &'a str, asset: &str) -> Option<&'a str> {
    sums.lines().find_map(|line| {
        let (hash, name) = line.trim().split_once(char::is_whitespace)?;
        let name = name.trim_start().trim_start_matches('*');
        (name == asset).then_some(hash)
    })
}

/// Video ids are exactly 11 chars of `[A-Za-z0-9_-]`. Also blocks argument
/// injection such as an id of `--exec=...`.
fn validate_video_id(id: &str) -> Result<()> {
    let valid = id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if valid {
        Ok(())
    } else {
        Err(ExtractorError::InvalidVideoId(id.to_owned()))
    }
}

fn classify_failure(status: ExitStatus, stderr: &str) -> ExtractorError {
    let message = stderr
        .lines()
        .rev()
        .find(|l| l.starts_with("ERROR:"))
        .or_else(|| stderr.lines().rev().find(|l| !l.trim().is_empty()))
        .unwrap_or("no output")
        .trim()
        .to_owned();

    // Final errors: retrying with a JS runtime or a newer yt-dlp won't help.
    // Matched case-insensitively ("Video unavailable" / "This video is unavailable").
    const UNAVAILABLE: [&str; 7] = [
        "video unavailable",
        "is unavailable",
        "private video",
        "has been removed",
        "sign in to confirm your age",
        "not available in your country",
        "members-only",
    ];
    let lower = message.to_lowercase();
    if UNAVAILABLE.iter().any(|needle| lower.contains(needle)) {
        ExtractorError::Unavailable(message)
    } else {
        ExtractorError::Failed { status, message }
    }
}

/// Subset of yt-dlp's info dict. With a single (non-merged) format selected,
/// the chosen format's fields are hoisted to the top level.
#[derive(Debug, Deserialize)]
struct RawInfo {
    title: Option<String>,
    duration: Option<f64>,
    url: Option<String>,
    format_id: Option<String>,
    ext: Option<String>,
    acodec: Option<String>,
    abr: Option<f64>,
    asr: Option<f64>,
    filesize: Option<u64>,
    filesize_approx: Option<f64>,
    #[serde(default)]
    http_headers: HashMap<String, String>,
}

fn parse_stream(video_id: &str, json: &str) -> Result<AudioStream> {
    let info: RawInfo = serde_json::from_str(json)?;
    let url = info
        .url
        .filter(|u| u.starts_with("http"))
        .ok_or_else(|| ExtractorError::NoUrl(video_id.to_owned()))?;

    Ok(AudioStream {
        video_id: video_id.to_owned(),
        title: info.title,
        expires_at: expiry_from_url(&url),
        url,
        format_id: info.format_id,
        ext: info.ext,
        codec: info.acodec.filter(|c| c != "none"),
        bitrate_kbps: info.abr,
        sample_rate: info.asr.map(|r| r as u32),
        duration: info
            .duration
            .filter(|d| d.is_finite() && *d >= 0.0)
            .map(Duration::from_secs_f64),
        content_length: info.filesize.or(info.filesize_approx.map(|s| s as u64)),
        http_headers: info.http_headers,
    })
}

fn expiry_from_url(raw: &str) -> Option<SystemTime> {
    let url = url::Url::parse(raw).ok()?;
    let secs = url
        .query_pairs()
        .find(|(k, _)| k == "expire")
        .and_then(|(_, v)| v.parse::<u64>().ok())?;
    Some(UNIX_EPOCH + Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_id_validation() {
        assert!(validate_video_id("dQw4w9WgXcQ").is_ok());
        assert!(validate_video_id("a-b_c-d_e-f").is_ok());
        assert!(validate_video_id("--exec=rm -").is_err());
        assert!(validate_video_id("short").is_err());
        assert!(validate_video_id("dQw4w9WgXcQ&list=1").is_err());
    }

    #[test]
    fn checksum_lookup() {
        let sums = "\
aaaa  yt-dlp
BBBB *yt-dlp_macos
cccc  yt-dlp_linux
";
        assert_eq!(find_checksum(sums, "yt-dlp_macos"), Some("BBBB"));
        assert_eq!(find_checksum(sums, "yt-dlp_linux"), Some("cccc"));
        assert_eq!(find_checksum(sums, "yt-dlp.exe"), None);
    }

    #[test]
    fn current_platform_has_asset() {
        assert!(release_asset().is_ok());
    }

    #[test]
    fn parses_info_json() {
        let json = r#"{
            "id": "dQw4w9WgXcQ",
            "title": "Never Gonna Give You Up",
            "duration": 213.0,
            "format_id": "140",
            "ext": "m4a",
            "acodec": "mp4a.40.2",
            "abr": 129.5,
            "asr": 44100,
            "filesize": 3433514,
            "url": "https://rr1---sn-abc.googlevideo.com/videoplayback?expire=1900000000&itag=140",
            "http_headers": {"User-Agent": "Mozilla/5.0"}
        }"#;
        let s = parse_stream("dQw4w9WgXcQ", json).unwrap();
        assert_eq!(s.format_id.as_deref(), Some("140"));
        assert_eq!(s.sample_rate, Some(44100));
        assert_eq!(s.duration, Some(Duration::from_secs(213)));
        assert_eq!(
            s.expires_at,
            Some(UNIX_EPOCH + Duration::from_secs(1_900_000_000))
        );
        assert_eq!(s.http_headers["User-Agent"], "Mozilla/5.0");
        assert!(s.is_decodable());
    }

    #[test]
    fn unavailable_is_final() {
        #[cfg(unix)] // wait status: exit code 1 lives in the high byte
        let status = std::os::unix::process::ExitStatusExt::from_raw(1 << 8);
        #[cfg(windows)]
        let status = std::os::windows::process::ExitStatusExt::from_raw(1);
        for stderr in [
            "ERROR: [youtube] aaaaaaaaaaa: This video is unavailable",
            "ERROR: [youtube] x: Video unavailable. This content isn't available.",
            "ERROR: [youtube] x: Private video. Sign in if you've been granted access",
        ] {
            assert!(matches!(
                classify_failure(status, stderr),
                ExtractorError::Unavailable(_)
            ));
        }
        let other = classify_failure(
            status,
            "WARNING: x\nERROR: [youtube] x: Requested format is not available",
        );
        assert!(matches!(other, ExtractorError::Failed { .. }));
    }

    #[test]
    fn missing_url_is_error() {
        let err = parse_stream("dQw4w9WgXcQ", r#"{"title":"x"}"#).unwrap_err();
        assert!(matches!(err, ExtractorError::NoUrl(_)));
    }

    #[test]
    fn opus_is_not_decodable() {
        let json = r#"{"url":"https://x.googlevideo.com/v?expire=1","ext":"webm","acodec":"opus"}"#;
        assert!(!parse_stream("dQw4w9WgXcQ", json).unwrap().is_decodable());
    }

    /// Hits the network and needs yt-dlp: `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn resolves_real_video() {
        let dir = std::env::temp_dir().join("ytm-player-test-bin");
        let ytdlp = YtDlp::ensure(&reqwest::Client::new(), &dir).await.unwrap();
        let stream = ytdlp.resolve("dQw4w9WgXcQ").await.unwrap();
        assert!(stream.url.starts_with("https://"));
        assert!(stream.is_fresh(Duration::from_secs(60)));
    }
}
