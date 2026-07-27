# Authentication state-file contract v1

Status: implemented in Crawlson 0.8.0 through journey v4 and run-report v3.

## Purpose and boundary

Journey v4 adds one replaceable authentication provider:
`agent-browser-state-file`. A journey declares only a role and the visible
checkpoint that proves that role. The state itself is supplied at run time with
`--auth-state`; its path and contents never enter the journey.

This provider imports an already-created `agent-browser` state document. It
does not collect a username, password, one-time code, recovery value, API token,
or arbitrary request header. It does not automate a login form, reuse a browser
profile, persist a named `agent-browser` session, or provide hosted secret
storage. Those require separate provider contracts.

## Journey and report contracts

A journey v4 `[authentication]` table contains exactly:

```toml
[authentication]
provider = "agent-browser-state-file"
role = "viewer"
verification_step = "verify-viewer-session"
```

`role` is application-defined public context, not a credential identifier.
`verification_step` must name the first checkpoint after navigation, and that
checkpoint must be `check_text`. It must run before any capture, guide
instruction, link action, or other checkpoint and cannot itself publish a guide
instruction. This makes the authentication gate part of the same visible
journey that later produces evidence.

Run-report v3 records provider, role, verification step, status, and a SHA-256
binding over the journey identity, revision, source digest, exact origin, and
those three public authentication fields. It does not bind or retain the source
path, file bytes, cookie names or values, storage names or values, file digest,
size, timestamp, or temporary path.

Authentication statuses are honest and distinct:

| Status | Meaning |
| --- | --- |
| `missing` | The required external state was not supplied. |
| `unsupported` | The declared provider is not implemented. |
| `invalid` | The source did not satisfy the bounded exact-origin contract. |
| `load_failed` | Private staging, driver import, or immediate deletion failed. |
| `blocked` | Safety preflight or the visible verification checkpoint blocked use. |
| `verified` | The declared visible role checkpoint passed. |

The corresponding stable reasons include
`authentication_state_missing`, `authentication_provider_unsupported`,
`authentication_state_invalid`, `authentication_state_load_failed`, and
`authentication_verification_failed`. Supplying state to a journey without an
executable v4 authentication declaration produces `authentication_unexpected`.

## Fail-closed order

Crawlson validates the journey and builds public provenance first. It then
requires the exact `--allow-origin` grant and every exact action grant before it
looks at the state path. Provider support and state presence follow. Only then
does it validate the source, check `agent-browser`, create an owned session, and
start browser preparation.

The state is loaded before trace capture and before journey navigation. The
temporary copy is deleted immediately after the single load attempt, including
when the driver rejects it. A safety or state preflight block launches no
browser and records no driver command.

## State document and local handling

The provider accepts a strict JSON object with the `cookies` and `origins`
arrays emitted by supported `agent-browser` state files. `cookies` must be
empty. Every local- or session-storage origin must equal the target scheme,
hostname, and effective port. Unknown fields, duplicate storage origins or
keys, empty effective state, malformed values, and documents larger than 8 MiB
are rejected. Counts and string lengths are bounded.

Cookie import deliberately fails closed. `agent-browser 0.26` restricts
traffic by hostname, while browser cookies ignore ports; an exact-host cookie
could therefore be sent automatically to an unauthorized service on another
port. Cookie-bearing sessions require a future driver boundary that can block
every off-origin main-frame, subresource, fetch, and WebSocket request before it
leaves the browser. Passing a broader cookie document is not silently filtered.
This is a deliberately narrow first provider; a state document for several
origins must be split outside Crawlson.

The source must be a regular non-symlink file. Unix sources must have no group
or other permission bits. Windows sources must not be reparse points; operators
remain responsible for supplying them from a user-private ACL-protected
location. Crawlson reads through the validated file handle, bounds the read,
and rejects a size or modification change during the read.

The validated bytes are copied to a newly created operating-system temporary
directory outside the run tree, using the neutral leaf name `state.json` and
mode `0600` on Unix. The source name is never reused. `agent-browser state load`
currently echoes the path it loaded, so Crawlson deliberately records zero
stdout/stderr bytes and the SHA-256 of empty output for the
`authentication_load` provenance entry. Upstream output remains available only
for in-memory typed validation and is not retained.

## Verification and evidence privacy

A successful state import proves only that the driver accepted a document. It
does not prove that the application recognized a session or that the declared
role is active. Only the visible `verification_step` can change authentication
to `verified`. A mismatch produces a blocked run and stops before later
evidence or guide steps.

Authenticated screenshots and traces intentionally retain the visible UI
needed to reproduce a result. The red-box/dimmed focused image is a focusing
derivative, not a redaction. Use disposable users and fixtures, avoid pages that
display secrets, and review every retained artifact before publication. The
loopback demo uses a disposable per-run storage value and scans its complete
retained output for both the value and source path; the required real-browser
integration applies the same privacy check to reports, logs, screenshots,
traces, rendered guides, and other run files.
