# Changelog

## [Unreleased]

### Added

- `SystemStats` (`cpu_percent`, `ram_percent`, `ram_used_mb`, `ram_total_mb`)
  and a `system` field on `IslandSnapshot`, filled via
  `IslandManager::set_system`; host-side only, never sampled by the crate itself
- `NowPlaying::volume` (optional, `None` unless the host pushes it from MPRIS)
- `NowPlaying` carrying `volume` in ambient snapshot tests

## [0.1.0] - 2026-11-16

Initial module release: core domain logic, unit tests, ecosystem integration
spec (Orbit Module Protocol v1).
