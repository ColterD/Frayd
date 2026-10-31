#!/usr/bin/env bash
set -euo pipefail
FRAYD_BUNDLE=$(cd -- "$(dirname -- "$0")" && pwd -P)
if [[ ! -x "$FRAYD_BUNDLE/frayd-native" ]]; then
  printf '%s\n' 'Run this script from an extracted native bundle or installed prefix.' >&2
  exit 2
fi
exec "$FRAYD_BUNDLE/frayd-native" install --bundle "$FRAYD_BUNDLE" "$@"
