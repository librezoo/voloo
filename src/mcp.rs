//! Model Context Protocol server (JSON-RPC 2.0). Transport-independent: the background
//! process serves it at `POST /mcp` (Streamable HTTP, JSON responses) and `voloo mcp`
//! relays stdio lines to it. Five tools keep agents from guessing.
use crate::{
    device::{ErrorKind, OpError},
    home,
    manager::{Job, Manager},
    observe::read_log,
    protocol::{Command, Uid},
};
use serde_json::{Value, json};
use std::sync::Arc;

pub const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "Voloo switches externally supplied DC power to real loads over USB. \
Call voloo_list_devices first and use the exact UID or nickname it returns. Change power only on \
the device and in the way the user asked. Results are confirmed by reading the device back: \
'completed' or 'already_satisfied' are success. If an action returns outcome 'unknown', read \
voloo_get_status before any further action and never repeat the command blindly. OFF does not \
mean the load is at 0 V; use action 'discharge' to confirm VOUT <= 1 V. Measurements that are \
null are invalid, not zero. Each fault carries a suggestion: follow it and tell the user.";

fn tools() -> Value {
    let device = json!({"type": "string", "description": "UID (e.g. CD-AB-05-F3-69-BE-3B-5D) or unique nickname, as listed by voloo_list_devices."});
    json!([
        {
            "name": "voloo_list_devices",
            "title": "List Voloo devices",
            "description": "List every Voloo connected to this computer with UID, nickname, online state, output state, VIN/VOUT in mV and active faults. Offline devices have an unknown output state. Does not change anything.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}
        },
        {
            "name": "voloo_get_status",
            "title": "Read Voloo status",
            "description": "Read the device now: output state, VIN/VOUT in mV (null = invalid measurement), last command result and active faults with suggested next steps. Optionally include identity/firmware info and diagnostics. The device may be omitted when exactly one Voloo is connected.",
            "inputSchema": {"type": "object", "properties": {
                "device": device,
                "include": {"type": "array", "items": {"type": "string", "enum": ["info", "communication", "adc"]}, "description": "Extra blocks to read."}
            }, "additionalProperties": false},
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}
        },
        {
            "name": "voloo_power",
            "title": "Switch Voloo power",
            "description": "Switch the real load connected to one Voloo and confirm the result by reading it back. on: output ON. off: output OFF (the load may keep its voltage). discharge: OFF and actively discharge until VOUT <= 1 V is confirmed (5 s device limit). cycle: discharge (or plain OFF with discharge=false), wait off_ms, then ON; stops at the first failure and never turns ON after an error. A lost reply is never answered by resending: the outcome is read back or reported as 'unknown'.",
            "inputSchema": {"type": "object", "properties": {
                "device": device,
                "action": {"type": "string", "enum": ["on", "off", "discharge", "cycle"]},
                "off_ms": {"type": "integer", "minimum": 0, "maximum": 30000, "default": 1000, "description": "cycle only: time OFF before ON."},
                "discharge": {"type": "boolean", "default": true, "description": "cycle only: confirm discharge before waiting."}
            }, "required": ["device", "action"], "additionalProperties": false},
            "annotations": {"readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": false}
        },
        {
            "name": "voloo_set_nickname",
            "title": "Rename a Voloo",
            "description": "Store a persistent nickname (up to 24 UTF-8 bytes, empty string clears it) and verify it by reading back. Requires the output fully OFF and idle. The UID never changes.",
            "inputSchema": {"type": "object", "properties": {
                "device": device,
                "nickname": {"type": "string"}
            }, "required": ["device", "nickname"], "additionalProperties": false},
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}
        },
        {
            "name": "voloo_get_log",
            "title": "Read Voloo logs",
            "description": "Recent records to explain what happened. operations: every power/nickname action with before/after state (use it to check whether an action already ran). faults: raised, occurred and cleared faults with suggestions. trace: serial transactions of one operation (pass trace_id from a result or error).",
            "inputSchema": {"type": "object", "properties": {
                "kind": {"type": "string", "enum": ["operations", "faults", "trace"], "default": "operations"},
                "device": device,
                "trace_id": {"type": "string"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 500, "default": 20}
            }, "additionalProperties": false},
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}
        }
    ])
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// Handles one JSON-RPC message or batch. `None` means no response (notifications only).
pub async fn handle(manager: &Arc<Manager>, message: Value) -> Option<Value> {
    if let Value::Array(batch) = message {
        let mut out = Vec::new();
        for m in batch {
            if let Some(r) = Box::pin(handle(manager, m)).await {
                out.push(r);
            }
        }
        return (!out.is_empty()).then_some(Value::Array(out));
    }
    let id = message.get("id").cloned();
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        // A response from the client (we never send requests) or garbage.
        return id.map(|id| rpc_error(id, -32600, "Invalid request"));
    };
    let id = id?; // notifications (e.g. notifications/initialized) get no response
    let params = message.get("params").cloned().unwrap_or(json!({}));
    let result = match method {
        "initialize" => {
            let asked = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or("");
            let version = PROTOCOL_VERSIONS
                .iter()
                .find(|v| **v == asked)
                .copied()
                .unwrap_or(PROTOCOL_VERSIONS[0]);
            json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "voloo", "title": "Voloo", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools": tools()}),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match call_tool(manager, name, &args).await {
                Ok(v) => tool_result(v, false),
                Err(ToolError::Unknown) => {
                    return Some(rpc_error(id, -32602, &format!("Unknown tool {name:?}")));
                }
                Err(ToolError::Op(e)) => tool_result(json!({"error": e}), true),
            }
        }
        _ => {
            return Some(rpc_error(
                id,
                -32601,
                &format!("Method not found: {method}"),
            ));
        }
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn tool_result(value: Value, is_error: bool) -> Value {
    json!({
        "content": [{"type": "text", "text": serde_json::to_string(&value).unwrap()}],
        "structuredContent": value,
        "isError": is_error
    })
}

