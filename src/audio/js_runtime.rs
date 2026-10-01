//! Optional JavaScript runtime for yt-dlp's YouTube challenge solver (EJS).
//!
//! yt-dlp runs it as a short-lived child only while resolving a URL; nothing
//! stays resident. We prefer a runtime the user already has, and otherwise
//! fetch QuickJS-NG (~1.3-2.5 MB) instead of Deno (~100 MB).

use std::path::{Path, PathBuf};

use super::install::{InstallError, download_verified};

/// Pinned so the binary is verified against hashes compiled into the app,
/// not against metadata fetched from the same server as the binary.
/// QuickJS-NG >= 0.12 is required for acceptable EJS speed.
const QUICKJS_VERSION: &str = "v0.17.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsKind {
    Deno,
    Node,
    QuickJs,
}

impl JsKind {
    /// Name yt-dlp expects in `--js-runtimes NAME:PATH`.
    fn ytdlp_name(self) -> &'static str {
        match self {
            Self::Deno => "deno",
            Self::Node => "node",
            Self::QuickJs => "quickjs",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsRuntime {
    pub kind: JsKind,
    pub path: PathBuf,
}

impl JsRuntime {
    /// Value for yt-dlp's `--js-runtimes`.
    pub fn ytdlp_arg(&self) -> String {
        format!("{}:{}", self.kind.ytdlp_name(), self.path.display())
    }

    pub fn managed_path(managed_dir: &Path) -> PathBuf {
        managed_dir.join(if cfg!(windows) { "qjs.exe" } else { "qjs" })
    }

    /// A runtime already on this machine, cheapest to start first.
    /// yt-dlp itself rejects versions that are too old.
    pub fn locate(managed_dir: &Path) -> Option<Self> {
        let managed = Self::managed_path(managed_dir);
        if managed.is_file() {
            return Some(Self {
                kind: JsKind::QuickJs,
                path: managed,
            });
        }
        [
            ("deno", JsKind::Deno),
            ("node", JsKind::Node),
            ("qjs", JsKind::QuickJs),
        ]
        .into_iter()
        .find_map(|(bin, kind)| which::which(bin).ok().map(|path| Self { kind, path }))
    }

    /// Downloads the pinned QuickJS-NG build into `managed_dir`.
    pub async fn download_quickjs(
        client: &reqwest::Client,
        managed_dir: &Path,
    ) -> Result<Option<Self>, InstallError> {
        let Some((asset, sha256)) = quickjs_asset() else {
            return Ok(None);
        };
        let url = format!(
            "https://github.com/quickjs-ng/quickjs/releases/download/{QUICKJS_VERSION}/{asset}"
        );
        let path = Self::managed_path(managed_dir);
        download_verified(client, &url, sha256, &path).await?;
        tracing::info!(path = %path.display(), version = QUICKJS_VERSION, "QuickJS-NG installed");
        Ok(Some(Self {
            kind: JsKind::QuickJs,
            path,
        }))
    }
}

fn quickjs_asset() -> Option<(&'static str, &'static str)> {
    use std::env::consts::{ARCH, OS};
    Some(match (OS, ARCH) {
        ("macos", "aarch64") => (
            "qjs-darwin-arm64",
            "8be3ddfe3397d2e692e4e1e8972ee9d032a0a580505d2f8b4ea528cf1b651c11",
        ),
        ("macos", "x86_64") => (
            "qjs-darwin-x86_64",
            "9e5e101b4fd13cda3204222ca9f8be35412c41dcdef3745829633b7a67245412",
        ),
        ("linux", "x86_64") => (
            "qjs-linux-x86_64",
            "0bfc02511a9f549c28b53880d988fc7cd5d361e90c5e8afdfcd7dc6774ceace5",
        ),
        ("linux", "aarch64") => (
            "qjs-linux-aarch64",
            "3372133484edf50a69f3c67903af41206d22a061e930e3cfb63269272ef56d2e",
        ),
        ("linux", "x86") => (
            "qjs-linux-x86",
            "fa29de6c5e9b27ccc4b75fefb6d9b73f744e1899b4011288de11a2f5f4afab2a",
        ),
        ("linux", "arm") => (
            "qjs-linux-armv7",
            "dbda6e499471c675bd991dbc477f14d6134c182409d15047e9b63e3df7bc3e64",
        ),
        // No native arm64 build; Windows on ARM runs x64 under emulation.
        ("windows", "x86_64" | "aarch64") => (
            "qjs-windows-x86_64.exe",
            "2aeabf0092c3262d6b2609824418f7dd7ed1f1df939f73b2b15645230cac0d77",
        ),
        ("windows", "x86") => (
            "qjs-windows-x86.exe",
            "dedc4dd8da20234d206c433a706336fede47a14e75c690cb607f81aa9c9c71be",
        ),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ytdlp_arg_format() {
        let rt = JsRuntime {
            kind: JsKind::QuickJs,
            path: PathBuf::from("/opt/qjs"),
        };
        assert_eq!(rt.ytdlp_arg(), "quickjs:/opt/qjs");
    }

    #[test]
    fn current_platform_has_quickjs() {
        assert!(quickjs_asset().is_some());
    }
}
