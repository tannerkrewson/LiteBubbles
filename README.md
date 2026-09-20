# LiteBubbles

LiteBubbles is a native GTK4/libadwaita GNOME client for Apple messaging and
services. It uses the pinned `OpenBubbles/rustpush` implementation through a
long-running `litebubblesd` backend and keeps the GTK application independent
from protocol details.

The project targets Fedora 44 Silverblue. Development dependencies belong in a
dedicated Fedora Toolbx; the Silverblue host is not modified with
`rpm-ostree`.

## Development

Create the project Toolbx and install its dependencies with:

```sh
./scripts/bootstrap-toolbox.sh
```

Enter it and build the workspace:

```sh
toolbox enter litebubbles-dev
cargo build --workspace
cargo test --workspace
```

The application can be started with `cargo run -p litebubbles`; the daemon
prints its command-line help with `cargo run -p litebubblesd -- --help`.

The shell uses the user-session D-Bus daemon transport by default and performs
the handshake asynchronously. For development or UI tests that need the
synthetic conversation fixture, opt in explicitly:

```sh
LITEBUBBLES_TRANSPORT=mock cargo run -p litebubbles
```

Only the exact value `mock` selects fixture mode; unset or other values keep
the production D-Bus path. Fixture mode is synthetic and credential-free.

The architecture and protocol boundaries are documented in `docs/`. Reference
repositories are checked out outside this repository and are never vendored
into it.

## Local user installation

Build release binaries inside the Toolbx, then run the installer as the regular
user on the Silverblue host. The host is not changed with `rpm-ostree`.

```sh
toolbox enter litebubbles-dev
cargo build --workspace --release
exit
./scripts/install-user.sh
systemctl --user status litebubblesd.service
```

The installer places `litebubbles` and `litebubblesd` in
`$HOME/.local/bin`, installs the desktop entry and D-Bus activation file below
`$XDG_DATA_HOME` (or `$HOME/.local/share`), and installs the user unit below
`$XDG_CONFIG_HOME/systemd/user` (or `$HOME/.config/systemd/user`). It enables
the daemon for the user session so it is independent of the application
window's lifetime. Use `./scripts/install-user.sh --no-start` when inspecting
the generated files without starting the service.

Remove only the files installed by this workflow with:

```sh
./scripts/uninstall-user.sh
```

The D-Bus service is `io.github.tannerkrewson.LiteBubbles.Backend`, and its
object/interface contract is defined in the protocol crate. No OpenBubbles
data, import, or host package is needed by these scripts.

## Status

This repository is under active foundational development. Capability claims
will only be made after the corresponding pinned `rustpush` behavior is
implemented and tested.

LB-011 is not yet acceptance-complete: it depends on the live backend adapter.
The backend-independent LB-010 service boundary now owns the versioned D-Bus
name and storage-backed state, while production refresh/send operations remain
explicitly unavailable until LB-050 unblocks rustpush. LB-008 records the
exact rustpush build blocker: the audited upstream revision references ten
missing FairPlay certificate/key pairs under `certs/fairplay/`.
