# Apple validation provider

LiteBubbles has two deliberately separate validation modes.

## Development and CI

The public rustpush checkout is prepared by `scripts/prepare-rustpush.sh`.
That preparation removes rustpush's compile-time dependency on unavailable
source-tree FairPlay files. The optional `development-dummy-fairplay` Cargo
feature is only a compile/mock-test switch; it does not create valid Apple
activation data and must not be presented as a production configuration.

CI uses no Apple credentials, private FairPlay material, Mac hardware input,
or OpenBubbles release component.

## Production validation data

The production validation boundary is in the
`litebubbles-validation-provider` crate. `OpenBubblesValidationProvider`
starts `litebubbles-validation-helper` for each two-phase validation request.
The helper is a separate short-lived process that loads the opaque
`openbubbles.so` component, calls the documented compatibility ABI, and sends
only bounded JSON-lines messages across its stdin/stdout pipe. A helper crash,
timeout, malformed response, unsupported version, or hash mismatch becomes a
safe provider error instead of taking down `litebubblesd`.

The provider validates the exact compatibility contract currently documented by
`openbubbles-build-modules`:

* OpenBubbles release: `v1.15.0+136`;
* library name: `openbubbles.so`;
* x86_64 Linux library SHA-256:
  `f47fbd299bf5c83449bf6485a2c00c0f059d0e059646e20c64111bc5fac84b2a`;
* the reference symbol, function offsets, and hardware-config wire shape are
  pinned in the provider source.

LiteBubbles does not contain or redistribute that library. During local setup,
the user-supplied, hash-validated library is also processed by the public
`fairplay-certs` algorithm from the same build-modules project. The ten
certificate/private-key pairs are written only below the user's private
compatibility directory and are used by rustpush's activation signer. They
are not committed, uploaded, printed, or exposed through GTK/D-Bus. The
library itself remains loaded only in a short-lived helper process for the
separate Mac validation-data exchange.

## Installing the local component

Automatic downloading is intentionally disabled. The current OpenBubbles
artifact distribution and applicable terms have not been established as a
redistribution grant for LiteBubbles, so the supported setup asks the user to
provide an official release archive they obtained themselves.

After building and installing LiteBubbles, run:

```sh
./scripts/setup-production.sh install \
  /path/to/official/bluebubbles-linux-x86_64.tar
./scripts/setup-production.sh status
```

`scripts/install-user.sh` also installs the wrapper as
`~/.local/bin/litebubbles-setup-production`. The wrapper invokes the existing
`litebubbles-validation-component` installer, which prepares both the opaque
library and the local FairPlay material.

The installer extracts only the one required `openbubbles.so`, verifies its
exact SHA-256, derives and validates the ten FairPlay pairs, writes a small
local manifest, and installs everything below:

```text
$XDG_DATA_HOME/litebubbles/compat/v1.15.0+136/
```

or `$HOME/.local/share/litebubbles/compat/v1.15.0+136/` when
`XDG_DATA_HOME` is unset. The archive is not copied into the repository and
the installed directory is private to the user. The component and derived
FairPlay material can be removed with:

```sh
~/.local/bin/litebubbles-validation-component remove
```

Removal does not inspect or modify any OpenBubbles user configuration.

## Genuine Mac activation input

The validation provider and genuine hardware input are separate requirements.
On a genuine Mac, run the official Mac Hardware Info helper and copy its
base64 `OABS` payload. In LiteBubbles setup, paste that payload; the backend
validates it before use. The payload is kept behind the existing Secret Service
boundary, not in SQLite, logs, D-Bus messages, or diagnostics. See
[`hardware-input.md`](hardware-input.md) for the accepted format and the
manual-first flow.

The Mac is used to produce supported activation information, not as a
continuous relay. LiteBubbles does not read an existing OpenBubbles profile or
session.

## Current verification boundary

The public provider, archive validation, FairPlay extraction/validation,
helper isolation, error handling, and hardware parsing are tested without
proprietary inputs. No real Apple-account/IDS/APS smoke test is claimed until
a user supplies a compatible official component, genuine Mac hardware input,
and Apple credentials and runs the setup command. The current Linux rustpush
build uses its upstream remote Anisette v3 provider; that network dependency
is not silently replaced by LiteBubbles and should be reviewed before a
distribution release.
