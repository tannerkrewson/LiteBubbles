# LB-008 build blocker

Status: public compilation unblocked by LB-060. Production FairPlay signing
remains tracked separately by LB-064.

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
does not copy, synthesize, or modify those files.

LB-060 records the public-build resolution in
[`lb-060-public-build.md`](lb-060-public-build.md). The pinned submodule is
prepared by a separable patch that removes the compile-time file dependency and
returns an explicit runtime-unavailable error in both normal and
`dummy-fairplay` builds. This unblocks LB-008's compile dependency without
claiming that Apple activation works. The production signer/provider and its
security/licensing review are intentionally separate LB-064 work.

The upstream checkout's own metadata is not LiteBubbles project guidance. The
LiteBubbles superproject has no agent-specific instruction files.
