//! The signed-in YouTube session: youtube.com cookies left by Google's own
//! sign-in page (`signin_window`) or imported from a browser, kept in the
//! OS credential store. It signs InnerTube requests (`SAPISIDHASH`) and
//! gives yt-dlp a cookies.txt for age-restricted songs. Same scheme as the
//! iPhone app's `Auth` (ios/YTMKit).

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use anyhow::{Context, Result, bail};
use sha1::{Digest, Sha1};

use crate::api::token_store::{SERVICE, take_old_secret};

pub type Cookies = BTreeMap<String, String>;

const ACCOUNT: &str = "youtube-session";

/// Where the cookies live: the OS credential store, or memory in tests.
/// Blocking calls (D-Bus on Linux).
pub trait SecretStore: Send + Sync + 'static {
    fn load(&self) -> Result<Option<String>>;
    fn save(&self, value: &str) -> Result<()>;
    fn clear(&self) -> Result<()>;
}

pub struct Keyring;

impl Keyring {
    fn entry() -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, ACCOUNT).context("OS credential store unavailable")
    }
}

impl SecretStore for Keyring {
    fn load(&self) -> Result<Option<String>> {
        match Self::entry()?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(take_old_secret(ACCOUNT)),
            Err(err) => Err(err).context("reading the YouTube session"),
        }
    }

    fn save(&self, value: &str) -> Result<()> {
        Self::entry()?
            .set_password(value)
            .context("saving the YouTube session")
    }

    fn clear(&self) -> Result<()> {
        match Self::entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err).context("deleting the YouTube session"),
        }
    }
}

#[derive(Default)]
pub struct InMemory(std::sync::Mutex<Option<String>>);

impl SecretStore for InMemory {
    fn load(&self) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }

    fn save(&self, value: &str) -> Result<()> {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(value.to_owned());
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
        Ok(())
    }
}

/// Cheap to clone; every clone sees sign-in / sign-out.
#[derive(Clone)]
pub struct YouTubeSession {
    inner: Arc<Inner>,
}

struct Inner {
    cookies: RwLock<Cookies>,
    store: Box<dyn SecretStore>,
    /// cookies.txt for yt-dlp (0600), present while signed in.
    file: PathBuf,
}

impl YouTubeSession {
    /// Reads the stored session (blocking: call off the async runtime).
    pub fn load(store: impl SecretStore, file: PathBuf) -> Self {
        let cookies: Cookies = match store.load() {
            Ok(Some(json)) => serde_json::from_str(&json).unwrap_or_default(),
            Ok(None) => Cookies::new(),
            Err(err) => {
                tracing::warn!("{err:#}");
                Cookies::new()
            }
        };
        let session = Self {
            inner: Arc::new(Inner {
                cookies: RwLock::new(Cookies::new()),
                store: Box::new(store),
                file,
            }),
        };
        if is_signed_in(&cookies) {
            session.absorb_file_into(cookies);
        }
        session
    }

    /// Signed out, in memory (tests, `yarek resolve`).
    pub fn none() -> Self {
        Self::load(InMemory::default(), PathBuf::new())
    }

    pub fn signed_in(&self) -> bool {
        is_signed_in(&self.read())
    }

    /// The cookies.txt to hand yt-dlp, while signed in.
    pub fn cookies_file(&self) -> Option<&Path> {
        self.signed_in().then_some(self.inner.file.as_path())
    }

    pub fn sign_in(&self, cookies: Cookies) -> Result<()> {
        if !is_signed_in(&cookies) {
            bail!("no YouTube sign-in found (SAPISID / SID cookies missing)");
        }
        self.inner.store.save(&serde_json::to_string(&cookies)?)?;
        write_cookies_file(&self.inner.file, &cookies)?;
        *self
            .inner
            .cookies
            .write()
            .unwrap_or_else(|e| e.into_inner()) = cookies;
        Ok(())
    }

    pub fn sign_out(&self) -> Result<()> {
        self.inner
            .cookies
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        if self.inner.file.exists() {
            std::fs::remove_file(&self.inner.file).ok();
        }
        self.inner.store.clear()
    }

    /// Request headers for an InnerTube call from `origin`; none when
    /// signed out.
    pub fn headers(&self, origin: &str) -> Vec<(&'static str, String)> {
        let cookies = self.read();
        if !is_signed_in(&cookies) {
            return Vec::new();
        }
        let now = chrono::Utc::now().timestamp();
        vec![
            ("Cookie", cookie_header(&cookies)),
            ("Authorization", authorization(&cookies, origin, now)),
            ("X-Origin", origin.to_owned()),
            ("X-Goog-AuthUser", "0".to_owned()),
        ]
    }

    /// yt-dlp writes rotated cookies back into the cookies.txt; keep them
    /// (in the store too) so the session doesn't go stale.
    pub fn absorb_file(&self) {
        let current = self.read();
        if is_signed_in(&current) {
            self.absorb_file_into(current);
        }
    }

    fn absorb_file_into(&self, mut cookies: Cookies) {
        let mut changed = false;
        if let Ok(text) = std::fs::read_to_string(&self.inner.file) {
            for (name, value) in parse_cookies_txt(&text) {
                if cookies.get(&name) != Some(&value) && cookies.contains_key(&name) {
                    cookies.insert(name, value);
                    changed = true;
                }
            }
        }
        if changed && let Ok(json) = serde_json::to_string(&cookies) {
            if let Err(err) = self.inner.store.save(&json) {
                tracing::warn!("{err:#}");
            }
        }
        if let Err(err) = write_cookies_file(&self.inner.file, &cookies) {
            tracing::warn!("{err:#}");
        }
        *self
            .inner
            .cookies
            .write()
            .unwrap_or_else(|e| e.into_inner()) = cookies;
    }

