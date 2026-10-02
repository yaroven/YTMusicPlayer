//! Self-update from GitHub releases: a check at startup (one small API
//! request), and an install that fits how the player was installed:
//!
//! - our own files (install.sh's `~/.local/bin` and `~/Applications` app,
//!   an unpacked tarball): the binary is replaced in place;
//! - the macOS `.pkg` (root-owned `/Applications`): its new `.pkg` opens in
//!   Installer;
//! - the Windows setup: the new setup runs silently, then the player
//!   starts again;
//! - Linux packages (`/usr/bin`): the system's package manager owns the
//!   files, so the player only says which package to install.
//!
//! Downloads are checked against the release's `SHA256SUMS`.

use std::{
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const LATEST: &str = "https://api.github.com/repos/yaroven/YTMusicPlayer/releases/latest";
/// Larger downloads are refused (the packages are ~10-20 MB).
const MAX_DOWNLOAD: u64 = 64 << 20;

/// A newer release.
#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    /// "0.7.1"
    pub version: String,
    /// Its GitHub page.
    pub page: String,
    assets: Vec<(String, String)>,
}

impl Update {
    fn asset(&self, name: &str) -> Option<&str> {
        self.assets
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, url)| url.as_str())
    }
}

/// What installing did.
#[derive(Debug, Clone, PartialEq)]
pub enum Installed {
    /// The binary was replaced: restart to run it.
    Replaced,
    /// An installer was started (it may need the user); `quit`: the player
    /// should exit so the installer can replace it.
    InstallerStarted { quit: bool },
    /// Nothing done: the system's package manager owns the files.
    Manual(String),
}

#[derive(Deserialize)]
struct ReleaseJson {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    assets: Vec<AssetJson>,
}

#[derive(Deserialize)]
struct AssetJson {
    name: String,
    browser_download_url: String,
}

/// The latest release, if it's newer than this build.
pub async fn check(http: &reqwest::Client) -> Result<Option<Update>> {
    let release: ReleaseJson = http
        .get(LATEST)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .context("checking for updates")?
        .json()
        .await
        .context("reading the latest release")?;
    let version = release.tag_name.trim_start_matches('v').to_owned();
    if release.draft || release.prerelease || !newer(&version, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    Ok(Some(Update {
        version,
        page: release.html_url,
        assets: release
            .assets
            .into_iter()
            .map(|a| (a.name, a.browser_download_url))
            .collect(),
    }))
}

/// Whether `a` ("0.7.1") is a later version than `b`.
fn newer(a: &str, b: &str) -> bool {
    fn parts(v: &str) -> Option<(u32, u32, u32)> {
        let mut it = v.split(['.', '-', '+']).map(|p| p.parse().ok());
        Some((it.next()??, it.next()??, it.next()??))
    }
    matches!((parts(a), parts(b)), (Some(a), Some(b)) if a > b)
}

/// How this copy of the player can be updated.
#[derive(Debug, Clone, PartialEq)]
enum Method {
    /// Replace `exe` with the tarball's binary; `bundle`: the macOS app it
    /// lives in (re-signed afterwards).
    Replace {
        exe: PathBuf,
        bundle: Option<PathBuf>,
    },
    /// Open the release's .pkg in Installer.
    MacPkg,
    /// Run the release's setup.exe silently, then start `exe` again.
    WindowsSetup { exe: PathBuf },
    /// Installed from a Linux package: `kind` is "deb", "rpm" or "arch".
    LinuxPackage { kind: &'static str },
}

fn method() -> Result<Method> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let text = exe.to_string_lossy();
    if cfg!(windows) {
        return Ok(Method::WindowsSetup { exe });
    }
    if text.starts_with("/usr/") && !text.starts_with("/usr/local/") {
        let kind = if Path::new("/etc/debian_version").exists() {
            "deb"
        } else if Path::new("/etc/arch-release").exists() {
            "arch"
        } else {
            "rpm"
        };
        return Ok(Method::LinuxPackage { kind });
    }
    let bundle = exe
        .ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .map(Path::to_path_buf);
    // The .pkg puts the app in /Applications, owned by root.
    let writable = exe.parent().is_some_and(is_writable);
    if cfg!(target_os = "macos") && !writable {
        return Ok(Method::MacPkg);
    }
    if !writable {
        bail!("can't write to {}", exe.display());
    }
    Ok(Method::Replace { exe, bundle })
}

fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".ytm-update-probe-{}", std::process::id()));
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// This platform's release archive.
fn tarball() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("ytm-aarch64-apple-darwin.tar.gz"),
        ("macos", "x86_64") => Some("ytm-x86_64-apple-darwin.tar.gz"),
        ("linux", "x86_64") => Some("ytm-x86_64-unknown-linux-gnu.tar.gz"),
        _ => None,
    }
}

