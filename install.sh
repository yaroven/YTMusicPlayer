#!/usr/bin/env bash
# ytm-player installer for Linux and macOS.
# Installs build dependencies and Rust if missing, builds and installs `ytm`,
# helps with the Google OAuth client and sign-in, then starts the player.
# Safe to re-run: use it again after `git pull` to update.
set -euo pipefail

YES=0
DEPS=1
LAUNCH=1

usage() {
  cat <<'EOF'
Usage: ./install.sh [options]

  -y, --yes      don't ask; accept defaults (never starts the TUI)
  --no-deps      skip system package installation
  --no-launch    don't start ytm at the end
  -h, --help     show this help
EOF
}

while (($#)); do
  case $1 in
    -y | --yes) YES=1 ;;
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
[[ $OS == Linux || $OS == Darwin ]] || die "unsupported OS: $OS (on Windows see README)"

# --- 1. System dependencies -----------------------------------------------------

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

# --- 2. Rust ----------------------------------------------------------------------

say "Rust toolchain"
CARGO_BIN="${CARGO_HOME:-$HOME/.cargo}/bin"
if ! command -v cargo >/dev/null && [[ -x $CARGO_BIN/cargo ]]; then
  export PATH="$CARGO_BIN:$PATH"
fi
if ! command -v cargo >/dev/null; then
  ask "Rust is not installed. Install it with rustup (https://rustup.rs)?" y ||
    die "Rust is required"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
  export PATH="$CARGO_BIN:$PATH"
fi

# Cargo.toml requires 1.85+.
rust_minor=$(rustc --version | awk '{split($2, v, "."); print v[2]}')
if ((rust_minor < 85)); then
  if command -v rustup >/dev/null; then
    run rustup update stable
  else
    die "Rust $(rustc --version | awk '{print $2}') is too old; 1.85+ required (distro Rust? install via rustup)"
  fi
fi
info "$(rustc --version)"

# --- 3. Build and install --------------------------------------------------------

say "Building ytm (release, takes a few minutes the first time)"
run cargo install --path . --locked
BIN="${CARGO_INSTALL_ROOT:-${CARGO_HOME:-$HOME/.cargo}}/bin/ytm"
[[ -x $BIN ]] || die "build finished but $BIN is missing"
info "installed: $BIN"
# Homebrew/distro Rust (unlike rustup) doesn't put ~/.cargo/bin on PATH.
if [[ $(command -v ytm || true) != "$BIN" ]]; then
  bin_dir=$(dirname "$BIN")
  case ${SHELL##*/} in
    zsh) profile="$HOME/.zshrc" ;;
    bash) [[ $OS == Darwin ]] && profile="$HOME/.bash_profile" || profile="$HOME/.bashrc" ;;
    *) profile="$HOME/.profile" ;;
  esac
  line="export PATH=\"$bin_dir:\$PATH\""
  # Also recognise the portable "$HOME/.cargo/bin" spelling.
  if grep -qsF "$line" "$profile" ||
    { [[ $bin_dir == "$HOME/.cargo/bin" ]] && grep -qsF "\$HOME/.cargo/bin" "$profile"; }; then
    info "PATH is set in $profile; open a new terminal to use 'ytm'"
  elif ask "Add $bin_dir to PATH in $profile?" y; then
    printf '\n# ytm-player: cargo bin\n%s\n' "$line" >>"$profile"
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
status=$("$BIN" status)

valid_value() { [[ $1 =~ ^[A-Za-z0-9._-]+$ ]]; }

if grep -q 'OAuth client: missing' <<<"$status"; then
  info "No Google OAuth client yet. One-time setup (~5 min), README section"
  info "\"Set up Google sign-in\": create a Desktop-app client, then paste it here."
  if interactive && ask "Enter client ID and secret now?" y; then
    read -r -p "    client_id: " client_id </dev/tty
    read -r -p "    client_secret: " client_secret </dev/tty
    if ! valid_value "$client_id" || ! valid_value "$client_secret"; then
      die "unexpected characters in client_id/client_secret"
    fi
    tmp=$(mktemp)
    awk -v id="$client_id" -v secret="$client_secret" '
      /^client_id[[:space:]]*=/     { print "client_id = \"" id "\""; next }
      /^client_secret[[:space:]]*=/ { print "client_secret = \"" secret "\""; next }
      { print }' "$CONFIG" >"$tmp"
    mv "$tmp" "$CONFIG"
    chmod 600 "$CONFIG"
    info "saved to $CONFIG"
  else
    info "Later: edit $CONFIG, then run: ytm login"
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
    warn "over SSH the browser redirect goes to 127.0.0.1 on the machine running the browser;"
    info "easiest is to run 'ytm login' at this computer's own desktop."
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
