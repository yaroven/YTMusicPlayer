# Install, update, uninstall

For the iPhone app see [iphone.md](iphone.md).

## Installers

Download the file for your system from the
[latest release](https://github.com/yaroven/YTMusicPlayer/releases/latest).

| System | File | Install |
|---|---|---|
| Debian, Ubuntu, Mint | `ytm-player_<version>_amd64.deb` | `sudo apt install ./ytm-player_*_amd64.deb` |
| Fedora | `ytm-player-<version>-1.x86_64.rpm` | `sudo dnf install ./ytm-player-*.x86_64.rpm` |
| Arch, Manjaro | `ytm-player-<version>-1-x86_64.pkg.tar.zst` | `sudo pacman -U ./ytm-player-*.pkg.tar.zst` |
| Windows 10/11 | `ytm-player-<version>-setup-x64.exe` | run it (no admin rights needed) |
| macOS 11+ (Apple Silicon and Intel) | `ytm-player-<version>-macos.pkg` | open it |

Each one adds **ytm-player** to your applications (opens the window) and the
`ytm` command (terminal UI and the [commands](usage.md#commands)). Linux
packages need glibc 2.35+ (Debian 12, Ubuntu 22.04, current Fedora and Arch)
and are x86_64 only — other architectures can use `install.sh`, which builds
from source.

The installers aren't signed with a paid developer certificate, so the system
asks once:

- **macOS:** "Apple could not verify…" → **System Settings → Privacy &
  Security → Open Anyway**, then the installer runs normally.
- **Windows:** SmartScreen "Windows protected your PC" → **More info → Run
  anyway**.

## Install scripts (from the repository)

Linux, macOS:

```bash
git clone https://github.com/yaroven/YTMusicPlayer.git
cd YTMusicPlayer
./install.sh
```

Windows (PowerShell):

```powershell
git clone https://github.com/yaroven/YTMusicPlayer.git
cd YTMusicPlayer
powershell -ExecutionPolicy Bypass -File .\install.ps1
```

The scripts download a prebuilt binary from the latest release, or build from
source when there is none for your platform, installing build dependencies and
Rust as needed. They add the app to your applications (Linux menu entry,
`~/Applications/ytm-player.app` on macOS), remove older copies so only one
remains, and start the player. Re-run to update.

`install.sh` options: `--yes`, `--from-source`, `--no-deps`, `--no-launch`,
`--launcher-only` (only re-create the app entry).
`install.ps1` options: `-Yes`, `-FromSource`, `-NoLaunch`.

## Build from source

Requires Rust 1.85+ ([rustup](https://rustup.rs)) and a C toolchain (SQLite is
compiled in):

| OS | Install first |
|---|---|
| macOS | `xcode-select --install` |
| Debian/Ubuntu | `sudo apt install build-essential pkg-config libasound2-dev libfontconfig1-dev` |
| Fedora | `sudo dnf install gcc pkgconf-pkg-config alsa-lib-devel fontconfig-devel` |
| Arch | `sudo pacman -S base-devel alsa-lib fontconfig` |
| Windows | Visual Studio Build Tools, workload "Desktop development with C++" |

```bash
cargo install --path . --locked                        # window + terminal UI
cargo install --path . --locked --no-default-features  # terminal UI only (smaller)
```

## Updates

The player checks GitHub for a newer release when it starts (one small
request; `check_updates = false` turns it off). A new version shows as
**Update to X** in the sidebar and a red dot on the settings button;
**Settings → Update** installs it (also `ytm update` in a terminal), and
**Check now** looks again. Downloads are checked against the release's
SHA256SUMS.

| Installed with | What Update does |
|---|---|
| install.sh, or an unpacked archive | replaces the binary and restarts the window |
| the macOS .pkg | opens the new .pkg in Installer (asks for your password) |
| the Windows setup | installs the new setup silently and starts the player again |
| a Linux .deb / .rpm / Arch package | opens the release page and shows the install command (the package manager needs sudo) |

Installing a newer file over the old one always works too.

## Uninstall

Packages remove like any other package (your library and sign-in stay):

| Installed with | Remove |
|---|---|
| `.deb` | `sudo apt remove ytm-player` |
| `.rpm` | `sudo dnf remove ytm-player` |
| Arch package | `sudo pacman -R ytm-player` |
| Windows setup | **Settings → Apps → ytm-player → Uninstall** (asks whether to delete your library and sign-in) |

To remove everything else — the macOS .pkg, `install.sh` / `cargo install`
copies, menu entries, the PATH line, and optionally your library, settings,
logs and the stored Google sign-in — use the uninstaller:

```bash
ytm uninstall              # built into the app; asks before deleting your data
ytm uninstall --purge -y   # delete everything, no questions
./uninstall.sh             # the same script (repository or release download)
```

On Windows, `ytm uninstall` opens the setup program's uninstaller; for
`install.ps1` installs run
`powershell -ExecutionPolicy Bypass -File .\uninstall.ps1 [-Purge]`.

By hand: the binary (`/usr/bin/ytm`, `/usr/local/bin/ytm`, `~/.cargo/bin/ytm`
or `%LOCALAPPDATA%\Programs\ytm-player`), the app (`/Applications` or
`~/Applications/ytm-player.app`), the sign-in (`ytm logout`), and the data:

| OS | Data |
|---|---|
| Linux | `~/.config/ytm-player`, `~/.local/share/ytm-player`, `~/.cache/ytm-player` |
| macOS | `~/Library/Application Support/dev.ytm-player.ytm-player`, `~/Library/Caches/dev.ytm-player.ytm-player` |
| Windows | `%APPDATA%\ytm-player`, `%LOCALAPPDATA%\ytm-player` |

## Releases (for maintainers)

Pushing a tag builds every installer above (plus plain binaries for the
install scripts and the iPhone `.ipa`) and publishes them as a GitHub
Release. The workflow also installs each package in a clean Debian, Ubuntu,
Fedora, Arch, Windows and macOS environment and runs it.

```bash
git tag v0.9.4 && git push origin v0.9.4
```

**Actions → Release → Run workflow** builds everything as workflow artifacts
without publishing a release.
