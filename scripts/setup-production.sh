#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage:
  scripts/setup-production.sh install ARCHIVE [--data-home DIR]
  scripts/setup-production.sh status [--data-home DIR]

Install or check the user-supplied production validation component. ARCHIVE
must be a compatible official OpenBubbles release archive obtained by the user.
This script does not download artifacts or inspect an OpenBubbles installation.

The validation component supplies Mac validation data only. Real Apple
activation separately requires a production FairPlay device-activation signer;
the public LiteBubbles build does not provide or configure that signer.
EOF
}

die() {
    printf 'setup-production: %s\n' "$1" >&2
    exit 1
}

fairplay_notice() {
    cat <<'EOF'

FairPlay setup requirement:
  The validation component covers Mac validation-data generation only.
  Real Apple activation also requires the separate rustpush FairPlay
  device-activation signer. The public LiteBubbles build does not provide or
  configure that signer, and development dummy FairPlay is not valid for
  production. Production activation remains unavailable until that boundary
  is resolved (LB-064).
EOF
}

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
repo_root=$(cd -- "${script_dir}/.." && pwd -P)

find_component_command() {
    local candidate
    local candidates=(
        "${script_dir}/litebubbles-validation-component"
    )

    if [[ -n ${HOME:-} ]]; then
        candidates+=("${HOME}/.local/bin/litebubbles-validation-component")
    fi
    candidates+=("${repo_root}/target/release/litebubbles-validation-component")

    for candidate in "${candidates[@]}"; do
        if [[ -f ${candidate} && -x ${candidate} ]]; then
            printf '%s\n' "${candidate}"
            return 0
        fi
    done

    command -v litebubbles-validation-component 2>/dev/null || return 1
}

if (($# == 0)); then
    usage >&2
    exit 1
fi

case $1 in
    -h|--help)
        (($# == 1)) || die 'unexpected arguments after --help'
        usage
        exit 0
        ;;
    install)
        (($# >= 2)) || die 'install requires an official archive path'
        ;;
    status)
        ;;
    *)
        die "unknown command: $1"
        ;;
esac

component_command=$(find_component_command) || die \
    'litebubbles-validation-component is not installed; run scripts/install-user.sh first'

component_status=0
"${component_command}" "$@" || component_status=$?
fairplay_notice
exit "${component_status}"
