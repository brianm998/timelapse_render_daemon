# 1115 — The LAN listener authorizes per RPC, pairing codes are held in memory, and revocation is a stream wrapper

Status: accepted (TRD-18)

Decision: the LAN listener's interceptor only resolves the bearer token into a `Caller` extension;
it never refuses. Each RPC except `GetServerInfo` and `Pair` calls `authorize()` first, so a LAN
call without a paired device's caller is `UNAUTHENTICATED`, and device RPCs then answer
`PERMISSION_DENIED`.

Why: tonic's interceptor sees only the metadata, not the RPC path, so it cannot exempt two methods.
A tower layer would need body types that the rest of the crate does not use. The per-RPC check is
explicit; `lan_refusals` and the server unit test cover the unary and streaming RPCs it guards.

Decision: TLS is tonic's `ServerTlsConfig` with the `tls-ring` feature, fed the PEM identity
files. The fingerprint is the SHA-256 of the DER that the PEM is encoded from, serialized once.

Decision: the one pairing code lives in memory in `Shared`. Expiry and the clock are passed in, so
the tests use a fake clock. The fifth wrong code clears the code (`RESOURCE_EXHAUSTED`). The code is
compared in constant time and never logged.

Decision: revocation sends a signal per device. A stream from a paired device is wrapped so that it
ends with `UNAUTHENTICATED` when the signal fires. A revoke that lands before a device's next
subscription is still seen.

Decision: `TRD_LISTENING` prints once the loopback listener is bound and the first LAN bind has
been tried. A busy LAN port raises `PROBLEM_LAN_LISTENER_FAILED` and is retried every 30 s; it does
not hold up the loopback clients.