    fn read(&self) -> Cookies {
        self.inner
            .cookies
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

/// The cookies that make a youtube.com session.
pub fn is_signed_in(cookies: &Cookies) -> bool {
    (cookies.contains_key("SAPISID") || cookies.contains_key("__Secure-3PAPISID"))
        && (cookies.contains_key("SID") || cookies.contains_key("__Secure-3PSID"))
}

fn cookie_header(cookies: &Cookies) -> String {
    cookies
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// `SAPISIDHASH <ts>_<sha1("<ts> <SAPISID> <origin>")>`, plus the 1P / 3P
/// variants, as youtube.com sends it.
fn authorization(cookies: &Cookies, origin: &str, ts: i64) -> String {
    let hash = |value: &str| hex::encode(Sha1::digest(format!("{ts} {value} {origin}")));
    [
        (
            "SAPISIDHASH",
            cookies
                .get("SAPISID")
                .or_else(|| cookies.get("__Secure-3PAPISID")),
        ),
        ("SAPISID1PHASH", cookies.get("__Secure-1PAPISID")),
        ("SAPISID3PHASH", cookies.get("__Secure-3PAPISID")),
    ]
    .into_iter()
    .filter_map(|(scheme, value)| value.map(|v| format!("{scheme} {ts}_{}", hash(v))))
    .collect::<Vec<_>>()
    .join(" ")
}

/// Netscape cookies.txt (what yt-dlp's `--cookies` reads).
fn cookies_txt(cookies: &Cookies) -> String {
    let expires = chrono::Utc::now().timestamp() + 365 * 24 * 3600;
    let mut out = String::from("# Netscape HTTP Cookie File\n");
    for (name, value) in cookies {
        out.push_str(&format!(
            ".youtube.com\tTRUE\t/\tTRUE\t{expires}\t{name}\t{value}\n"
        ));
    }
    out
}

/// youtube.com cookies from a cookies.txt (also yt-dlp's exports).
pub fn parse_cookies_txt(text: &str) -> Cookies {
    text.lines()
        .filter_map(|line| {
            // "#HttpOnly_.youtube.com" marks HttpOnly cookies, not comments.
            let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
            if line.starts_with('#') {
                return None;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            (fields.len() == 7 && fields[0].ends_with("youtube.com"))
                .then(|| (fields[5].to_owned(), fields[6].trim_end().to_owned()))
        })
        .collect()
}

fn write_cookies_file(path: &Path, cookies: &Cookies) -> Result<()> {
    if path.as_os_str().is_empty() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    options
        .open(path)
        .and_then(|mut f| f.write_all(cookies_txt(cookies).as_bytes()))
        .with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookies() -> Cookies {
        [
            ("SAPISID", "abc"),
            ("__Secure-3PAPISID", "abc"),
            ("SID", "s"),
            ("YSC", "y"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
    }

    #[test]
    fn authorization_matches_youtube() {
        // sha1("1700000000 abc https://www.youtube.com"), same as YTMKitChecks.
        let h = "1700000000_27b236f59d4ec583d7530f2c7055d2f9c6aecf92";
        assert_eq!(
            authorization(&cookies(), "https://www.youtube.com", 1_700_000_000),
            format!("SAPISIDHASH {h} SAPISID3PHASH {h}")
        );
    }

    #[test]
    fn sign_in_writes_a_private_cookies_file_and_round_trips() {
        let dir = std::env::temp_dir().join(format!("ytm-session-{}", std::process::id()));
        let file = dir.join("youtube-cookies.txt");
        let session = YouTubeSession::load(InMemory::default(), file.clone());
        assert!(!session.signed_in() && session.headers("x").is_empty());
        assert!(
            session
                .sign_in([("YSC".to_owned(), "y".to_owned())].into())
                .is_err()
        );

        session.sign_in(cookies()).unwrap();
        assert!(session.signed_in());
        assert_eq!(session.cookies_file(), Some(file.as_path()));
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(parse_cookies_txt(&text), cookies());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let headers = session.headers("https://music.youtube.com");
        assert!(headers.iter().any(
            |(k, v)| *k == "Cookie" && v == "SAPISID=abc; SID=s; YSC=y; __Secure-3PAPISID=abc"
        ));

        // yt-dlp rotated a cookie in the file.
        std::fs::write(&file, text.replace("\tYSC\ty", "\tYSC\tz")).unwrap();
        session.absorb_file();
        assert!(
            session
                .headers("o")
                .iter()
                .any(|(k, v)| *k == "Cookie" && v.contains("YSC=z"))
        );

        session.sign_out().unwrap();
        assert!(!session.signed_in() && !file.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parses_yt_dlp_exports() {
        let text = "# Netscape HTTP Cookie File\n\
            #HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t0\tSID\ts\n\
            .youtube.com\tTRUE\t/\tTRUE\t0\tSAPISID\tabc\n\
            .google.com\tTRUE\t/\tTRUE\t0\tNID\tn\n";
        let parsed = parse_cookies_txt(text);
        assert_eq!(parsed.len(), 2);
        assert!(is_signed_in(&parsed));
    }
}
