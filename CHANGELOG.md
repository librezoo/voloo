# Changelog

All notable changes are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/) and versions follow
[Semantic Versioning](https://semver.org/). Per-release details and validation
evidence are in [docs/releases](docs/releases/).

## Unreleased

## 0.8.0 - 2026-11-26

### Added

- Tab completion for Bash, Zsh, Fish and PowerShell. Complete commands, options,
  device nicknames and UIDs, local firmware paths, serial ports and recent trace IDs.
- On first interactive use of `voloo` or `voloo ls`, accept a one-time prompt to
  enable completion in new terminals. Use `voloo completions install` to enable
  it later.
- The CLI and dashboard advise checking the USB connection when it drops
  repeatedly.

## 0.7.5 - 2026-11-17

### Fixed

- `voloo firmware rollback` no longer reports "Connection reset by peer" after the
  rollback completed: the background process now always reads the request body, and
  the client sends each request in one write.

## 0.7.4 - 2026-11-17

### Added

- `voloo firmware update` reads the published firmware manifest, picks the newest image
  for the device and this version of voloo, verifies size and SHA-256, and refuses to
  install anything older than the running firmware. New firmware needs only a manifest
  entry, not a new npm release. Package addresses must be https on known hosts.

### Fixed

- A background process left over from another version is replaced on connect when idle;
  when busy, the online firmware update refuses instead of installing an older package.

## 0.7.3 - 2026-11-17

### Changed

- Paired with firmware 3.3.0: same features and protocol as 3.2.0 in a smaller image.

## 0.7.2 - 2026-11-15

### Added

- Recognize device hardware revision 0.2 and show the value read from the device in the
  CLI, dashboard, HTTP API and MCP. Legacy 3.0 devices stay supported.

### Changed

- Updates and rollbacks to firmware that cannot read the hardware identity are refused.

## 0.7.1 - 2026-11-15

### Added

- Authenticated `POST /v1/shutdown/idle`: stops the background process only when no
  device request is queued or running (409 otherwise). Device outputs are never changed.

## 0.7.0 - 2026-11-15

voloo is rebuilt for the USB-C voloo Switch (computer → USB → Switch). This repository
starts from a clean history; earlier Core-era software (0.6) is not part of it.

### Added

- One `voloo` executable: CLI with `--json` and stable exit codes, a background process
  that owns the ports (auto-start, idle exit, never changes outputs), a local HTTP API with
  OpenAPI and server-sent events, and an MCP server over stdio and HTTP.
- Protocol 2.5 device layer: read-back confirmation, no resend after a lost reply, explicit
  unknown outcomes, a fault catalog, transaction tracing and bounded JSONL logs.
- Web dashboard (`voloo ui`) with hold-to-switch controls, device view, activity and faults,
  system/light/dark appearance, and English, Simplified and Traditional Chinese.
- Managed USB firmware updates with A/B trial, confirmation and rollback, shared by the CLI,
  dashboard and API; local `.voloofw` files remain supported.
- `voloo doctor`, `voloo report` (local diagnostic file), `voloo demo` (simulated devices),
  `voloo lock` and `unlock`.
- npm (`@librezoo/voloo` plus one package per platform) and pip (`voloo`) packages with
  isolated install checks. Licensed under Apache-2.0.
