# Voloo Host

Local host software for the USB-C Voloo Switch. One `voloo` executable talks to
each Switch over its own USB cable (USB CDC, no driver needed on macOS, Linux or
Windows 10/11) and offers the same functions as a CLI, an MCP server for AI agents
a local HTTP API for any language and a web dashboard for every connected Voloo.
See the [user guide](docs/user-guide.md).

Voloo only switches the externally supplied SOURCE power; it is not a power source.
**OFF does not mean the load terminals are at zero volts** — use `discharge` to
confirm VOUT ≤ 1 V.

## Install

```sh
npm install -g @librezoo/voloo # or: npx @librezoo/voloo ls
pipx install voloo         # or: uvx voloo ls
```

## Build

```sh
cargo build --release      # target/release/voloo
cargo test                 # simulated devices only; never touches real hardware
node packaging/npm/build.mjs && (cd packaging/pypi && uvx maturin==1.15.0 build --release --out ../../dist/pypi)
sh packaging/check-install.sh
```

## How it runs

A serial port can be open in only one program, so a background process
(`voloo daemon run`) owns every Voloo port. The CLI, `voloo mcp` and HTTP clients
start it automatically on first use. It listens on 127.0.0.1 only, follows USB
hot-plug, polls each device (1 Hz, 4 Hz while an event stream is open), and exits
after 10 minutes without clients. **Starting or stopping it never changes any
output.** It is not a system service.

A device that is unplugged or stops answering is shown **offline with an unknown
output state** — the Switch keeps its last output when communication is lost.

## Commands

Devices are named by full UID (`CD-AB-05-F3-69-BE-3B-5D`) or by nickname. Read
commands may omit the device when exactly one Voloo is connected; write commands
always need it.

| Command | Effect |
| --- | --- |
| `voloo ls` | Connected Voloo devices with state and active faults |
| `voloo status [DEV]` / `info` / `diag --kind communication\|adc\|fast` | Read-only |
| `voloo on DEV` / `off DEV` / `discharge DEV` | Switch and confirm by status |
| `voloo cycle DEV [--off-ms 1000] [--no-discharge]` | Discharge (or OFF), wait, ON; stops at the first failure |
| `voloo firmware status DEV` / `firmware update DEV [FILE]` / `firmware rollback DEV` | Managed USB firmware maintenance and A/B rollback; [details and first-install limits](docs/firmware-updates.md) |
| `voloo name DEV NICKNAME` | Persistent nickname (≤ 24 UTF-8 bytes); output must be fully OFF |
| `voloo faults [DEV] [--all]` | Faults with the suggested next step |
| `voloo log --kind operations\|faults\|trace` / `voloo trace ID` | Logs and serial transactions |
| `voloo ui` | Web dashboard: all devices live, switch, rename, lock, activity and faults |
| `voloo demo` | The dashboard with simulated devices; no hardware is touched |
| `voloo lock DEV` / `voloo unlock DEV` | Refuse every change from any client (kept across restarts) |
| `voloo doctor` | Checks USB, permissions, devices, measurements and logs, with fixes |
| `voloo report` | One local file for support: versions, device states, faults, logs (nothing is sent) |
| `voloo ports` | Serial ports with USB metadata; opens nothing |
| `voloo mcp` | MCP server on stdio |
| `voloo api-info` | Local API address and token |
| `voloo daemon status\|stop` | The background process (outputs are never changed) |

Add `--json` for machine-readable output, `--direct` (or `--port PATH`) to open
the port without the background process. Exit codes:

| Code | Meaning |
| ---: | --- |
| 0 | Completed, or already in the requested state |
| 2 | The device reported failure (e.g. discharge timeout) |
| 3 | Outcome unknown, device offline or link failure — read `status` before any further action |
| 4 | Invalid argument or device not found |
| 5 | Busy (device busy, port in use, device locked, or nickname change while ON) |

A lost acknowledgement is never answered by resending a power command; the
outcome is confirmed by reading status or reported as unknown.

## Web dashboard

