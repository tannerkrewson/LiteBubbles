# LB-008 build blocker

Status: blocked pending an upstream rustpush source and licensing decision.

The audited rustpush revision is
`f35c4ee062b3c3eae54dc96b89b90ee99f5e1d0c`. Its Cargo manifest has path
dependencies into nested gitlinks, so a plain Cargo git dependency cannot
reproduce the graph. A reproducible checkout requires the recorded nested
submodules.

## Exact Fedora 44 evidence

In the Fedora 44 Toolbx, after installing the Perl modules needed by rustpush's
vendored OpenSSL build, Cargo reaches rustpush and fails with errors such as:

```text
error: couldn't read `vendor/rustpush/src/../certs/fairplay/4056631661436364584235346952193.crt`: No such file or directory
error: couldn't read `vendor/rustpush/src/../certs/fairplay/4056631661436364584235346952193.pem`: No such file or directory
```

The failure repeats for all ten FairPlay certificate/key pairs referenced by
rustpush `src/activation.rs`. The activation module is unconditional, so
disabling the `macos-validation-data` default feature does not avoid these
`include_bytes!` calls. The audited tree contains no required files under
`certs/fairplay/`, and its `.gitignore` ignores that directory. LiteBubbles does
not copy, synthesize, or modify those files.

The missing-file evidence is tracked in LB-050 (#49), which blocks LB-008 (#8).
Until upstream publishes the required files in a compatible, reviewed form, or
an approved upstream change makes the module build reproducibly without
weakening activation security, the adapter cannot honestly be reported as
compiling or passing workspace checks.

The upstream checkout's own metadata is not LiteBubbles project guidance. The
LiteBubbles superproject has no agent-specific instruction files.
