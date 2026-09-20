# Reference repositories

This audit was performed on 2026-09-20. The checkouts are outside the
LiteBubbles tree at `/var/home/tannerkrewson/Projects/litebubbles-reference`.
The paths below are audit inputs only; no reference source, asset, resource, or
template is vendored by LiteBubbles.

## Pinned checkouts

| Project | Repository and checkout | Audited revision | License facts from that revision |
| --- | --- | --- | --- |
| rustpush | `https://github.com/OpenBubbles/rustpush` — `/var/home/tannerkrewson/Projects/litebubbles-reference/rustpush` — `master` | `f35c4ee062b3c3eae54dc96b89b90ee99f5e1d0c` | `LICENSE` is Server Side Public License, version 1 (SSPL-1.0). `LICENSE.exceptions` grants a special unrestricted-dealing exception to OpenBubbles; it does not name LiteBubbles. |
| openbubbles-app | `https://github.com/OpenBubbles/openbubbles-app` — `/var/home/tannerkrewson/Projects/litebubbles-reference/openbubbles-app` — `rustpush` | `eed1b6332efbb17adbf5ebfa2263ad770169f75e` | `LICENSE` is Apache License 2.0. The checkout also has a BlueBubbles `upstream` remote; the audited application source is the OpenBubbles `origin` checkout on `rustpush`. |
| Tether | `https://github.com/zackb/tether` — `/var/home/tannerkrewson/Projects/litebubbles-reference/tether` — `main` | `f4173de675f1463c02543fa62d4914115b337590` | `LICENSE` is the MIT License, copyright Zack Bartel 2026. GitHub has no `OpenBubbles/tether` repository; this is the Linux+iPhone Tether project whose daemon/UI patterns were requested. |
| Fractal | `https://gitlab.gnome.org/World/fractal.git` — `/var/home/tannerkrewson/Projects/litebubbles-reference/fractal` — `main` | `6c1adadb5d2bc47ceb1183be90e53e2eb8ba9928` | `Cargo.toml` declares `GPL-3.0-or-later`; `LICENSE` points to `LICENSES/GPL-3.0-or-later.txt`. The REUSE inventory also records file-specific Apache-2.0, AGPL-3.0-or-later, GPL-2.0-or-later, CC0-1.0, CC-BY-SA-4.0, and `LicenseRef-BSD-Mapbox` material. |

The rustpush checkout contains gitlinks for `open-absinthe`,
`third_party/apple-private-apis`, `third_party/quinn`, `third_party/rtc`,
`third_party/rustls`, and `third_party/rustls-platform-verifier`. They were not
initialized for this audit, and none is copied into LiteBubbles. A future
implementation pin must resolve those gitlinks reproducibly and audit their
licenses separately.

The rustpush license is a material project constraint: the OpenBubbles-only
exception is not a general grant to downstream projects. LiteBubbles must keep
the pinned license/exception notice, obtain any required legal review, and not
claim that the exception applies to LiteBubbles.

## What was inspected

### rustpush

The complete checked-in rustpush source tree was inventory-checked and its
public exports and service implementations were reviewed. The capability
surface is recorded in [rustpush-parity.md](rustpush-parity.md), including exact
names from this revision such as `OSConfig`, `activate`, `APSConnection`,
`IdentityManager`, `IMClient`, `CloudMessagesClient`, `CloudKitClient`,
`KeychainClient`, `SharedStreamClient`, `StatusKitClient`, `FTClient`,
`FindMyClient`, `RelayConfig`, and `PasswordManager`.

The most important source locations are:

- `src/lib.rs`, `src/activation.rs`, `src/auth.rs`, `src/aps.rs`, `src/relay.rs`
  for configuration, activation, authentication, tokens, APS, and relay
  boundaries.
- `src/ids/user.rs`, `src/ids/identity_manager.rs`,
  `src/imessage/aps_client.rs`, and `src/imessage/messages.rs` for identity,
  registration, send/receive, message parts, mutations, and scheduling.
- `src/imessage/cloud_messages.rs`, `src/icloud/mmcs.rs`,
  `src/icloud/cloudkit.rs`, `src/icloud/pcs.rs`, and
  `src/icloud/keychain.rs` for history, attachments, CloudKit, encryption, and
  iCloud Keychain state.
- `src/sharedstreams.rs`, `src/statuskit.rs`, `src/facetime.rs`,
  `src/avconference.rs`, `src/findmy.rs`,
  `src/imessage/name_photo_sharing.rs`, and `src/passwords.rs` for the
  additional service capabilities.

### openbubbles-app

The application was used only as a source reference for current integration
shape. The rustpush-facing wrapper is in `rust/src/api/api.rs`; the native
Android bridge and keystore boundary are in `rust/src/native.rs` and
`rust/src/keystore.rs`; the application orchestration is in
`lib/services/rustpush/rustpush_service.dart`; password UI is under
`lib/app/layouts/settings/pages/passwords/`.

The integration constructs long-lived service state in `SharedPushState`,
restores or initializes APS/Anisette/account/IDS/CloudKit/keychain clients,
dispatches APS events through `recv_wait`, and exposes narrow wrapper functions
such as `send`, `sync_chats`, `sync_messages`, `upload_attachment`,
`create_facetime`, `refresh_devices`, `get_albums`, `set_status`, and the
password CRUD/group/invite functions. LiteBubbles may use these observations to
define its adapter boundary, but does not copy the Dart, Rust, Kotlin, native
library, resources, or generated bindings.

### Fractal interaction patterns

Fractal was inspected for interaction patterns only. Useful observations from
the pinned checkout are:

- `src/application.rs` owns application actions, restores the session list
  asynchronously, and observes network state.
- `src/session/mod.rs` models session state as observable GTK properties while
  keeping sync and session-change task handles cancellable; network changes
  update reachability asynchronously.
- `src/session_list/mod.rs` exposes a GTK `gio::ListModel`, restores persisted
  sessions asynchronously, and represents loading/error/ready states.
- `src/components/offline_banner.rs`, `src/utils/toast.rs`, and `src/window.rs`
  provide explicit offline and transient-error presentation boundaries.
- `src/secret/mod.rs` and `src/secret/linux.rs` keep stored session material
  behind a secret-storage abstraction.

These patterns inform LiteBubbles' GTK state ownership, cancellation,
observable lists, offline states, and redacted error presentation. No Fractal
implementation, UI template, resource, icon, or asset was copied.

### Tether interaction patterns

Tether was inspected as a daemon/UI and local-IPC reference, not as an Apple
messaging implementation. `src/gtk/daemon_client.hpp` and
`src/gtk/daemon_client.cpp` show a GTK client with one local event subscription,
newline-delimited JSON commands, disconnect callbacks, and scheduled reconnect.
`src/gtk/main.cpp` shows a `GtkStack`-based application shell and action routing.
`docs/PROTOCOL.md` documents feature negotiation, explicit pairing state,
streamed file transfers, and daemon error responses. These are interaction and
failure-handling observations only; no Tether source, resource, or protocol is
part of LiteBubbles.

## Data-scope statement

The audit accessed only the four named source checkouts and the LiteBubbles
repository. It did not detect, inspect, print, migrate, modify, back up,
coexist with, or depend on any installed OpenBubbles user data or runtime data
paths. LiteBubbles has no OpenBubbles-data import path in this audit.