`voloo ui` opens the dashboard in your browser through a one-time link (valid for
60 s) that becomes a browser session; the page has no other way in. It shows
every connected Voloo with its state, VIN/VOUT (invalid readings are shown as
invalid, not 0 V), the last minute of VOUT, the last operation and active faults,
and updates live. Every switch asks for confirmation and shows the result the
device confirmed; an unknown outcome is highlighted. `voloo ui --no-open` prints
the link instead.

The dashboard uses the promo page's paper background, deep-purple outlines,
enclosure yellow and hard offset shadows. Hold a device's switch (or a button in
its detail sheet) for 0.6 s to change power; a short press only shows a hint.
Releasing early, losing focus or hiding the page cancels the hold. Unknown outcomes
require **Check state**, a fresh device read, before another power action.

English, Simplified Chinese and Traditional Chinese are available from the language
menu (the first visit follows the browser language). The appearance menu offers
system, light and dark modes. Both preferences are stored in this browser; the UI
loads no external fonts or services.

To try it without hardware: `voloo demo` starts a separate background process with
simulated devices (its own home, USB scanning off) and opens the dashboard.

## USB firmware maintenance

`voloo firmware update DEV` automatically downloads the encrypted firmware paired
with the CLI release, updates over USB, reconnects to the same UID and verifies
the application. The temporary ciphertext file is removed after success or
handled failure. Factory-initialized secure 3.0 boards are required for online
updates; keys and plaintext firmware are never included in npm.

The dashboard offers the same operation under **USB firmware**, plus optional
local files. A/B trials are confirmed after host checks; a failed trial rolls
back. `voloo firmware rollback DEV` verifies the previous image before confirming.
The daemon handles port ownership throughout. See [firmware updates](docs/firmware-updates.md)
for protection requirements, initial provisioning and recovery limits.

## AI agents (MCP)

Every MCP client that runs a stdio server works with the same command:

| Tool | Setup |
| --- | --- |
| Claude Code | `claude mcp add voloo -- voloo mcp` |
| Codex | `~/.codex/config.toml`: `[mcp_servers.voloo]` `command = "voloo"` `args = ["mcp"]` |
| Cursor, Windsurf, Cline, VS Code, Gemini CLI, … | `{"command": "voloo", "args": ["mcp"]}` |

Tools: `voloo_list_devices`, `voloo_get_status`, `voloo_power`
(on/off/discharge/cycle), `voloo_set_nickname`, `voloo_get_log`. Clients that
speak Streamable HTTP can use `POST http://127.0.0.1:<port>/mcp` with the token
from `voloo api-info`.

## Other programs (HTTP)

```sh
voloo api-info            # url, token, OpenAPI location
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:$PORT/v1/devices
curl -H "Authorization: Bearer $TOKEN" -X POST -d '{"action":"on"}' \
     -H 'Content-Type: application/json' http://127.0.0.1:$PORT/v1/devices/bench-a/power
curl -N -H "Authorization: Bearer $TOKEN" http://127.0.0.1:$PORT/v1/events   # live events
```

The full API is described at `/openapi.json`; generate a client for any language
from it, or read port and token from `$VOLOO_HOME/daemon.json` (they change when
the process restarts). Requests must come from 127.0.0.1 with the exact Host
header; browser requests from other origins are refused.

## Configuration

`$VOLOO_HOME/config.toml` (default `~/.voloo/config.toml`), all optional:

```toml
idle_exit_s = 600
[mcp]
read_only = true                        # agents may read but not switch
[devices."CD-AB-05-F3-69-BE-3B-5D"]
locked = true                           # no client may switch this device
```

## Logs

`$VOLOO_HOME/logs/` holds `operations-*.jsonl` (every write with its source —
cli, mcp, http — and before/after state, fsynced), `events-*.jsonl` (faults
raised, occurred and cleared) and `trace-*.jsonl` (one span per serial
transaction; background polling is traced only at `VOLOO_LOG_LEVEL=trace`).
Logs are kept for 14 days and 200 MB in total.

## License

[Apache-2.0](LICENSE)
