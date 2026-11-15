# MCP client compatibility

Checked on 2026-11-15 on macOS arm64, with installed Voloo host
`0.7.0-rc.6` and one USB-C X035 board running firmware `2.7`. These
cross-client results describe that setup. A separate host run also verified all
five tools through Codex against firmware `3.0`; the OpenCode and VS Code
evidence below remains limited to firmware `2.7`.
Client versions and test depth matter: a valid configuration is not evidence
that an agent has called a tool successfully.

## Evidence levels

| Client | Evidence from this workstation | Limits |
| --- | --- | --- |
| Codex desktop | Actual calls to all five tools; power, discharge, both cycle modes, nickname changes and restoration were exercised in the preceding hardware run | Other Codex versions and operating systems were not tested |
| OpenCode `1.14.19` | Actual stdio connection; a local deterministic model fixture drove the installed client's device-list, status, log and missing-device calls against the real Voloo service | The model and its tool requests were simulated; the installed client dispatched them to the real service and USB device. No write calls were made through OpenCode |
| VS Code `1.141.0`, Local harness | Actual launch from `.vscode/mcp.json`; its output reported `Running` and `Discovered 5 tools` | No agent tool call: the isolated profile was not signed into Copilot. Native Copilot harness configuration was checked against documentation only |
| Claude Code `2.1.273` / Claude Desktop `5.11492.5` | Configuration documentation checked; installed versions identified | Model testing was skipped at the user's request because Claude had no quota. This run does not establish connection or call compatibility |
| Cursor | Official configuration checked | No installed application or executable found; no connection or tool call tested |
| Gemini CLI | Official configuration checked | No executable found; no connection or tool call tested |
| Cline | Official configuration checked | No CLI or VS Code extension found; no connection or tool call tested |
| Windsurf | Not verified | No installed client found. Its former official MCP documentation URL redirected to an inaccessible page during this check |

The test board's VIN was about 0.12 V. The hardware evidence confirms protocol,
control state and readback behavior; it does not validate switching a powered
load or discharging stored load energy. Cross-client checks were read-only and
left the board OFF with its original nickname.

## Findings that affect setup

### OpenCode V1 and V2 use different configuration layouts

The installed V1 client accepted and connected with this layout:

```json
{
  "mcp": {
    "voloo": {
      "type": "local",
      "command": ["voloo", "mcp"],
      "enabled": true
    }
  }
}
```

Current [OpenCode documentation](https://opencode.ai/docs/mcp-servers/) describes
V2, which nests definitions under `mcp.servers` and uses `disabled` rather than
`enabled`. This V2 layout was checked against the documentation, not executed:

```json
{
  "mcp": {
    "servers": {
      "voloo": {
        "type": "local",
        "command": ["voloo", "mcp"]
      }
    }
  }
}
```

OpenCode exposes names such as `voloo_voloo_get_status`. In V1's observed event
stream, a missing-device call had client execution state `completed`, while its
returned payload still contained `error.code = "not_found"`. Read the returned
error and device outcome; client execution state alone does not establish a
successful device action.

### VS Code Local and Copilot use different destinations

The actual Local-harness check used workspace `.vscode/mcp.json`:

```json
{
  "servers": {
    "voloo": {
      "type": "stdio",
      "command": "voloo",
      "args": ["mcp"]
    }
  }
}
```

The current [VS Code configuration reference](https://code.visualstudio.com/docs/agents/reference/mcp-configuration)
uses a top-level `mcpServers` object for the native Copilot harness, in
workspace-root `.mcp.json` or `~/.copilot/mcp-config.json` (unless
`COPILOT_HOME` changes the user directory):

```json
{
  "mcpServers": {
    "voloo": {
      "type": "stdio",
      "command": "voloo",
      "args": ["mcp"],
      "tools": ["*"]
    }
  }
}
```

This Copilot configuration is documentation-checked only. Do not describe the
Local-harness discovery result as a successful Copilot agent call.

## Configuration checks for the other clients

Merge entries into existing configuration files; retain other servers and
permission settings. GUI applications may need an absolute executable path.
The actual OpenCode and VS Code runs used `/opt/homebrew/bin/voloo` on this Mac;
use the path reported by `command -v voloo` on the target machine.

| Client | Configuration destination or command | Reference |
| --- | --- | --- |
| Codex | `codex mcp add voloo -- voloo mcp`, or `[mcp_servers.voloo]` in `~/.codex/config.toml` with `command = "voloo"`, `args = ["mcp"]` | [Official Codex commands](https://learn.chatgpt.com/docs/developer-commands#codex-mcp) |
| Claude Code | `claude mcp add --transport stdio voloo -- voloo mcp` | [Claude Code MCP](https://code.claude.com/docs/en/mcp) |
| Claude Desktop | `mcpServers.voloo` with `command: "voloo"`, `args: ["mcp"]` in `claude_desktop_config.json` | [MCP local-server guide](https://modelcontextprotocol.io/docs/develop/connect-local-servers) |
| Cursor | `~/.cursor/mcp.json` or project `.cursor/mcp.json`; `mcpServers.voloo` with `type: "stdio"`, `command: "voloo"`, `args: ["mcp"]` | [Cursor MCP](https://cursor.com/docs/mcp) |
| Gemini CLI | `~/.gemini/settings.json` or project `.gemini/settings.json`; `mcpServers.voloo` with `command: "voloo"`, `args: ["mcp"]` | [Gemini CLI MCP](https://geminicli.com/docs/tools/mcp-server/) |
| Cline | Use Configure MCP Servers in the extension, or CLI `~/.cline/data/settings/cline_mcp_settings.json`; `mcpServers.voloo` with `command: "voloo"`, `args: ["mcp"]` | [Cline MCP](https://docs.cline.bot/mcp/mcp-overview) |

The five Voloo tools and their device-side rules are shared across clients.
Each client still controls its own tool approvals, naming and presentation.
Keep ordinary approval prompts for power and nickname changes. A follow-up
check on any untested client should first confirm tool discovery and an actual
device-list/status call, then report write coverage separately.
