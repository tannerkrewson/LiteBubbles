# Architecture

LiteBubbles is a Cargo workspace with independent domain, protocol, storage,
backend, daemon, and GTK application crates. `litebubblesd` owns persistence,
authentication/session state, synchronization, push connectivity, and D-Bus;
`litebubbles` renders state and sends commands through the versioned protocol.

The `rustpush-backend` crate is the only planned direct consumer of rustpush
types. The core and UI-facing crates remain independent of that implementation.

The daemon's backend-independent service boundary lives in `crates/daemon`.
It owns SQLite-backed protocol state and exports the versioned D-Bus interface;
the `BackendAdapter` seam remains explicit while the live rustpush session is
being promoted into the general D-Bus synchronization service. See
[`lb-010-daemon-service.md`](lb-010-daemon-service.md).

Credential and ordinary session-state storage is a separate boundary in
`crates/storage`; see [`lb-012-credentials.md`](lb-012-credentials.md). It
does not expose rustpush objects or sensitive Apple session material to the
GTK-facing protocol.
