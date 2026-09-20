# LB-008 build blocker

Status: blocked pending an upstream rustpush source/licensing decision.

The LiteBubbles worktree pins `vendor/rustpush` to the audited revision
`f35c4ee062b3c3eae54dc96b89b90ee99f5e1d0c`. Its Cargo manifest has path
dependencies into six nested gitlinks, so a plain Cargo git dependency cannot
reproduce the graph. A fresh checkout must use:

```sh
git submodule update --init --recursive
```

The recursive checkout resolves the recorded commits for `open-absinthe`,
`third_party/apple-private-apis` (including its `clearadi` gitlink),
`third_party/quinn`, `third_party/rtc`, `third_party/rustls`, and
`third_party/rustls-platform-verifier`. The backend selects rustpush's
declared `remote-anisette-v3` feature and repeats its Quinn patch at the
workspace root because Cargo does not inherit patches from path dependencies.

## Exact Fedora 44 evidence

In the Fedora 44 Toolbx (`litebubbles-dev`), after installing the Perl modules
needed by rustpush's vendored OpenSSL build, `cargo check -p
litebubbles-rustpush-backend` reaches rustpush and fails with errors such as:

```text
error: couldn't read `vendor/rustpush/src/../certs/fairplay/4056631661436364584235346952193.crt`: No such file or directory
error: couldn't read `vendor/rustpush/src/../certs/fairplay/4056631661436364584235346952193.pem`: No such file or directory
```

The failure repeats for all ten FairPlay certificate/key pairs referenced by
`vendor/rustpush/src/activation.rs`. `mod activation;` is unconditional in
`vendor/rustpush/src/lib.rs`, so disabling the `macos-validation-data` default
feature does not avoid these `include_bytes!` calls. The audited commit tracks
only `certs/legacy-fairplay/*` and root certificates; its `.gitignore` has
`/certs/fairplay/*`, and no required FairPlay files are present in the pinned
checkout. LiteBubbles does not copy, synthesize, or modify those files.

Earlier failures were resolved without changing the pin: Fedora's minimal
Toolbx needed `perl-FindBin`, `perl-IPC-Cmd`, and `perl-Time-Piece`, now listed
in `scripts/bootstrap-toolbox.sh`; the default Linux feature configuration
then exposed the missing upstream FairPlay files.

## Hygiene and licensing

`vendor/rustpush/AGENTS.md` is tracked by the upstream submodule and is not a
LiteBubbles project file. The LiteBubbles superproject has no `AGENTS.md` or
other tool-specific metadata. The submodule necessarily exposes upstream
source and metadata for reproducible Cargo path dependencies; no upstream
`AGENTS.md` was copied, interpreted as LiteBubbles guidance, or added to the
superproject. The upstream `LICENSE` is SSPL-1.0, and its
`LICENSE.exceptions` names OpenBubbles only, so LiteBubbles must obtain legal
review before treating this source as distributable.

Until the upstream project publishes the required files in a compatible,
reviewed form (or LiteBubbles receives an explicit legal/build decision), the
adapter cannot honestly be reported as compiling or passing workspace checks.