/// Installs `update` the way this copy was installed.
pub async fn install(http: &reqwest::Client, update: &Update) -> Result<Installed> {
    let v = &update.version;
    match method()? {
        Method::LinuxPackage { kind } => {
            let (file, command) = match kind {
                "deb" => (format!("ytm-player_{v}_amd64.deb"), "sudo apt install ./"),
                "arch" => (
                    format!("ytm-player-{v}-1-x86_64.pkg.tar.zst"),
                    "sudo pacman -U ",
                ),
                _ => (
                    format!("ytm-player-{v}-1.x86_64.rpm"),
                    "sudo dnf install ./",
                ),
            };
            Ok(Installed::Manual(format!(
                "Installed from a package: download {file} from {} and run `{command}{file}`",
                update.page
            )))
        }
        Method::MacPkg => {
            let name = format!("ytm-player-{v}-macos.pkg");
            let pkg = download(http, update, &name).await?;
            let dest = std::env::temp_dir().join(&name);
            tokio::fs::write(&dest, &pkg).await?;
            std::process::Command::new("open")
                .arg(&dest)
                .spawn()
                .context("opening the installer")?;
            Ok(Installed::InstallerStarted { quit: false })
        }
        Method::WindowsSetup { exe } => {
            let name = format!("ytm-player-{v}-setup-x64.exe");
            let setup = download(http, update, &name).await?;
            let dest = std::env::temp_dir().join(&name);
            tokio::fs::write(&dest, &setup).await?;
            // Wait for this process to exit, install silently, start again.
            let script = format!(
                "timeout /t 2 /nobreak >nul & \"{}\" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART & start \"\" \"{}\" gui",
                dest.display(),
                exe.display()
            );
            std::process::Command::new("cmd")
                .args(["/C", &script])
                .spawn()
                .context("starting the installer")?;
            Ok(Installed::InstallerStarted { quit: true })
        }
        Method::Replace { exe, bundle } => {
            let name = tarball().context("no prebuilt binary for this platform")?;
            let archive = download(http, update, name).await?;
            let binary = tokio::task::spawn_blocking(move || unpack_binary(&archive)).await??;
            replace(&exe, &binary)?;
            if let Some(bundle) = &bundle {
                // A changed binary breaks the app's ad-hoc signature, and
                // Apple Silicon won't run it unsigned.
                let _ = std::process::Command::new("codesign")
                    .args(["--force", "--sign", "-"])
                    .arg(bundle)
                    .status();
                // install.sh also put a copy on PATH.
                if let Some(home) = directories::BaseDirs::new() {
                    let cli = home.home_dir().join(".local/bin/ytm");
                    if cli.is_file() && cli != exe {
                        let _ = replace(&cli, &binary);
                    }
                }
            }
            Ok(Installed::Replaced)
        }
    }
}

