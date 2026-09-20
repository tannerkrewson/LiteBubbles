#!/usr/bin/env bash
set -euo pipefail

readonly rustpush_revision='f35c4ee062b3c3eae54dc96b89b90ee99f5e1d0c'

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
repo_root=$(cd -- "${script_dir}/.." && pwd -P)
rustpush_dir=${repo_root}/vendor/rustpush
patch_file=${repo_root}/patches/rustpush/0001-public-build-no-source-tree-fairplay.patch

git -c url."https://github.com/".insteadOf='git@github.com:' \
    -c url."https://github.com/".insteadOf='ssh://git@github.com/' \
    submodule update --init --recursive -- vendor/rustpush

[[ -f ${rustpush_dir}/Cargo.toml ]] || {
    printf '%s\n' 'prepare-rustpush: vendor/rustpush is not initialized' >&2
    exit 1
}
[[ -f ${patch_file} ]] || {
    printf '%s\n' "prepare-rustpush: missing patch: ${patch_file}" >&2
    exit 1
}

actual_revision=$(git -C "${rustpush_dir}" rev-parse HEAD)
if [[ ${actual_revision} != "${rustpush_revision}" ]]; then
    printf 'prepare-rustpush: expected rustpush %s, found %s\n' \
        "${rustpush_revision}" "${actual_revision}" >&2
    exit 1
fi

if ! git -C "${rustpush_dir}" diff --quiet; then
    if git -C "${rustpush_dir}" apply --reverse --check "${patch_file}" >/dev/null 2>&1; then
        printf '%s\n' 'prepare-rustpush: public-build patch is already applied'
        exit 0
    fi
    printf '%s\n' 'prepare-rustpush: rustpush has unexpected local changes' >&2
    git -C "${rustpush_dir}" status --short >&2
    exit 1
fi

git -C "${rustpush_dir}" apply --check "${patch_file}"
git -C "${rustpush_dir}" apply "${patch_file}"
printf 'prepare-rustpush: patched pinned rustpush %s for public builds\n' \
    "${rustpush_revision}"
