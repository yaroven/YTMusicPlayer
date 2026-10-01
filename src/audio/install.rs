//! Download-and-verify for helper binaries (yt-dlp, QuickJS-NG) kept in the
//! app-managed `bin` directory.

use std::path::Path;

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("download failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("checksum mismatch for {url}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        url: String,
        expected: String,
        actual: String,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Streams `url` into `dest`, verifying SHA-256 (hex) before the file appears.
/// Writes to `<dest>.part` first, so a crash never leaves a half-written binary.
pub async fn download_verified(
    client: &reqwest::Client,
    url: &str,
    expected_sha256: &str,
    dest: &Path,
) -> Result<(), InstallError> {
    if let Some(dir) = dest.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let part = dest.with_extension("part");

    let mut response = client.get(url).send().await?.error_for_status()?;
    let mut file = tokio::fs::File::create(&part).await?;
    let mut hasher = Sha256::new();
    while let Some(chunk) = response.chunk().await? {
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
    }
    file.sync_all().await?;
    drop(file);

    let actual = hex::encode(hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected_sha256) {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(InstallError::ChecksumMismatch {
            url: url.to_owned(),
            expected: expected_sha256.to_ascii_lowercase(),
            actual,
        });
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755)).await?;
    }
    // Atomic on the same filesystem; replaces an older copy on all platforms.
    tokio::fs::rename(&part, dest).await?;
    Ok(())
}
