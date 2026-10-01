#!/usr/bin/env bash
# ytm-player uninstaller for Linux and macOS. Removes every kind of install:
# .deb / .rpm / Arch packages, the macOS .pkg (/Applications/ytm-player.app
# and /usr/local/bin/ytm), and what install.sh or `cargo install` created
# (binaries, ~/Applications/ytm-player.app, the app menu entry, the PATH
# line). Asks before deleting your library, settings, logs and sign-in.
#
#   ./uninstall.sh [--purge | --keep-data] [-y]
#   ytm uninstall [--purge | --keep-data] [-y]   (same script, built in)
#
# Must stay compatible with macOS bash 3.2.
set -euo pipefail

# `ytm uninstall` runs a temporary copy of this script.
if [[ -n ${YTM_UNINSTALL_TMP:-} && ${YTM_UNINSTALL_TMP} == "$0" ]]; then
  trap 'rm -f "$0"' EXIT
fi

PURGE=ask
YES=0
for arg in "$@"; do
  case $arg in
    --purge) PURGE=yes ;;
    --keep-data) PURGE=no ;;
    -y | --yes) YES=1 ;;
    -h | --help)
      sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "unknown option: $arg (try --help)" >&2
      exit 2
      ;;
  esac
done

say() { printf '\033[1;31m==>\033[0m \033[1m%s\033[0m\n' "$*"; }
info() { printf '    %s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }

interactive() { ((!YES)) && [[ -r /dev/tty ]]; }
# ask "question" [y|n]; without a terminal (or with -y) the default wins.
ask() {
  local default=${2:-y} hint reply
  if ! interactive; then
    [[ $default == y ]]
    return
  fi
  [[ $default == y ]] && hint="[Y/n]" || hint="[y/N]"
  read -r -p "    $1 $hint " reply </dev/tty || reply=""
  reply=${reply:-$default}
  [[ $reply =~ ^[Yy] ]]
}

SUDO=()
((EUID == 0)) || SUDO=(sudo)
# Runs a command as root (asks for the password once, via sudo).
as_root() {
  printf '    $ %s\n' "$*"
  ${SUDO[@]+"${SUDO[@]}"} "$@"
}
# Removes a path, with sudo when the parent directory isn't writable.
remove() {
  local path=$1
  [[ -e $path || -L $path ]] || return 0
  if [[ -w $(dirname "$path") ]]; then
    rm -rf "$path"
  else
    as_root rm -rf "$path"
  fi
  info "removed $path"
}

OS=$(uname -s)
is_ytm() { [[ -x $1 ]] && "$1" --help 2>/dev/null | grep -q 'lightweight YouTube Music player'; }

# --- 1. What is installed -------------------------------------------------------

BINS=()
add_bin() {
  local b=$1 seen
  is_ytm "$b" || return 0
  for seen in ${BINS[@]+"${BINS[@]}"}; do [[ $seen == "$b" ]] && return 0; done
  BINS+=("$b")
}
while IFS= read -r b; do add_bin "$b"; done < <(type -ap ytm 2>/dev/null || true)
for b in "${CARGO_INSTALL_ROOT:-${CARGO_HOME:-$HOME/.cargo}}/bin/ytm" "$HOME/.local/bin/ytm" \
  /usr/local/bin/ytm /usr/bin/ytm \
  "$HOME/Applications/ytm-player.app/Contents/MacOS/ytm-player" \
  /Applications/ytm-player.app/Contents/MacOS/ytm-player; do
  add_bin "$b"
done

say "Uninstalling ytm-player"
if ((${#BINS[@]} == 0)); then
  info "no ytm binary found (already removed?)"
else
  for b in "${BINS[@]}"; do info "found: $b"; done
fi

# --- 2. Stop a running player -----------------------------------------------------

pkill -x ytm 2>/dev/null || true
pkill -x ytm-player 2>/dev/null || true

# --- 3. User data (before the binaries: `ytm logout` clears the keyring) --------

case $OS in
  Darwin)
    DATA_DIRS=("$HOME/Library/Application Support/dev.ytm-player.ytm-player"
      "$HOME/Library/Caches/dev.ytm-player.ytm-player")
    ;;
  *)
    DATA_DIRS=("${XDG_CONFIG_HOME:-$HOME/.config}/ytm-player"
      "${XDG_DATA_HOME:-$HOME/.local/share}/ytm-player"
      "${XDG_CACHE_HOME:-$HOME/.cache}/ytm-player")
    ;;
esac

if [[ $PURGE == ask ]]; then
  if ask "Also delete your library, settings, logs and Google sign-in?" n; then
    PURGE=yes
  else
    PURGE=no
  fi
fi
if [[ $PURGE == yes ]]; then
  if ((${#BINS[@]})); then
    if "${BINS[0]}" logout >/dev/null 2>&1; then info "removed the stored Google sign-in"; fi
  fi
  for d in "${DATA_DIRS[@]}"; do remove "$d"; done
else
  info "keeping your data: ${DATA_DIRS[*]}"
fi

# --- 4. System packages -----------------------------------------------------------

if [[ $OS == Linux && -e /usr/bin/ytm ]]; then
  if command -v dpkg >/dev/null && dpkg -S /usr/bin/ytm >/dev/null 2>&1; then
    as_root apt-get remove -y ytm-player || as_root dpkg -r ytm-player
  elif command -v pacman >/dev/null && pacman -Qo /usr/bin/ytm >/dev/null 2>&1; then
    as_root pacman -R --noconfirm ytm-player
  elif command -v rpm >/dev/null && rpm -qf /usr/bin/ytm >/dev/null 2>&1; then
    if command -v dnf >/dev/null; then as_root dnf remove -y ytm-player; else as_root rpm -e ytm-player; fi
  fi
fi

LSREGISTER=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
if [[ $OS == Darwin ]]; then
  if pkgutil --pkg-info dev.ytm-player.pkg >/dev/null 2>&1 || [[ -d /Applications/ytm-player.app ]]; then
    "$LSREGISTER" -u /Applications/ytm-player.app >/dev/null 2>&1 || true
    remove /Applications/ytm-player.app
    if [[ -L /usr/local/bin/ytm ]]; then remove /usr/local/bin/ytm; fi
    if pkgutil --pkg-info dev.ytm-player.pkg >/dev/null 2>&1; then
      as_root pkgutil --forget dev.ytm-player.pkg >/dev/null
    fi
  fi
fi

# --- 5. install.sh / cargo installs --------------------------------------------------

cargo_bin="${CARGO_INSTALL_ROOT:-${CARGO_HOME:-$HOME/.cargo}}/bin/ytm"
for b in ${BINS[@]+"${BINS[@]}"}; do
  case $b in
    *.app/Contents/MacOS/* | /usr/bin/ytm) ;; # apps and packages: above / below
    "$cargo_bin")
      # Also forgets it in cargo's install list.
      if command -v cargo >/dev/null && cargo uninstall ytm-player >/dev/null 2>&1; then
        info "removed $b (cargo uninstall)"
      else
        remove "$b"
      fi
      ;;
    *) remove "$b" ;;
  esac
done
if [[ $OS == Darwin ]]; then
  "$LSREGISTER" -u "$HOME/Applications/ytm-player.app" >/dev/null 2>&1 || true
  remove "$HOME/Applications/ytm-player.app"
else
  share="${XDG_DATA_HOME:-$HOME/.local/share}"
  remove "$share/applications/ytm-player.desktop"
  remove "$share/icons/hicolor/256x256/apps/ytm-player.png"
  update-desktop-database "$share/applications" >/dev/null 2>&1 || true
fi

# install.sh's PATH line ("# ytm-player: bin dir" and the export after it).
for profile in "$HOME/.zshrc" "$HOME/.bashrc" "$HOME/.bash_profile" "$HOME/.profile"; do
  if [[ ! -f $profile ]] || ! grep -q '^# ytm-player: bin dir$' "$profile"; then continue; fi
  tmp=$(mktemp)
  awk 'skip { skip = 0; next } /^# ytm-player: bin dir$/ { skip = 1; next } { print }' "$profile" >"$tmp"
  cat "$tmp" >"$profile" && rm -f "$tmp"
  info "removed the PATH line from $profile"
done

say "Done"
if command -v ytm >/dev/null 2>&1 && is_ytm "$(command -v ytm)"; then
  warn "still found: $(command -v ytm) — remove it by hand"
fi
info "ytm-player is uninstalled."
