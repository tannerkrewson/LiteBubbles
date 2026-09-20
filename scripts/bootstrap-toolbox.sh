#!/usr/bin/env bash
set -euo pipefail

toolbox_name="litebubbles-dev"

if ! command -v toolbox >/dev/null 2>&1; then
    printf '%s\n' 'toolbox is required on the Fedora Silverblue host.' >&2
    exit 1
fi

if ! toolbox list --containers | awk -v name="$toolbox_name" '$2 == name { found = 1 } END { exit !found }'; then
    toolbox --assumeyes create --release 44 "$toolbox_name"
fi

if ! command -v podman >/dev/null 2>&1; then
    printf '%s\n' 'podman is required to provision the Toolbx container.' >&2
    exit 1
fi

if [[ "$(podman inspect --format '{{.State.Running}}' "$toolbox_name")" != "true" ]]; then
    podman start "$toolbox_name" >/dev/null
fi

podman exec "$toolbox_name" sudo dnf install -y \
    gcc gcc-c++ clang pkgconf-pkg-config \
    gtk4-devel libadwaita-devel glib2-devel openssl-devel \
    sqlite-devel protobuf-compiler dbus-devel libsecret-devel \
    gstreamer1-devel gstreamer1-plugins-base-devel pipewire-devel \
    git gh rust cargo rustfmt clippy buildah make

printf 'Toolbx %s is ready. Enter it with: toolbox enter %s\n' "$toolbox_name" "$toolbox_name"