/// Downloads a release file and checks it against `SHA256SUMS`.
async fn download(http: &reqwest::Client, update: &Update, name: &str) -> Result<Vec<u8>> {
    let url = update
        .asset(name)
        .with_context(|| format!("{name} isn't in release {}", update.version))?;
    let sums_url = update
        .asset("SHA256SUMS")
        .context("the release has no SHA256SUMS")?;
    let sums = http
        .get(sums_url)
        .send()
        .await
        .and_then(|r| r.error_for_status())?
        .text()
        .await?;
    let expected = sums
        .lines()
        .find_map(|l| {
            let (hash, file) = l.split_once(char::is_whitespace)?;
            (file.trim_start_matches(['*', ' ']) == name).then(|| hash.to_lowercase())
        })
        .with_context(|| format!("no checksum for {name}"))?;
    let response = http
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .with_context(|| format!("downloading {name}"))?;
    if response.content_length().is_some_and(|n| n > MAX_DOWNLOAD) {
        bail!("{name} is unexpectedly large");
    }
    let bytes = response.bytes().await?.to_vec();
    let actual = hex::encode(Sha256::digest(&bytes));
    if actual != expected {
        bail!("{name}: checksum mismatch, not installing");
    }
    Ok(bytes)
}

/// The `ytm` binary out of a release tarball.
fn unpack_binary(archive: &[u8]) -> Result<Vec<u8>> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in tar.entries()? {
        let mut entry = entry?;
        if entry.path()?.file_name().is_some_and(|n| n == "ytm") {
            let mut binary = Vec::new();
            entry.read_to_end(&mut binary)?;
            return Ok(binary);
        }
    }
    bail!("the archive has no ytm binary")
}

/// Writes `binary` next to `exe` and renames it over (atomic; the running
/// process keeps its old file).
fn replace(exe: &Path, binary: &[u8]) -> Result<()> {
    let tmp = exe.with_extension("update");
    std::fs::write(&tmp, binary).with_context(|| format!("writing {}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&tmp, exe).with_context(|| format!("replacing {}", exe.display()))?;
    Ok(())
}

/// Starts the window again once this process has exited (after
/// [`Installed::Replaced`]).
pub fn relaunch_gui() -> Result<()> {
    let exe = std::env::current_exe()?;
    let bundle = exe
        .ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .map(Path::to_path_buf);
    let target = bundle.as_deref().unwrap_or(&exe);
    let quoted = target.to_string_lossy().replace('\'', r"'\''");
    // A LaunchServices start for the app (an exec'd one loses its menu bar
    // icon); the binary itself otherwise.
    let script = if bundle.is_some() {
        format!("sleep 1; open -n '{quoted}'")
    } else {
        format!("sleep 1; exec '{quoted}' gui")
    };
    std::process::Command::new("sh")
        .args(["-c", &script])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(newer("0.7.1", "0.7.0"));
        assert!(newer("0.10.0", "0.9.9"));
        assert!(newer("1.0.0", "0.99.0"));
        assert!(!newer("0.7.0", "0.7.0"));
        assert!(!newer("0.6.9", "0.7.0"));
        assert!(!newer("garbage", "0.7.0"));
    }

    #[test]
    fn finds_the_binary_in_a_release_tarball() {
        let mut tar = tar::Builder::new(Vec::new());
        let data = b"\x7fELF fake";
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, "ytm", &data[..]).unwrap();
        let raw = tar.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &raw).unwrap();
        let archive = gz.finish().unwrap();
        assert_eq!(unpack_binary(&archive).unwrap(), data);
    }

    #[test]
    fn replaces_a_file_atomically() {
        let dir = std::env::temp_dir().join(format!("ytm-update-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("ytm");
        std::fs::write(&exe, b"old").unwrap();
        replace(&exe, b"new").unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    #[ignore]
    async fn checks_github_live() {
        let http = reqwest::Client::builder()
            .user_agent("ytm-player-test")
            .build()
            .unwrap();
        // Either "up to date" or a newer release with its assets.
        if let Some(update) = check(&http).await.unwrap() {
            assert!(update.asset("SHA256SUMS").is_some());
        }
    }
}
