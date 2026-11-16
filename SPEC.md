# Orbit Module Protocol v1

Draft 0.1.0 — status: **draft, unimplemented in hosts**. This document is the
single source of truth for how Orbit hosts talk to out-of-process module
binaries. The `orbit-island` crate is the reference implementation of the
message envelope and the Island engine semantics; nothing else implements this
yet, on either side of the pipe.

## Goals

- One small, open protocol shared by all Orbit ecosystem modules.
- Zero network access. The protocol is a local, parent↔child conversation.
- No dynamic code loading, no eval, no scripts shipped by modules. A module is a
  single statically-linked binary the host launches and sandboxes.
- Explicit permission negotiation. A module can request capabilities; the user
  grants or denies them; the host enforces them. No implicit access.
- Forward compatibility. Unknown fields are rejected, not ignored, in
  security-relevant positions (see "Invariants").

## Transport

- The host spawns the module binary as a child process.
- Communication is newline-delimited JSON (NDJSON) over stdin/stdout.
- Each line is exactly one message (see "Message envelope"). Lines larger than
  1 MiB are protocol violations and close the channel.
- stderr is module-owned free text, shown in a developer console only, never
  parsed by the host.
- The host may kill the child at any time. Modules must exit cleanly on EOF.

## Manifest (`orbit-module.json`)

Every module ships a manifest next to its binary. Hosts read the manifest
*before* launching. Example:

```json
{
  "format": 1,
  "id": "ghostdrop",
  "name": "GhostDrop",
  "version": "0.1.0",
  "protocol": "1",
  "description": "Temporary clipboard and file drop zone for Orbit.",
  "permissions": ["clipboard_read", "file_system_watch"],
  "island": true
}
```

Rules:

- `format` must be `1`. Unknown format → host refuses to load.
- `id` is lowercase slug `[a-z0-9-]{1,64}`, unique per install.
- `protocol` is the protocol major version; hosts reject mismatches.
- `permissions` lists requested permission identifiers using the Orbit
  permission registry (see `orbit-core::permissions`). Requesting is not
  granting; the user approves per request on first launch and can revoke at
  any time in Settings.
- Manifest parsing uses strict schemas (`deny_unknown_fields`). A manifest
  containing executable hooks (`on_load`, `pre_hook`, …) fails to parse and the
  module is not started.

## Message envelope

```json
// request
{ "v": 1, "id": 7,  "kind": "request",  "method": "island.notify", "params": { … } }
// response
{ "v": 1, "id": 7,  "kind": "response", "method": "island.notify", "result": { … }, "error": null }
// event (unsolicited)
{ "v": 1,           "kind": "event",    "method": "island.changed", "params": { … } }
```

- `v` is always `1`.
- `id` is present on request/response; replies echo it. Events have no `id`.
- `kind`: `request | response | event`.
- `method`: dotted path, lowercase. Namespaced by capability area.
- `params` / `result` / `error`: objects or strings. `error` is a
  human-readable message, never a stack trace.

### Handshake

1. Host → `module.hello` (request) with `{ "host": "orbit", "host_version": "0.1.0", "granted": ["clipboard_read"] }`.
2. Module → response `{ "ok": true, "name": "ghostdrop", "version": "0.1.0", "pid_ready": true }`.
3. Only after a successful handshake may the module emit events or receive non-`module.*` requests. Permission without grant is inert.

## Namespaces

| Namespace | Purpose | Permission |
| --- | --- | --- |
| `module.*` | handshake, shutdown, ping | always allowed |
| `island.*` | notifications, island state | host-mediated, no grant needed for *display request*; content is not persisted by the host |
| `clipboard.*` | read clipboard | `clipboard_read` |
| `screenshot.*` | request capture | `screen_capture` |
| `files.*` | watch / stat (never write) paths | `file_system_watch` |
| `system.*` | CPU/RAM percentages (read-only, sampling) | `process_metrics` |

Methods are added per namespace over time; hosts must ignore unknown methods
with an error response, not crash.

## Security invariants

1. No network. The module protocol never carries sockets outward. A module with
   the `network` permission may open its own sockets — visibly, opt-in,
   revocable — but protocol traffic itself is strictly child↔parent.
2. No code loading. Static binaries only; no plugin scripts, no eval.
3. Strict schemas everywhere. Unknown fields in requests the module cannot
   ignore are rejected; the host treats malformed messages as module bugs and
   may terminate the child.
4. Secrets are never sent over the pipe. Hosts pass handles, not credentials.
5. Data minimization. Events announce *that* something happened and minimal
   metadata; bulk/content payloads require the matching permission.
6. Honest capability reporting. If the module cannot do the thing on this
   platform (e.g. Wayland global hotkeys), it must answer with a structured
   "unavailable" error and keep running. No fake success.

## island.* surface

| Method | Kind | Notes |
| --- | --- | --- |
| `island.notify` | request | `{ priority: "low"|"normal"|"high", content: {…tagged IslandContent…}, auto_dismiss_ms?: u64 }` → `{ id }`. Queue ≤16, drop-oldest. |
| `island.dismiss` | request | `{ id }` → `{}`. Unknown id → structured error. |
| `island.dismiss_active` | request | → `{}` |
| `island.snapshot` | request | → collapsed state, active/queued counts, timer |
| `island.start_timer` | request | `{ label, duration_ms }` — 1s..8h → `{ timer }` |
| `island.cancel_timer` | request | → `{}` |
| `island.toggle_timer_pause` | request | → `{ timer }` |
| `island.event.changed` | event | fired after any state change; payload is the snapshot |

Content variants are the tagged `IslandContent` union from the `orbit-island`
crate (`toast`, `clipboard_stored`, `screenshot_captured`, `file_transfer`,
`workspace_switched`, `system_load`, `timer_done`).

## Versioning

- Breaking changes bump `v`. Additive changes are `v`-preserving.
- `orbit-module.json` `protocol` gates negotiation; a host may ship v1 and v2
  simultaneously during transitions.

## Open questions (v1.1)

- Per-permission rate limits and budget enforcement (notifications per minute).
- Island content size caps for non-toast payloads.
- Module update channel (signed releases only; no silent updates).
