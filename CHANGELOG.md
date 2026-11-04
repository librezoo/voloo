# Changelog

All notable changes to this project are documented here. Versions follow
[Semantic Versioning](https://semver.org/); commits follow Conventional Commits.

## Unreleased

The host is rebuilt for the USB-C Voloo Switch (PC → USB → Switch, no Core).

### Breaking changes

- This repository starts from a clean history. Earlier Core-era software (0.6) is not
  part of it.

### Added

- `voloo` single executable: CLI with `--json` and stable exit codes, a background
  process that owns the ports (auto-start, idle exit, never changes outputs), local HTTP
  API with OpenAPI and server-sent events, MCP server over stdio and HTTP.
- Protocol 2.5 device layer with read-back confirmation, no resend after a lost reply,
  explicit unknown outcomes, fault catalog (conditions and events), transaction tracing
  and bounded JSONL logs.
- Web dashboard (`voloo ui`) in the LibreZoo style with hold-to-switch controls, device
  view, activity and faults; device locks for every client.
- `voloo doctor`, `voloo report` (local diagnostic bundle), `voloo demo` (simulated
  devices), `voloo lock`/`unlock`.
- Licensed under Apache-2.0.
- npm (`voloo` + `@librezoo/voloo-<platform>`) and pip (`voloo`) packages with isolated
  install checks.
