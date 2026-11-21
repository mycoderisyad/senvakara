# API & Agent Contract — Senvakara

**Status:** **proposed**, not an implemented API. This describes application ↔ Linux host agent communication if a custom agent API is chosen. SSH/SFTP/ADB/RDP remain separate protocols; never imply they are HTTP endpoints. No public cloud API in MVP.

## 1. Boundary and trust

Desktop core is client. Linux host agent is authenticated remote service. Transport candidate: **mutual authenticated encrypted channel** (e.g., mTLS gRPC or appropriately authenticated WebSocket) or **SSH tunnel**. Final mechanism pending PoC/ADR. Raw ADB port must not be publicly exposed.

Discovery advertisements are untrusted until identity/pairing is complete. All actions require authorization and permissions scoped to host, user and operation. Auth failures must not reveal sensitive details.

## 2. Versioning

- Protocol version sent in handshake; capabilities negotiated.
- Unsupported commands return `UNSUPPORTED_CAPABILITY`, not an unexplained timeout.
- Add fields compatibly; breaking messages require major version increment.
- Idempotency tokens recommended for requests that may be retried; installs/commands never blindly retried after ambiguous result.

## 3. Common envelope (illustrative JSON, not wire format mandate)

```json
{
  "protocol_version": "0.1-draft",
  "request_id": "uuid",
  "operation": "host.get_info",
  "host_id": "host-fedora-01",
  "payload": {}
}
```

Responses conceptually include `request_id`, `ok`, `result` or `error`. Use structured error codes:

`UNAUTHENTICATED`, `UNAUTHORIZED`, `HOST_OFFLINE`, `DEVICE_OFFLINE`, `ADB_UNAUTHORIZED`, `ROUTE_STALE`, `UNSUPPORTED_CAPABILITY`, `TIMEOUT`, `CANCELLED`, `CONFLICT`, `INVALID_INPUT`, `INTERNAL_ERROR`.

## 4. Capability / inventory operations (proposed)

| Operation | Input | Output | Safety |
|---|---|---|---|
| `agent.hello` | client version, negotiated capabilities | agent version, supported capabilities | authenticated after pairing |
| `host.get_info` | host ID | OS/version, uptime, non-sensitive metadata | read-only |
| `host.get_metrics` | host ID, interval/fields | CPU/RAM/storage/network | rate-limit; not high-frequency by default |
| `devices.list` | host ID | devices with host-local ADB selectors and auth states | output only authorized devices |
| `devices.watch` | host ID | discover/connect/disconnect events | reconnect and event ordering |
| `operations.cancel` | operation/request ID | cancel acknowledged/too late | idempotent where possible |

## 5. Android action contracts

All actions require **host ID + host-local device selector + verified route ID**. No unqualified ADB commands.

```json
{
  "protocol_version": "0.1-draft",
  "request_id": "run-123",
  "operation": "android.install_apk",
  "host_id": "host-fedora-01",
  "payload": {
    "device_id": "android-b",
    "route_id": "route-fedora-usb-b",
    "expected_transport_identifier": "ADB_HOST_LOCAL_SERIAL",
    "artifact_ref": "staged-upload-456"
  }
}
```

Proposed operations: `android.get_info`, `android.shell` (strict consent/restrictions), `android.install_apk`, `android.uninstall_package`, `android.logcat.start`, `android.file.push`, `android.file.pull`, `android.screenshot`, `android.mirror.start` (transport for media may be separate).

**Shell cautions:** arbitrary shell commands are sensitive; authenticated ownership, explicit user initiation, duration/output bounds, and sanitization are mandatory. Never allow a generic unauthenticated remote command API.

## 6. Host process and workflow operations

For MVP, terminal/SSH/SFTP should preferably use their native SSH connection, not duplicated agent endpoints. Later operations may include `process.start_allowed`, `process.cancel`, `artifact.upload/download`, and `workflow.step_status` with strict allowlists and capability checks. Avoid accepting a raw `cmd` string from arbitrary clients.

## 7. Event schema (conceptual)

```json
{
  "event_id": "evt-001",
  "host_id": "host-fedora-01",
  "type": "device.route_state_changed",
  "timestamp": "2026-11-21T00:00:00Z",
  "payload": {"route_id": "route-fedora-usb-b", "state": "unavailable"}
}
```

Events include monotonic per-host sequence/epoch if implemented; on reconnect client must request a **fresh authoritative snapshot** rather than trust stale events. Event schemas must be bounded and redact secrets.

## 8. Streaming / transfers

- PTY, SFTP, RDP/VNC, scrcpy and logcat have different performance, auth and flow-control needs.
- Specify bounded buffers, cancellation, EOF/error semantics and per-stream identifiers.
- Large files stream/chunk; don't embed APK binary into JSON control messages.
- Compare checksums/size for staged artifacts; inspect paths and restrict traversal.
- No direct public ADB server or remote desktop listener by default.

## 9. Pairing and rotation (pending ADR)

Candidate flow: user initiates pair against discovered/manual host → sees verifiable agent identity/fingerprint → approves scope → client and agent persist trust material in appropriate OS-protected store → session is encrypted and mutually authenticated → revocation/rotation supported. Details and cryptographic algorithm selection must use established libraries and security review.

## 10. Error handling and retries

- Network timeouts and `HOST_OFFLINE` may use bounded reconnect.
- Unauthorized state triggers re-authorization, not automatic insecure bypass.
- `ROUTE_STALE` forces a fresh device snapshot; do not target same serial on another host blindly.
- Never auto-replay commands such as install/uninstall/shell if execution outcome is unknown.

## 11. API verification gates

Authenticated inventory, revoked trust, malformed requests, stale route, two identical phones, oversized inputs, slow consumers, cancel during transfer, unsupported agent version, dropped connection and no exposed service without auth. See `TESTING.md` and `SECURITY.md`.
