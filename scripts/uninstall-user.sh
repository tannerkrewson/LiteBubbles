#!/usr/bin/env bash
set -euo pipefail

readonly application_id='io.github.tannerkrewson.LiteBubbles'
readonly backend_bus_name='io.github.tannerkrewson.LiteBubbles.Backend'
readonly service_name='litebubblesd.service'

usage() {
    cat <<'EOF'
Usage: scripts/uninstall-user.sh

Remove the current user's LiteBubbles binaries, desktop entry, D-Bus
activation file, and systemd user unit. No host packages are changed.
EOF
}

die() {
    printf 'uninstall-user: %s\n' "$1" >&2
    exit 1
}

warn() {
    printf 'uninstall-user: warning: %s\n' "$1" >&2
}

[[ ${EUID} -ne 0 ]] || die 'run this as the target user, not as root'
[[ -n ${HOME:-} && ${HOME} == /* ]] || die 'HOME must be set to an absolute path'
[[ ${HOME} != *$'\n'* && ${HOME} != *$'\r'* ]] || die 'HOME cannot contain newlines'

if (($# > 0)); then
    case $1 in
        -h|--help)
            (($# == 1)) || die 'unexpected arguments after --help'
            usage
            exit 0
            ;;
        *)
            die "unknown option: $1"
            ;;
    esac
fi

data_home=${XDG_DATA_HOME:-${HOME}/.local/share}
config_home=${XDG_CONFIG_HOME:-${HOME}/.config}
for xdg_path in "${data_home}" "${config_home}"; do
    [[ ${xdg_path} == /* ]] || die 'XDG_DATA_HOME and XDG_CONFIG_HOME must be absolute paths'
    [[ ${xdg_path} != *$'\n'* && ${xdg_path} != *$'\r'* ]] || die 'XDG paths cannot contain newlines'
done

paths=(
    "${HOME}/.local/bin/litebubbles"
    "${HOME}/.local/bin/litebubblesd"
    "${HOME}/.local/bin/litebubbles-validation-component"
    "${HOME}/.local/bin/litebubbles-validation-helper"
    "${HOME}/.local/bin/litebubbles-setup-production"
    "${data_home}/applications/${application_id}.desktop"
    "${data_home}/dbus-1/services/${backend_bus_name}.service"
    "${config_home}/systemd/user/${service_name}"
)

systemd_failed=0
if command -v systemctl >/dev/null 2>&1; then
    if ! systemctl --user disable --now "${service_name}"; then
        warn "could not stop or disable ${service_name}; continuing with file removal"
        systemd_failed=1
    fi
    if ! systemctl --user daemon-reload; then
        warn 'could not reload the user systemd manager'
        systemd_failed=1
    fi
else
    warn 'systemctl is unavailable; the user service was not stopped'
    systemd_failed=1
fi

for path in "${paths[@]}"; do
    if [[ -d ${path} && ! -L ${path} ]]; then
        die "refusing to remove directory at expected file path: ${path}"
    fi
    rm -f -- "${path}"
done

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "${data_home}/applications" >/dev/null 2>&1 || \
        warn 'could not refresh the desktop entry cache'
fi

printf 'Removed LiteBubbles user files.\n'
if ((systemd_failed)); then
    exit 1
fi
