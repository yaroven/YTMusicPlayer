#!/usr/bin/env bash
# Yarek Music uninstaller for Linux and macOS. Removes every kind of install:
# .deb / .rpm / Arch packages, the macOS .pkg (/Applications/Yarek Music.app
# and /usr/local/bin/yarek), and what install.sh or `cargo install` created
# (binaries, ~/Applications/Yarek Music.app, the app menu entry, the PATH
# line), also those of versions up to 1.0.0 (ytm-player, `ytm`). Asks
# before deleting your library, settings, logs and sign-in.
#
#   ./uninstall.sh [--purge | --keep-data] [-y]
#   yarek uninstall [--purge | --keep-data] [-y]   (same script, built in)
#
# Must stay compatible with macOS bash 3.2.
set -euo pipefail

# `yarek uninstall` runs a temporary copy of this script.
if [[ -n ${YAREK_UNINSTALL_TMP:-} && ${YAREK_UNINSTALL_TMP} == "$0" ]]; then
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
is_ours() { [[ -x $1 ]] && "$1" --help 2>/dev/null | grep -q 'lightweight YouTube Music player'; }

# --- 1. What is installed -------------------------------------------------------

BINS=()
add_bin() {
  local b=$1 seen
  is_ours "$b" || return 0
  for seen in ${BINS[@]+"${BINS[@]}"}; do [[ $seen == "$b" ]] && return 0; done
  BINS+=("$b")
}
while IFS= read -r b; do add_bin "$b"; done < <(type -ap yarek ytm 2>/dev/null || true)
for name in yarek ytm; do
  for b in "${CARGO_INSTALL_ROOT:-${CARGO_HOME:-$HOME/.cargo}}/bin/$name" "$HOME/.local/bin/$name" \
    "/usr/local/bin/$name" "/usr/bin/$name"; do
    add_bin "$b"
  done
done
for b in "$HOME/Applications/Yarek Music.app/Contents/MacOS/yarek-music" \
  "/Applications/Yarek Music.app/Contents/MacOS/yarek-music" \
  "$HOME/Applications/ytm-player.app/Contents/MacOS/ytm-player" \
  /Applications/ytm-player.app/Contents/MacOS/ytm-player; do
  add_bin "$b"
done

say "Uninstalling Yarek Music"
if ((${#BINS[@]} == 0)); then
  info "no yarek binary found (already removed?)"
else
  for b in "${BINS[@]}"; do info "found: $b"; done
fi

# --- 2. Stop a running player -----------------------------------------------------

for p in yarek yarek-music ytm ytm-player; do pkill -x "$p" 2>/dev/null || true; done

# --- 3. User data (before the binaries: `yarek logout` clears the keyring) --------

case $OS in
  Darwin)
    DATA_DIRS=("$HOME/Library/Application Support/dev.yarek.yarek-music"
      "$HOME/Library/Caches/dev.yarek.yarek-music"
      "$HOME/Library/Application Support/dev.ytm-player.ytm-player"
      "$HOME/Library/Caches/dev.ytm-player.ytm-player")
    ;;
  *)
    DATA_DIRS=()
    for name in yarek-music ytm-player; do
      DATA_DIRS+=("${XDG_CONFIG_HOME:-$HOME/.config}/$name"
        "${XDG_DATA_HOME:-$HOME/.local/share}/$name"
        "${XDG_CACHE_HOME:-$HOME/.cache}/$name")
    done
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

# The package and its binary, then the same under the old name.
for pair in yarek-music:yarek ytm-player:ytm; do
  pkg=${pair%%:*} bin=/usr/bin/${pair##*:}
  [[ $OS == Linux && -e $bin ]] || continue
  if command -v dpkg >/dev/null && dpkg -S "$bin" >/dev/null 2>&1; then
    as_root apt-get remove -y "$pkg" || as_root dpkg -r "$pkg"
  elif command -v pacman >/dev/null && pacman -Qo "$bin" >/dev/null 2>&1; then
    as_root pacman -R --noconfirm "$pkg"
  elif command -v rpm >/dev/null && rpm -qf "$bin" >/dev/null 2>&1; then
    if command -v dnf >/dev/null; then as_root dnf remove -y "$pkg"; else as_root rpm -e "$pkg"; fi
  fi
done

LSREGISTER=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
if [[ $OS == Darwin ]]; then
  for triple in "dev.yarek.music.pkg:/Applications/Yarek Music.app:/usr/local/bin/yarek" \
    "dev.ytm-player.pkg:/Applications/ytm-player.app:/usr/local/bin/ytm"; do
    pkg=${triple%%:*} rest=${triple#*:}
    app=${rest%%:*} link=${rest#*:}
    if pkgutil --pkg-info "$pkg" >/dev/null 2>&1 || [[ -d $app ]]; then
      "$LSREGISTER" -u "$app" >/dev/null 2>&1 || true
      remove "$app"
      if [[ -L $link ]]; then remove "$link"; fi
      if pkgutil --pkg-info "$pkg" >/dev/null 2>&1; then
        as_root pkgutil --forget "$pkg" >/dev/null
      fi
    fi
  done
fi

# --- 5. install.sh / cargo installs --------------------------------------------------

cargo_bins="${CARGO_INSTALL_ROOT:-${CARGO_HOME:-$HOME/.cargo}}/bin"
for b in ${BINS[@]+"${BINS[@]}"}; do
  case $b in
    *.app/Contents/MacOS/* | /usr/bin/yarek | /usr/bin/ytm) ;; # apps and packages: above
    "$cargo_bins/yarek" | "$cargo_bins/ytm")
      # Also forgets it in cargo's install list.
      if command -v cargo >/dev/null &&
        { cargo uninstall yarek-music >/dev/null 2>&1 || cargo uninstall ytm-player >/dev/null 2>&1; }; then
        info "removed $b (cargo uninstall)"
      else
        remove "$b"
      fi
      ;;
    *) remove "$b" ;;
  esac
done
if [[ $OS == Darwin ]]; then
  for app in "$HOME/Applications/Yarek Music.app" "$HOME/Applications/ytm-player.app"; do
    "$LSREGISTER" -u "$app" >/dev/null 2>&1 || true
    remove "$app"
  done
else
  share="${XDG_DATA_HOME:-$HOME/.local/share}"
  for name in yarek-music ytm-player; do
    remove "$share/applications/$name.desktop"
    remove "$share/icons/hicolor/256x256/apps/$name.png"
  done
  update-desktop-database "$share/applications" >/dev/null 2>&1 || true
fi

# install.sh's PATH line ("# Yarek Music: bin dir", up to 1.0.0 "# ytm-player:
# bin dir", and the export after it).
for profile in "$HOME/.zshrc" "$HOME/.bashrc" "$HOME/.bash_profile" "$HOME/.profile"; do
  if [[ ! -f $profile ]] || ! grep -qE '^# (Yarek Music|ytm-player): bin dir$' "$profile"; then continue; fi
  tmp=$(mktemp)
  awk 'skip { skip = 0; next } /^# (Yarek Music|ytm-player): bin dir$/ { skip = 1; next } { print }' "$profile" >"$tmp"
  cat "$tmp" >"$profile" && rm -f "$tmp"
  info "removed the PATH line from $profile"
done

say "Done"
if command -v yarek >/dev/null 2>&1 && is_ours "$(command -v yarek)"; then
  warn "still found: $(command -v yarek) — remove it by hand"
fi
info "Yarek Music is uninstalled."
