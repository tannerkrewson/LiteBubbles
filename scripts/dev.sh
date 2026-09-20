#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
"${script_dir}/prepare-rustpush.sh"

exec cargo run -p litebubbles -- "$@"
