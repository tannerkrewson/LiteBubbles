# LB-010 daemon service boundary

LB-010's backend-independent service boundary is implemented in
`crates/daemon`. `ServiceState` owns the async-safe protocol state and the
SQLite store; `DbusService` exports the existing version-1 zbus identifiers.
Account, conversation, history, settings, refresh, and attachment metadata
operations are available without GTK. Attachments cross D-Bus only as
metadata references; bodies remain in daemon-owned storage.

The long-running D-Bus daemon still uses `UnavailableBackend` for the general
GUI service until the stored Apple session is promoted into the full sync
adapter. The production setup/send/listen commands in `litebubblesd` already
use rustpush directly, the backend-only validation provider, and Secret
Service state; they do not pretend to be complete GUI synchronization. Public
rustpush compilation is no longer the blocker: the preparation patch and
evidence are in [`docs/lb-008-blocker.md`](lb-008-blocker.md).

The daemon's tests load only the synthetic, credential-free mock fixture into
an in-memory SQLite database. They do not read OpenBubbles data or require
credentials. Refresh failures and unavailable send operations remain explicit;
this work does not claim full LB-010 acceptance or rustpush parity.
