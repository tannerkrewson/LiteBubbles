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

The architecture and protocol boundaries are documented in `docs/`. Reference
repositories are checked out outside this repository and are never vendored
into it.

## Status

This repository is under active foundational development. Capability claims
will only be made after the corresponding pinned `rustpush` behavior is
implemented and tested.
