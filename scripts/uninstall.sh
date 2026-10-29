#!/usr/bin/env bash
# Frayd uninstaller (Linux/macOS) — removes the server, service, and (optionally) your media library.
# Usage: ./uninstall.sh [--purge-library] [--purge-deps] [--yes]
set -u
PURGE_LIB=0; PURGE_DEPS=0; ASSUME_YES=0
for a in "$@"; do
  case "$a" in
    --purge-library) PURGE_LIB=1;;
    --purge-deps) PURGE_DEPS=1;;
    --yes|-y) ASSUME_YES=1;;
    *) echo "unknown arg: $a"; exit 1;;
  esac
done
have() { command -v "$1" >/dev/null 2>&1; }
confirm() { [ "$ASSUME_YES" = 1 ] && return 0; read -r -p "$1 [y/N] " a; [ "$a" = "y" ] || [ "$a" = "Y" ]; }

echo "== Frayd uninstaller =="

# --- stop & remove service ---
if have systemctl && systemctl --user list-unit-files 2>/dev/null | grep -q frayd; then
  echo "[*] Stopping systemd user service..."
  systemctl --user disable --now frayd.service 2>/dev/null || true
  rm -f "$HOME/.config/systemd/user/frayd.service"
  systemctl --user daemon-reload
  echo "[OK] service removed"
fi
if have launchctl; then
  PL="$HOME/Library/LaunchAgents/io.github.kattiest.frayd.plist"
  if [ -f "$PL" ]; then
    launchctl unload "$PL" 2>/dev/null || true
    rm -f "$PL"
    echo "[OK] LaunchAgent removed"
  fi
fi
# stray processes
pkill -x frayd-server 2>/dev/null && echo "[OK] stopped running frayd-server" || true

# --- remove source checkout + binary ---
if [ -d "$HOME/Frayd" ]; then
  if confirm "Remove ~/Frayd (source checkout, ~1GB with build artifacts)?"; then
    rm -rf "$HOME/Frayd"
    echo "[OK] ~/Frayd removed"
  fi
fi

# --- remove env file if outside checkout ---
rm -f "$HOME/Frayd/server/.env" 2>/dev/null || true

# --- media library ---
if [ "$PURGE_LIB" = 1 ]; then
  LIB="${FRAYD_LIBRARY:-$HOME/Media}"
  if confirm "DELETE media library at $LIB? This cannot be undone."; then
    rm -rf "$LIB" && echo "[OK] library deleted"
  else
    echo "[i] library kept at $LIB"
  fi
else
  echo "[i] media library untouched (pass --purge-library to delete; default ~/Media)"
fi

# --- optionally remove toolchain deps installed by bootstrap ---
if [ "$PURGE_DEPS" = 1 ]; then
  if confirm "Remove Rust toolchain (~/.cargo, ~/.rustup)? Other projects may use it."; then
    rm -rf "$HOME/.cargo" "$HOME/.rustup"
    echo "[OK] Rust removed"
  fi
fi

echo
echo "[DONE] Frayd removed. Nothing was uploaded or sent anywhere."
