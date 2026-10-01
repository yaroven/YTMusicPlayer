#!/usr/bin/env bash
# ytm-player installer for Linux and macOS.
# Installs a prebuilt `ytm` from GitHub Releases when one is available,
# otherwise builds it (installing build dependencies and Rust if missing).
# Then helps with the Google OAuth client and sign-in, and starts the player.
# Safe to re-run: use it again to update.
set -euo pipefail

YES=0
DEPS=1
LAUNCH=1
FROM_SOURCE=0

usage() {
  cat <<'EOF'
Usage: ./install.sh [options]

  -y, --yes        don't ask; accept defaults (never starts the TUI)
  --from-source    build from this checkout instead of downloading a release
  --no-deps        skip system package installation (source builds)
  --no-launch      don't start ytm at the end
  -h, --help       show this help
EOF
}

while (($#)); do
  case $1 in
    -y | --yes) YES=1 ;;
    --from-source) FROM_SOURCE=1 ;;
    --no-deps) DEPS=0 ;;
    --no-launch) LAUNCH=0 ;;
    -h | --help) usage && exit 0 ;;
    *) usage >&2 && exit 2 ;;
  esac
  shift
done

say() { printf '\033[1;31m==>\033[0m \033[1m%s\033[0m\n' "$*"; }
info() { printf '    %s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die() {
  printf '\033[1;31merror:\033[0m %s\n' "$*" >&2
  exit 1
}
run() {
  printf '    $ %s\n' "$*"
  "$@"
}

interactive() { ((!YES)) && [[ -r /dev/tty ]]; }

# ask "question" [y|n] -> success on yes. Non-interactive: the default.
ask() {
  local question=$1 default=${2:-y} hint reply
  if ! interactive; then
    [[ $default == y ]]
    return
  fi
  [[ $default == y ]] && hint="[Y/n]" || hint="[y/N]"
  read -r -p "    $question $hint " reply </dev/tty || reply=""
  reply=${reply:-$default}
  [[ $reply =~ ^[Yy] ]]
}

cd "$(dirname "$0")"
grep -q '^name = "ytm-player"' Cargo.toml 2>/dev/null ||
  die "run this script from the ytm-player repository"

OS=$(uname -s)
[[ $OS == Linux || $OS == Darwin ]] || die "unsupported OS: $OS (on Windows use install.ps1)"

REPO=$(git config --get remote.origin.url 2>/dev/null |
  sed -E 's#^(https://github.com/|git@github.com:)##; s#\.git$##' || true)
