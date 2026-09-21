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
./scripts/prepare-rustpush.sh
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
./scripts/prepare-rustpush.sh
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
window's lifetime. It also installs the validation setup command and its
isolated helper; the production compatibility component itself is never part
of the build or repository. Use `./scripts/install-user.sh --no-start` when
inspecting the generated files without starting the service.

### Apple production validation

Public builds and CI do not need FairPlay files, Apple credentials, or a Mac.
They cannot perform real Apple activation until a production compatibility
component is installed. If you have obtained a compatible official
OpenBubbles release archive yourself, install and verify only its required
component with:

```sh
./scripts/setup-production.sh install \
  /path/to/official/bluebubbles-linux-x86_64.tar
./scripts/setup-production.sh status
```

The user installer also installs this wrapper as
`~/.local/bin/litebubbles-setup-production`. Both forms delegate to the
hash-validating `litebubbles-validation-component` command.

The command verifies the supported version and SHA-256, installs the opaque
library in the user data directory, and derives the FairPlay signer material
there. The private files are mode-restricted, never printed, and never placed
in the source checkout. Automatic download is not enabled because LiteBubbles
does not redistribute that third-party artifact. Genuine Mac activation
information is entered separately with:

```sh
litebubblesd setup --account 'you@example.com' \
  --hardware-file /path/to/mac-hardware-info-payload.txt
```

The command prompts for the Apple password and any required two-factor code.
The payload is stored in Secret Service storage, not SQLite, logs, D-Bus, or
the repository. See [`docs/validation-provider.md`](docs/validation-provider.md)
and [`docs/hardware-input.md`](docs/hardware-input.md).

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

The backend-independent LB-010 service boundary owns the versioned D-Bus name
and storage-backed state. Public rustpush compilation and the user-local
production signer/provider are implemented by the audited preparation path
documented in [`docs/lb-060-public-build.md`](docs/lb-060-public-build.md) and
[`docs/validation-provider.md`](docs/validation-provider.md). Real Apple
service activation remains an interactive, user-supplied smoke test; this
checkout has no Apple credentials, activation payload, or proprietary release
artifact with which to claim that test.
