#!/usr/bin/env bash
set -euo pipefail

readonly application_id='io.github.tannerkrewson.LiteBubbles'
readonly backend_bus_name='io.github.tannerkrewson.LiteBubbles.Backend'
readonly service_name='litebubblesd.service'

usage() {
    cat <<'EOF'
Usage: scripts/install-user.sh [OPTIONS]

Install a release build into the current user's XDG directories.

Options:
  --build-dir DIR  Read litebubbles and litebubblesd from DIR
                   (default: target/release)
  --no-start       Install and enable the user service without starting it
  -h, --help       Show this help
EOF
}

die() {
    printf 'install-user: %s\n' "$1" >&2
    exit 1
}

warn() {
    printf 'install-user: warning: %s\n' "$1" >&2
}

[[ ${EUID} -ne 0 ]] || die 'run this as the target user, not as root'
[[ -n ${HOME:-} && ${HOME} == /* ]] || die 'HOME must be set to an absolute path'
[[ ${HOME} != *$'\n'* && ${HOME} != *$'\r'* ]] || die 'HOME cannot contain newlines'

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
repo_root=$(cd -- "${script_dir}/.." && pwd -P)
build_dir=${repo_root}/target/release
start_service=1

while (($# > 0)); do
    case $1 in
        --build-dir)
            (($# >= 2)) || die '--build-dir requires a directory'
            build_dir=$2
            shift 2
            ;;
        --no-start)
            start_service=0
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            die "unknown option: $1"
            ;;
    esac
done

if [[ ${build_dir} != /* ]]; then
    build_dir=${repo_root}/${build_dir}
fi
build_dir=$(cd -- "${build_dir}" && pwd -P) || die "build directory does not exist: ${build_dir}"

data_home=${XDG_DATA_HOME:-${HOME}/.local/share}
config_home=${XDG_CONFIG_HOME:-${HOME}/.config}
for xdg_path in "${data_home}" "${config_home}"; do
    [[ ${xdg_path} == /* ]] || die 'XDG_DATA_HOME and XDG_CONFIG_HOME must be absolute paths'
    [[ ${xdg_path} != *$'\n'* && ${xdg_path} != *$'\r'* ]] || die 'XDG paths cannot contain newlines'
done

app_source=${build_dir}/litebubbles
daemon_source=${build_dir}/litebubblesd
desktop_template=${repo_root}/data/io.github.tannerkrewson.LiteBubbles.desktop.in
dbus_template=${repo_root}/data/io.github.tannerkrewson.LiteBubbles.Backend.service.in
systemd_template=${repo_root}/data/litebubblesd.service

for executable in "${app_source}" "${daemon_source}"; do
    [[ -f ${executable} && -x ${executable} ]] || die "missing executable build artifact: ${executable}"
done
for metadata in "${desktop_template}" "${dbus_template}" "${systemd_template}"; do
    [[ -f ${metadata} ]] || die "missing packaging metadata: ${metadata}"
done

bin_dir=${HOME}/.local/bin
desktop_dir=${data_home}/applications
dbus_dir=${data_home}/dbus-1/services
systemd_dir=${config_home}/systemd/user
desktop_file=${desktop_dir}/${application_id}.desktop
dbus_file=${dbus_dir}/${backend_bus_name}.service
systemd_file=${systemd_dir}/${service_name}

for directory in "${bin_dir}" "${desktop_dir}" "${dbus_dir}" "${systemd_dir}"; do
    install -d -m 0755 -- "${directory}"
done

replace_file() {
    local source=$1
    local destination=$2
    local mode=$3
    local destination_dir tmp

    destination_dir=$(dirname -- "${destination}")
    tmp=$(mktemp "${destination_dir}/.litebubbles-install.XXXXXX")
    if ! install -m "${mode}" -- "${source}" "${tmp}"; then
        rm -f -- "${tmp}"
        die "could not prepare ${destination}"
    fi
    if ! mv -f -- "${tmp}" "${destination}"; then
        rm -f -- "${tmp}"
        die "could not install ${destination}"
    fi
}

render_file() {
    local template=$1
    local destination=$2
    local marker=$3
    local value=$4
    local destination_dir tmp escaped

    destination_dir=$(dirname -- "${destination}")
    tmp=$(mktemp "${destination_dir}/.litebubbles-install.XXXXXX")
    escaped=${value//\\/\\\\}
    escaped=${escaped//&/\\&}
    escaped=${escaped//|/\\|}
    if ! sed "s|${marker}|${escaped}|g" "${template}" >"${tmp}"; then
        rm -f -- "${tmp}"
        die "could not render ${destination}"
    fi
    chmod 0644 -- "${tmp}"
    if ! mv -f -- "${tmp}" "${destination}"; then
        rm -f -- "${tmp}"
        die "could not install ${destination}"
    fi
}

quote_exec_arg() {
    local value=$1
    value=${value//\\/\\\\}
    value=${value//\"/\\\"}
    printf '"%s"' "${value}"
}

app_exec=$(quote_exec_arg "${bin_dir}/litebubbles")
daemon_exec=$(quote_exec_arg "${bin_dir}/litebubblesd")

replace_file "${app_source}" "${bin_dir}/litebubbles" 0755
replace_file "${daemon_source}" "${bin_dir}/litebubblesd" 0755
render_file "${desktop_template}" "${desktop_file}" '@LITEBUBBLES_EXEC@' "${app_exec}"
render_file "${dbus_template}" "${dbus_file}" '@LITEBUBBLES_DAEMON_EXEC@' "${daemon_exec}"
replace_file "${systemd_template}" "${systemd_file}" 0644

if ((start_service)); then
    command -v systemctl >/dev/null 2>&1 || die 'systemctl is required unless --no-start is used'
    systemctl --user daemon-reload
    systemctl --user enable --now "${service_name}"
else
    if command -v systemctl >/dev/null 2>&1; then
        systemctl --user daemon-reload
        systemctl --user enable "${service_name}"
    else
        warn 'systemctl is unavailable; the unit was installed but not enabled'
    fi
fi

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "${data_home}/applications" >/dev/null 2>&1 || \
        warn 'could not refresh the desktop entry cache'
fi

printf 'Installed LiteBubbles for the current user.\n'
printf '  Application: %s\n' "${desktop_file}"
printf '  Daemon unit: %s\n' "${systemd_file}"
