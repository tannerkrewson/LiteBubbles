#!/usr/bin/env bash
set -euo pipefail

exec cargo run -p litebubbles -- "$@"
