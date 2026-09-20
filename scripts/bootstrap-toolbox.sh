#!/usr/bin/env bash
set -euo pipefail

toolbox_name="litebubbles-dev"

if ! command -v toolbox >/dev/null 2>&1; then
    printf '%s\n' 'toolbox is required on the Fedora Silverblue host.' >&2
    exit 1
fi

if ! toolbox list --containers | awk '{print $1}' | grep -qx "$toolbox_name"; then
    toolbox --assumeyes create --release 44 "$toolbox_name"
fi

toolbox run --container "$toolbox_name" bash -lc '
    set -euo pipefail
    sudo dnf install -y \
        gcc gcc-c++ clang pkgconf-pkg-config \
        gtk4-devel libadwaita-devel glib2-devel openssl-devel \
        sqlite-devel protobuf-compiler dbus-devel libsecret-devel \
        gstreamer1-devel gstreamer1-plugins-base-devel pipewire-devel \
        git gh rust cargo rustfmt clippy buildah make
'

printf 'Toolbx %s is ready. Enter it with: toolbox enter %s\n' "$toolbox_name" "$toolbox_name"
