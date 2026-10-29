#!/usr/bin/env bash
# Invoke from an extracted bundle or installed prefix; unknown legacy installs are refused.
set -euo pipefail
FRAYD_BUNDLE=$(cd -- "$(dirname -- "$0")" && pwd -P)
if [[ ! -x "$FRAYD_BUNDLE/frayd-native" ]]; then
  printf '%s\n' 'Use uninstall.sh from a qualified extracted native bundle or installed prefix.' >&2
  exit 2
fi
exec "$FRAYD_BUNDLE/frayd-native" uninstall "$@"
