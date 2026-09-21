# LB-012 credential and session-state boundary

The storage crate now owns a backend-independent boundary for credentials and
ordinary local state:

- Apple credentials and backend tokens are represented by `SecretValue` and
  stored through `SecretStore`, never in SQLite or plaintext files.
- Production uses GNOME Secret Service over the workspace's pinned `zbus`
  5.12 D-Bus stack; tests use an in-memory synthetic implementation.
- `AppPaths` separates XDG data, cache, and config directories. The daemon's
  default database is `$XDG_DATA_HOME/litebubbles/litebubbles.sqlite3`, or the
  `$HOME/.local/share/litebubbles/litebubbles.sqlite3` fallback. An explicit
  database CLI path remains unchanged.
- `SessionState::sign_out` deletes the selected Secret Service item and purges
  app-owned data/cache state while retaining config preferences.
- `Diagnostic`, `SecretKey`, `SecretValue`, and secret-store errors are
  value-free and safe for structured diagnostics.

The production CLI path now initializes rustpush's encrypted local keystore
and stores its Apple session snapshot and genuine-Mac hardware payload through
the same Secret Service boundary. The general D-Bus synchronization adapter is
still a separate follow-up; the public build and production signer are no
longer blocked by missing source-tree FairPlay files.