enum ToolError {
    Unknown,
    Op(OpError),
}
impl From<OpError> for ToolError {
    fn from(e: OpError) -> Self {
        ToolError::Op(e)
    }
}

fn bad(message: impl Into<String>) -> ToolError {
    ToolError::Op(OpError::new(ErrorKind::InvalidArgument, message))
}

async fn call_tool(m: &Arc<Manager>, name: &str, args: &Value) -> Result<Value, ToolError> {
    let device = args.get("device").and_then(Value::as_str);
    let writes = matches!(name, "voloo_power" | "voloo_set_nickname");
    if writes && m.config.mcp_read_only {
        return Err(ToolError::Op(OpError::new(
            ErrorKind::Busy,
            "MCP is read-only by configuration (config.toml [mcp] read_only = true); no command was sent",
        )));
    }
    match name {
        "voloo_list_devices" => {
            let devices: Vec<Value> = m
                .devices()
                .into_iter()
                .map(|d| {
                    json!({
                        "uid": d.uid, "nickname": d.nickname, "online": d.online, "locked": d.locked,
                        "power_state": d.status.as_ref().filter(|_| d.online).map(|s| s.power_state.clone()),
                        "vin_mv": d.status.as_ref().filter(|_| d.online).and_then(|s| s.vin_mv),
                        "vout_mv": d.status.as_ref().filter(|_| d.online).and_then(|s| s.vout_mv),
                        "observed_at": d.observed_at,
                        "faults": d.faults.iter().map(|f| json!({"code": f.code, "active": f.active, "message": f.message, "suggestion": f.suggestion})).collect::<Vec<_>>(),
                        "error": d.error.map(|e| e.message),
                    })
                })
                .collect();
            Ok(json!({"devices": devices}))
        }
        "voloo_get_status" => {
            let mut status = m.call(device, Job::Status, "mcp").await?;
            let uid = status["uid"].as_str().map(str::to_string);
            for part in args
                .get("include")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let job = match part.as_str() {
                    Some("info") => Job::Info,
                    Some(k @ ("communication" | "adc")) => Job::Diagnostics(k.into()),
                    _ => return Err(bad("include items must be info, communication or adc")),
                };
                let key = part.as_str().unwrap().to_string();
                status[key] = m.call(uid.as_deref(), job, "mcp").await?;
            }
            Ok(status)
        }
        "voloo_power" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| bad("action is required"))?;
            let job = match action {
                "on" => Job::Power(Command::On),
                "off" => Job::Power(Command::Off),
                "discharge" => Job::Power(Command::Discharge),
                "cycle" => {
                    let off_ms = args
                        .get("off_ms")
                        .map(|v| v.as_u64().ok_or_else(|| bad("off_ms must be an integer")))
                        .transpose()?
                        .unwrap_or(1000);
                    if off_ms > 30_000 {
                        return Err(bad("off_ms must be 0..30000"));
                    }
                    let discharge = args
                        .get("discharge")
                        .map(|v| {
                            v.as_bool()
                                .ok_or_else(|| bad("discharge must be a boolean"))
                        })
                        .transpose()?
                        .unwrap_or(true);
                    Job::Cycle { off_ms, discharge }
                }
                _ => return Err(bad("action must be on, off, discharge or cycle")),
            };
            if device.is_none() {
                return Err(bad("device is required for power actions"));
            }
            Ok(m.call(device, job, "mcp").await?)
        }
        "voloo_set_nickname" => {
            let nickname = args
                .get("nickname")
                .and_then(Value::as_str)
                .ok_or_else(|| bad("nickname is required"))?;
            if device.is_none() {
                return Err(bad("device is required"));
            }
            Ok(m.call(device, Job::Nickname(nickname.into()), "mcp")
                .await?)
        }
        "voloo_get_log" => {
            let kind = args
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("operations");
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(20)
                .clamp(1, 500) as usize;
            let uid = device
                .map(|d| {
                    d.parse::<Uid>()
                        .map(|u| u.to_string())
                        .or_else(|_| m.resolve(Some(d)).map(|u| u.to_string()))
                })
                .transpose()?;
            match kind {
                "trace" => {
                    let trace_id = args.get("trace_id").and_then(Value::as_str);
                    let spans: Vec<Value> = m
                        .recorder
                        .spans(trace_id, limit)
                        .into_iter()
                        .filter(|s| uid.as_deref().is_none_or(|u| s.uid == u))
                        .map(|s| json!(s))
                        .collect();
                    Ok(json!({"kind": "trace", "records": spans}))
                }
                "operations" | "faults" => {
                    let dir = home::logs().ok_or_else(|| {
                        ToolError::Op(OpError::new(ErrorKind::NotFound, "No Voloo home directory"))
                    })?;
                    let stem = if kind == "faults" {
                        "events"
                    } else {
                        "operations"
                    };
                    Ok(
                        json!({"kind": kind, "records": read_log(&dir, stem, uid.as_deref(), limit)}),
                    )
                }
                _ => Err(bad("kind must be operations, faults or trace")),
            }
        }
        _ => Err(ToolError::Unknown),
    }
}
