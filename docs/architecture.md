# Architecture

LiteBubbles is a Cargo workspace with independent domain, protocol, storage,
backend, daemon, and GTK application crates. `litebubblesd` owns persistence,
authentication/session state, synchronization, push connectivity, and D-Bus;
`litebubbles` renders state and sends commands through the versioned protocol.

The `rustpush-backend` crate is the only planned direct consumer of rustpush
types. The core and UI-facing crates remain independent of that implementation.
