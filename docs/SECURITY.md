# Security & Threat Model — Senvakara

**Status:** baseline security requirements for a highly privileged local-first remote-control tool. No security certification or completed implementation is claimed.

## 1. Assets and trust boundaries

**Sensitive assets:** host SSH credentials and keys, trusted host identities, ADB debugging authorizations, private source files, project APKs, terminal output, live screen/video/input, host filesystem access and saved sessions.

**Trust boundaries:** Windows UI↔Rust core; desktop↔Fedora SSH/agent; Linux host agent↔OS/ADB server; host↔Android; desktop↔remote desktop service; client↔VPN. Discovery messages are untrusted until authenticated.

## 2. Threats

| Threat | Consequence | Mitigation |
|---|---|---|
| Rogue LAN host / mDNS spoof | Fake remote host, credential theft | Verify SSH host key / agent identity; don't trust discovery alone |
| Exposed ADB TCP service | Unauthorized device access | Never bind remote ADB publicly; tunnel/authenticate; firewall |
| Compromised agent or overprivileged daemon | Host takeover | Least-privileged service, scoped operations, security updates |
| Wrong Android route/device | Unintended install/command/data leak | Bind host+route+transport, revalidate on action, explicit target confirmation |
| Command injection / path traversal | Host filesystem or shell compromise | Structured args, canonical paths, input checks, minimal permitted operations |
| Leaked credentials in SQLite/logs | Persistent host compromise | OS secrets store, redaction, minimal retention |
| Replay after network drop | Duplicate destructive action | Idempotency where possible; never blind replay side effects |
| Unauthorized screen/input sharing | Exposure/control of desktop or phone | Authenticated session, explicit user action, obvious active state, close/cancel |
| Malicious third-party binary | Code execution | Trusted source, hash/signature checks, dependency/license audit |

## 3. Secure defaults

1. No cloud login required; local-first.
2. All remote operations authenticated and encrypted; verify remote host identity on first trust and changes.
3. No `StrictHostKeyChecking=no`, trust-all TLS or insecure default endpoints.
4. Linux host agent should **not** run as root by default. Restrict privileges/operations.
5. ADB 5037 and other control listeners must not be exposed to untrusted networks; remote access uses protected route.
6. Wireless ADB pairing requires physical user approval via Android's supported workflow.
7. Do not persist raw secrets in SQLite, logs, analytics or exports.
8. Show target host/device clearly on sensitive operations (install/uninstall, shell, control input, file overwrite).
9. Default no background mirroring/recording or clipboard sync without user request.
10. Errors must disclose enough to act but not dump credentials or internal secret paths.

## 4. Authentication and authorization

**SSH:** public key preferred, host-key pinning/verification, minimal credential prompting. Password support can exist with OS secret storage.

**Agent:** mutual authentication, explicit first-time pairing and ability to revoke trust; encryption via established libraries. Message schema and pairing protocol selected only after PoC/security review.

**Android:** requires Android ADB authorization; Senvakara cannot override device-side permissions. Host-local ADB identifiers must be checked against expected route at operation time. Multi-user Android policy is out of scope.

**Desktop protocols:** prefer secure RDP/VNC configurations and warn when endpoint lacks required encryption/authentication. Clipboard and input sharing are explicit.

## 5. Credential and secret storage

Store private keys/tokens using OS-backed APIs when possible. Database should hold only opaque references and nonsecret metadata. Never place secrets in frontend source, localStorage, environment examples, session export or verbose logs. Document secure import, rotation, revocation and wipe behavior.

SSH password authentication is supported (TASKS CRED-01), including servers that only take the password through one hidden keyboard-interactive prompt; nothing else a server asks is answered with it. SSH passwords, key passphrases and keychain private keys live only in the OS credential store (Windows Credential Manager, ADR-025) and never in SQLite or logs; the database holds opaque references and non-secret metadata (label, algorithm, public key, certificate). A password is never read back into the UI. RDP credentials are stored as `TERMSRV/<address>` (ADR-026) and removed when the host is deleted. The host key is checked before any credential is sent.

## 6. Remote execution policy

Use typed commands with executable and arguments; avoid implicit shells, especially on agent. Provide capability- and action-level checks. Operations that mutate host/devices require user initiation or explicitly trusted recipe configuration. Restrict file roots if an agent manages files; avoid path traversal and symlink races. Cap execution duration/output and support cancellation.

Workflow host steps follow ADR-030 (reviewed or user-written bash, settings as quoted `SV_*` exports, sudo password per run on stdin). Changes that can lock the user out are guarded (ADR-031): the host schedules its own rollback before the change, and only a new sign-in afterwards cancels it; SSH hardening refuses to remove the way the current session signs in; a module runs only on the systems it declares. The host security posture check is read-only and runs without sudo, only on a user action; it never claims compliance with a benchmark.

## 7. Data plane safeguards

Encrypt tunnels; separate media from control messages; bound logcat buffers, video frames and transfer queues; avoid persisting screen streams. Verify file destination and checksum where reasonable, prevent unintended overwrites and keep copy logs redacted.

## 8. VPN and internet access

Start on local Wi-Fi/Ethernet. Later use established VPNs like WireGuard/Tailscale to provide reachability; VPN alone does not replace per-host auth. Never require public port forwarding for raw ADB. mDNS discovery is not an authentication mechanism and may not traverse VPN.

## 9. Incident and vulnerability handling

Before public release, provide a private vulnerability reporting route and maintenance plan; track dependency advisories and agent compatibility. Do not invent a security email address yet. Document revoking host trust, disabling agent, removing keys and disconnecting Android debugging.

## 10. Mandatory security tests

- MITM/changed SSH host key rejected.
- Unpaired agent actions denied.
- Unauthorized Android debugging denied.
- Swapped/stale route never silently targets another device.
- Agent cannot execute unauthorized arbitrary shell strings.
- No sensitive data in logs/database.
- Ports and default binding verified, firewall-tested.
- Process stream cancellation and large input resilience.

**References:** [API.md](API.md), [ARCHITECTURE.md](ARCHITECTURE.md), [TESTING.md](TESTING.md).
