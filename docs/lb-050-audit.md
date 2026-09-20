# LB-050 FairPlay and Apple validation audit

This note records the source audit behind LB-050. The reference checkouts are
outside this repository; no installed OpenBubbles data or account state was
read. The audit date is 2026-09-20.

## Audited revisions

| Source | Revision | Purpose |
| --- | --- | --- |
| `OpenBubbles/rustpush` | `f35c4ee062b3c3eae54dc96b89b90ee99f5e1d0c` | Pinned backend source and current `master` at audit time |
| `OpenBubbles/rustpush` PR #10 | `a16fccae27260cd239a13ad12ea8067a7b695f93` | Unmerged `dummy-fairplay` proposal |
| `OpenBubbles/openbubbles-app` `rustpush` | `eed1b6332efbb17adbf5ebfa2263ad770169f75e` | Application integration reference |
| `stevesoltys/openbubbles-build-modules` | `86fa9efa722f811fcb57fca3dd5d9c0d43db5be6` | Public adapters around the closed validation component |
| `OpenBubbles/Mac-Hardware-Info` | current public `main` | Genuine Mac hardware input reference |

Reference links are listed in [references.md](references.md).

## Exact compile-time boundary

`rustpush/src/activation.rs` defines `include_cert!` and expands ten names
into two `include_bytes!` calls each. The required paths are:

```text
certs/fairplay/4056631661436364584235346952193.crt
certs/fairplay/4056631661436364584235346952193.pem
certs/fairplay/4056631661436364584235346952194.crt
certs/fairplay/4056631661436364584235346952194.pem
certs/fairplay/4056631661436364584235346952195.crt
certs/fairplay/4056631661436364584235346952195.pem
certs/fairplay/4056631661436364584235346952196.crt
certs/fairplay/4056631661436364584235346952196.pem
certs/fairplay/4056631661436364584235346952197.crt
certs/fairplay/4056631661436364584235346952197.pem
certs/fairplay/4056631661436364584235346952198.crt
certs/fairplay/4056631661436364584235346952198.pem
certs/fairplay/4056631661436364584235346952199.crt
certs/fairplay/4056631661436364584235346952199.pem
certs/fairplay/4056631661436364584235346952200.crt
certs/fairplay/4056631661436364584235346952200.pem
certs/fairplay/4056631661436364584235346952201.crt
certs/fairplay/4056631661436364584235346952201.pem
certs/fairplay/4056631661436364584235346952208.crt
certs/fairplay/4056631661436364584235346952208.pem
```

These are twenty physical files: ten certificate/key pairs. The `.crt` bytes
are sent as the FairPlay certificate chain. The `.pem` bytes are parsed as
private RSA keys by `fairplay_sign`. The source tree ignores
`certs/fairplay/*`, and no build script creates the paths.

The manifest's `macos-validation-data` feature enables `open-absinthe`, and is
part of the default feature set. It does not guard `mod activation` or these
includes. Therefore `--no-default-features` does not avoid this compile-time
failure. LiteBubbles must not copy the missing material.

The public rustpush tree does contain a separate
`certs/legacy-fairplay/fairplay.crt` and `fairplay.pem`. Its public CI copies
that legacy pair into the ten ignored paths under a step named “fake Fairplay
keys”. LiteBubbles does not copy or vendor that pair: even publicly visible
legacy key material is outside the project's intended source boundary.

PR #10 proposes a `dummy-fairplay` feature which cfg-gates the real includes
and uses two dummy resources. It is still open and unmerged. A LiteBubbles
public-build patch must keep any generated dummy material confined to an
explicit development/CI configuration and must not make it a production
credential.

## Runtime distinction

The missing FairPlay pair is consumed by `rustpush::activate`. When an APS
state has no device key pair, the APS connection calls activation; activation
creates a local CSR, signs the activation request with the selected FairPlay
pair, posts it to Apple's device-activation endpoint, and receives the device
certificate used by APS.

This is separate from validation data. `OSConfig::generate_validation_data`
is called by Apple delegate login and IDS registration. The Mac implementation
fetches Apple's validation certificate, starts a validation context using Mac
hardware data, exchanges session information with Apple, and returns signed
validation data. A dummy FairPlay build therefore proves only that code can be
compiled; it does not prove Apple activation, IDS registration, or APS use.

## Compatibility-provider findings

The MIT-licensed `openbubbles-build-modules` repository documents two separate
adapters around an x86_64 `openbubbles.so` from an OpenBubbles release:

- `macos-validation-data` uses a JSON-lines helper and fixed ABI offsets to
  call the closed component and generate signed validation data from a
  `HardwareConfig` plus Apple's session response.
- `fairplay-certs` extracts the ten certificate/private-key pairs from fixed
  offsets in that same shared library and writes rustpush-compatible files.

The first adapter is the basis for the preferred opaque local provider. The
second is raw key extraction and is not the primary LiteBubbles design. The
public `open-absinthe` wrapper is Android-specific and cannot be reused as-is
on Fedora. The runtime FairPlay signing boundary is tracked separately in
LB-064 (#62), pending a reviewed rustpush/provider API or an explicitly
reviewed fallback.

## Genuine Mac hardware input

The official Mac Hardware Info helper emits `OABS`, one sharing flag, and a
protobuf `HwInfo` payload. The payload contains the product name, MAC address,
serial/UUID values, board and OS identifiers, encrypted identifier fields,
ROM, and MLB values required by the Mac configuration. These are sensitive
values and must never enter logs, diagnostics, CI artifacts, or source
control.

The current OpenBubbles quickstart says the Mac does not need to remain online
after Mac-based activation. Its renewal documentation says Mac-based
registration can renew indefinitely without a Mac connection. LiteBubbles will
support manual paste/entry first and will not depend on an OpenBubbles relay or
an existing OpenBubbles installation.

## Work split

- LB-060 (#63): public/CI rustpush compilation without private FairPlay files.
- LB-061 (#60): backend-only production validation provider and local bootstrap.
- LB-062 (#61): genuine Mac hardware input and secure validation.
- LB-063 (#59): provider/hardware integration with authentication, IDS, and APS.
- LB-064 (#62): runtime FairPlay signer boundary and separate review.

## Current implementation status

The public rustpush build path is complete: `scripts/prepare-rustpush.sh`
applies the minimal auditable patch and CI uses it without private files.
`litebubbles-validation-provider` now defines the backend-only provider
boundary, and `OpenBubblesValidationProvider` uses a hash- and version-checked
user-supplied `openbubbles.so` through the isolated
`litebubbles-validation-helper` process. The backend exposes a constructor for
the default installed provider, while GTK, D-Bus, core models, and SQLite do
not receive provider or key types.

Mac Hardware Info payloads are parsed independently by
`litebubbles-rustpush-backend` and persisted through the Secret Service
boundary when setup state is retained. No real component, Apple credential, or
hardware payload was used in tests. Consequently, this work establishes the
public build and production integration boundary but does not claim successful
FairPlay device activation, IDS registration, or APS connection; those remain
tracked in LB-064 and LB-063.

Until the last boundary is resolved, LiteBubbles must report production Apple
activation as unavailable rather than treating dummy material as valid.