[[ $REPO == */* ]] || REPO="yaroven/YTMusicPlayer"

# --- 1. Prebuilt release ------------------------------------------------------------

target_triple() {
  case "$OS/$(uname -m)" in
    Linux/x86_64) echo x86_64-unknown-linux-gnu ;;
    Linux/aarch64 | Linux/arm64) echo aarch64-unknown-linux-gnu ;;
    Darwin/arm64) echo aarch64-apple-darwin ;;
    Darwin/x86_64) echo x86_64-apple-darwin ;;
    *) return 1 ;;
  esac
}

sha256_check() {
  if command -v sha256sum >/dev/null; then sha256sum -c "$1"; else shasum -a 256 -c "$1"; fi
}

# Downloads and installs the latest release binary into ~/.local/bin.
install_prebuilt() {
  local target asset tmp base="https://github.com/$REPO/releases/latest/download"
  target=$(target_triple) || return 1
  asset="ytm-$target.tar.gz"
  tmp=$(mktemp -d)
  # Public repo: plain download. Private repo: GitHub CLI with your login.
  if curl -fsSL -o "$tmp/$asset" "$base/$asset" 2>/dev/null &&
    curl -fsSL -o "$tmp/$asset.sha256" "$base/$asset.sha256" 2>/dev/null; then
    :
  elif command -v gh >/dev/null && gh auth status >/dev/null 2>&1 &&
    gh release download --repo "$REPO" --pattern "$asset" --pattern "$asset.sha256" \
      --dir "$tmp" >/dev/null 2>&1; then
    :
  else
    rm -rf "$tmp"
    return 1
  fi
  (cd "$tmp" && sha256_check "$asset.sha256" >/dev/null) || {
    rm -rf "$tmp"
    die "checksum mismatch for $asset"
  }
  tar -xzf "$tmp/$asset" -C "$tmp"
  mkdir -p "$HOME/.local/bin"
  install -m 755 "$tmp/ytm" "$HOME/.local/bin/ytm"
  rm -rf "$tmp"
  BIN="$HOME/.local/bin/ytm"
  # A binary that can't start (e.g. missing system audio library) is useless.
  "$BIN" --help >/dev/null 2>&1 || {
    warn "prebuilt binary doesn't run on this system; building instead"
    return 1
  }
  info "installed release $target to $BIN"
}

BIN=""
if ((!FROM_SOURCE)); then
  say "Downloading ytm"
  install_prebuilt || {
    info "no prebuilt release available here; building from source"
    BIN=""
  }
fi

# --- 2. Build from source (fallback) ---------------------------------------------------

install_linux_deps() {
  local sudo=() pkgs=()
  if ((EUID != 0)); then
    command -v sudo >/dev/null || die "sudo is required to install packages (or use --no-deps)"
    sudo=(sudo)
  fi

  if command -v apt-get >/dev/null; then
    pkgs=(build-essential pkg-config libasound2-dev curl git ca-certificates)
    run "${sudo[@]}" apt-get update -q
    run "${sudo[@]}" env DEBIAN_FRONTEND=noninteractive apt-get install -y -q "${pkgs[@]}"
  elif command -v dnf >/dev/null; then
    pkgs=(gcc make pkgconf-pkg-config alsa-lib-devel curl git)
    run "${sudo[@]}" dnf install -y "${pkgs[@]}"
  elif command -v pacman >/dev/null; then
    pkgs=(base-devel alsa-lib curl git)
    # PipeWire systems need the ALSA bridge for sound to reach PipeWire.
    if pacman -Qq pipewire >/dev/null 2>&1; then pkgs+=(pipewire-alsa); fi
    run "${sudo[@]}" pacman -S --needed --noconfirm "${pkgs[@]}"
  elif command -v zypper >/dev/null; then
    pkgs=(gcc make pkg-config alsa-devel curl git)
    run "${sudo[@]}" zypper --non-interactive install "${pkgs[@]}"
  else
    warn "unknown package manager; install a C compiler, pkg-config and ALSA headers yourself"
  fi
}

build_from_source() {
  if ((DEPS)); then
    say "System dependencies"
    if [[ $OS == Darwin ]]; then
      if xcode-select -p >/dev/null 2>&1; then
        info "Xcode Command Line Tools: installed"
      else
        xcode-select --install || true
        die "finish the Xcode Command Line Tools installer, then run this script again"
      fi
    else
      install_linux_deps
    fi
  fi

  say "Rust toolchain"
  local cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin"
  if ! command -v cargo >/dev/null && [[ -x $cargo_bin/cargo ]]; then
    export PATH="$cargo_bin:$PATH"
  fi
  if ! command -v cargo >/dev/null; then
    ask "Rust is not installed. Install it with rustup (https://rustup.rs)?" y ||
      die "Rust is required"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
    export PATH="$cargo_bin:$PATH"
  fi
  # Cargo.toml requires 1.85+.
  local rust_minor
  rust_minor=$(rustc --version | awk '{split($2, v, "."); print v[2]}')
  if ((rust_minor < 85)); then
    if command -v rustup >/dev/null; then
      run rustup update stable
    else
      die "Rust $(rustc --version | awk '{print $2}') is too old; 1.85+ required (distro Rust? install via rustup)"
    fi
  fi
  info "$(rustc --version)"

  say "Building ytm (release, takes a few minutes the first time)"
  run cargo install --path . --locked
  BIN="${CARGO_INSTALL_ROOT:-${CARGO_HOME:-$HOME/.cargo}}/bin/ytm"
  [[ -x $BIN ]] || die "build finished but $BIN is missing"
  info "installed: $BIN"
}

[[ -n $BIN ]] || build_from_source

# --- 3. PATH ---------------------------------------------------------------------------

# Homebrew/distro Rust (unlike rustup) and ~/.local/bin aren't always on PATH.
if [[ $(command -v ytm || true) != "$BIN" ]]; then
  bin_dir=$(dirname "$BIN")
  case ${SHELL##*/} in
    zsh) profile="$HOME/.zshrc" ;;
    bash) [[ $OS == Darwin ]] && profile="$HOME/.bash_profile" || profile="$HOME/.bashrc" ;;
    *) profile="$HOME/.profile" ;;
  esac
  line="export PATH=\"$bin_dir:\$PATH\""
  portable="\$HOME${bin_dir#"$HOME"}"
  if grep -qsF "$line" "$profile" || grep -qsF "$portable" "$profile"; then
    info "PATH is set in $profile; open a new terminal to use 'ytm'"
  elif ask "Add $bin_dir to PATH in $profile?" y; then
    printf '\n# ytm-player: bin dir\n%s\n' "$line" >>"$profile"
    info "added; open a new terminal (or run: source $profile)"
  else
    warn "$bin_dir is not on PATH; add to your shell profile: $line"
  fi
  export PATH="$bin_dir:$PATH"
