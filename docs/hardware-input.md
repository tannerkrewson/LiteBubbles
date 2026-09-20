# Genuine Mac hardware input

LiteBubbles accepts the manual payload copied from the official OpenBubbles
Mac Hardware Info helper. The accepted input is standard base64 for:

```text
ASCII "OABS" | one byte (`0` or `1`) | `bbhwinfo.HwInfo` protobuf
```

The protobuf contains the macOS software metadata and all hardware fields
needed by rustpush's macOS configuration. LiteBubbles validates the prefix,
flag, protobuf structure, required strings, and byte lengths before the
backend can use the value.

The setup flow is intentionally manual-first:

1. On a genuine Mac, run Mac Hardware Info and copy its base64 activation
   payload. The Mac is used to produce this payload, not as a relay.
2. In LiteBubbles on Fedora, paste the base64 payload into the account setup
   screen. The parser accepts either `0` or `1` flag payload produced by the
   helper and does not make a network request while parsing it.
3. Continue the local rustpush activation and account-registration flow. The
   Mac does not need to remain online after the supported activation data has
   been entered; any later renewal requirement is determined by upstream
   rustpush and Apple services.

When retained between runs, the payload belongs in the GNOME Secret Service
through the storage boundary described in `docs/validation-provider.md`, not
in SQLite or a configuration file.

`MB...` sharing codes are not accepted. They require the OpenBubbles sharing
service, so LiteBubbles reports an actionable error asking for the base64
payload instead of contacting that service. Hardware values are backend-owned
and must not be logged, included in diagnostics, or exposed through GTK or
D-Bus types.
