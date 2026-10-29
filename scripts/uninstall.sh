#!/bin/sh
# Temporary default-main guard until qualified PR2 native removal is merged.
# Source checkouts have no installer ownership receipt; global cleanup is unsupported.
printf '%s\n' \
  'Removal refused: this source checkout is not installer-owned.' \
  'Use the qualified PR2 native bundle and its ownership-checked removal after acceptance.' \
  'All files and services are preserved; this command performs no removal.' >&2
exit 2
