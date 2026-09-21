# LB-008 build blocker

Status: public compilation unblocked by LB-060. Production FairPlay signing
now uses the user-local material prepared by the compatibility installer; real
Apple activation still requires the user's interactive smoke test.

The audited rustpush revision is
`f35c4ee062b3c3eae54dc96b89b90ee99f5e1d0c`. Its Cargo manifest has path
dependencies into nested gitlinks, so a plain Cargo git dependency cannot
reproduce the graph. A reproducible checkout requires the recorded nested
submodules.

## Exact Fedora 44 evidence

In the Fedora 44 Toolbx, before the LB-060 preparation patch, Cargo reached
rustpush and failed with errors such as:

```text
error: couldn't read `vendor/rustpush/src/../certs/fairplay/4056631661436364584235346952193.crt`: No such file or directory
error: couldn't read `vendor/rustpush/src/../certs/fairplay/4056631661436364584235346952193.pem`: No such file or directory
```

The failure repeated for all ten FairPlay certificate/key pairs referenced by
rustpush `src/activation.rs`. The activation module is unconditional, so
disabling the `macos-validation-data` default feature did not avoid these
`include_bytes!` calls. The audited tree contains no required files under
`certs/fairplay/`, and its `.gitignore` ignores that directory. LiteBubbles
does not copy, synthesize, or commit those files. Instead, the production
installer derives equivalent local material from the user-supplied official
`openbubbles.so` using the public `fairplay-certs` module's documented
extraction and pair-validation algorithm.

LB-060 records the public-build resolution in
[`lb-060-public-build.md`](lb-060-public-build.md). The pinned submodule is
prepared by a separable patch that removes the compile-time file dependency and
returns an explicit runtime-unavailable error in the `dummy-fairplay` build.
The normal production build reads only the user-local FairPlay directory
prepared by the compatibility installer. This unblocks LB-008's compile
dependency without putting proprietary material in the repository; Apple
credentials, genuine Mac hardware data, and the user-supplied artifact are
still required for a real activation test.

The upstream checkout's own metadata is not LiteBubbles project guidance. The
LiteBubbles superproject has no agent-specific instruction files.