fi

# --- 4. Credential store (Linux) --------------------------------------------------

if [[ $OS == Linux ]]; then
  say "Credential store"
  if command -v busctl >/dev/null && busctl --user list 2>/dev/null | grep -q org.freedesktop.secrets; then
    info "Secret Service: available"
  else
    warn "no Secret Service found: sign-in can't be saved."
    info "Install and unlock one, e.g. gnome-keyring (GNOME/i3/sway) or enable KWallet (KDE)."
  fi
fi

# --- 5. Google OAuth client ---------------------------------------------------------

say "Configuration"
CONFIG=$("$BIN" config)
info "config file: $CONFIG"

if grep -q 'OAuth client: missing' <<<"$("$BIN" status)"; then
  info "No Google OAuth client yet (one-time setup, README \"Set up Google sign-in\")."
  # Google's "Download JSON" lands here as client_secret_<id>.json.
  json=""
  for f in "$HOME"/Downloads/client_secret_*.json; do
    [[ -f $f && ( -z $json || $f -nt $json ) ]] && json=$f
  done
  if [[ -n $json ]] && ask "Import $(basename "$json") from Downloads?" y; then
    "$BIN" import-client "$json"
  elif interactive && ask "Paste client ID and secret now?" y; then
    read -r -p "    client_id: " client_id </dev/tty
    read -r -p "    client_secret: " client_secret </dev/tty
    tmp=$(mktemp)
    printf '{"installed":{"client_id":"%s","client_secret":"%s"}}' \
      "${client_id//\"/}" "${client_secret//\"/}" >"$tmp"
    "$BIN" import-client "$tmp" || warn "not saved; fix and retry with: ytm import-client <file.json>"
    rm -f "$tmp"
  else
    info "Later: ytm import-client ~/Downloads/client_secret_….json, then: ytm login"
  fi
else
  info "OAuth client: configured"
fi

# --- 6. Sign in -----------------------------------------------------------------------

status=$("$BIN" status)
if grep -q 'signed in:  yes' <<<"$status"; then
  info "Google account: signed in"
elif grep -q 'OAuth client: set' <<<"$status" && interactive; then
  say "Sign in"
  if [[ -n ${SSH_CONNECTION:-} ]]; then
    warn "over SSH the browser redirect can't reach this machine;"
    info "use 'ytm login --device' (needs a TV-type client, see README) or log in at the desktop."
  fi
  if ask "Sign in with Google now (opens the browser)?" y; then
    "$BIN" login || warn "sign-in failed; retry later with: ytm login"
  fi
fi

# --- 7. Done -------------------------------------------------------------------------

say "Done"
"$BIN" status | sed 's/^/    /'
echo
info "Start:        ytm"
info "Test audio:   ytm play dQw4w9WgXcQ"
info "Update:       git pull && ./install.sh"

if ((LAUNCH)) && interactive && [[ -t 1 ]] && ask "Start ytm now?" y; then
  exec "$BIN"
fi
