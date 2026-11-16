# Orbit Island

**Orbit Island engine: the in-domain core of the signature island UI, plus the Orbit Module Protocol v1 reference implementation.**

> Orbit companion module — phase 2/4 of the [Orbit ecosystem](https://github.com/Can-5/orbit).
> Standalone crate: usable without Orbit, without other modules, and without network.

## Implemented now

- IslandManager: priority queue (≤16, drop-oldest), high-priority preemption, auto-dismiss expiry
- FocusTimer with pause/resume rebase; TimerDone fires exactly once at High priority
- IslandMessage NDJSON-ish envelope (v:1, request/response/event) + SPEC.md (Orbit Module Protocol v1)

## Not implemented yet (explicitly out of scope for this crate)

- No GTK/Wayland/AppKit rendering — that lives in the Orbit host (src-tauri)
- No process management — Orbit spawns modules; this crate is the in-process reference

## Build & test

```sh
cargo test  # 8 tests
```

Dependencies: serde, serde_json, thiserror. No cross-repo path dependencies, no Git dependencies, no downloads at build time beyond crates.io.

## Privacy

Pure state machine. Nothing leaves the device; no network code, no telemetry.

## License

MIT © The Orbit contributors. See [LICENSE](./LICENSE).
