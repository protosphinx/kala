//! MCP (Model Context Protocol) server for `kala`.
//!
//! Speaks stdio MCP (`protocolVersion 2024-11-05`) and exposes the
//! Lamport / HLC / Vector clock primitives as tools an agent can
//! call. Compiled into the `kala-mcp` binary when the crate is built
//! with `--features mcp`. The library API is unchanged; this module
//! only exists when the feature is on.
//!
//! State model: the server keeps a registry of named clocks
//! (`clock_id -> ClockEntry`). Tools that take `clock_id` create the
//! clock on first use. The registry is per-process, so it resets
//! when the binary restarts (i.e. each new MCP-client session).

use crate::{Hlc, Lamport, VectorClock};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "kala";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

// ---------------------------------------------------------------------------
// Clock registry
// ---------------------------------------------------------------------------

enum ClockEntry {
    Lamport(Lamport),
    Hlc(Hlc),
    Vector(VectorClock),
}

impl ClockEntry {
    fn kind(&self) -> &'static str {
        match self {
            ClockEntry::Lamport(_) => "lamport",
            ClockEntry::Hlc(_) => "hlc",
            ClockEntry::Vector(_) => "vector",
        }
    }

    fn to_json(&self) -> Value {
        match self {
            ClockEntry::Lamport(c) => json!({ "kind": "lamport", "value": c.raw() }),
            ClockEntry::Hlc(c) => json!({ "kind": "hlc", "pt": c.pt, "l": c.l }),
            ClockEntry::Vector(c) => {
                let counts: Vec<u64> = (0..c.n_nodes()).map(|i| c.get(i)).collect();
                json!({ "kind": "vector", "counts": counts, "n_nodes": c.n_nodes() })
            }
        }
    }
}

static REGISTRY: OnceLock<Mutex<HashMap<String, ClockEntry>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, ClockEntry>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

// ---------------------------------------------------------------------------
// JSON-RPC / MCP envelope
// ---------------------------------------------------------------------------

/// Handle one line of JSON-RPC input. Returns `None` for notifications,
/// `Some(json)` otherwise.
pub fn handle_request(line: &str) -> Option<String> {
    let request: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            return Some(error_response(
                Value::Null,
                -32700,
                &format!("Parse error: {e}"),
            ));
        }
    };

    let method = request["method"].as_str().unwrap_or("");
    let params = &request["params"];
    let id = request.get("id").cloned();
    let is_notification = id.as_ref().map(|v| v.is_null()).unwrap_or(true);
    let id_value = id.unwrap_or(Value::Null);

    match method {
        "initialize" => Some(success_response(
            id_value,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION },
            }),
        )),

        "notifications/initialized" | "initialized" | "notifications/cancelled" => None,

        "ping" => Some(success_response(id_value, json!({}))),

        "tools/list" => Some(success_response(
            id_value,
            json!({ "tools": tool_definitions() }),
        )),

        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match dispatch(name, &args) {
                Ok(result) => {
                    let text =
                        serde_json::to_string_pretty(&result).unwrap_or_else(|_| String::new());
                    Some(success_response(
                        id_value,
                        json!({
                            "content": [{ "type": "text", "text": text }],
                        }),
                    ))
                }
                Err(e) => Some(success_response(
                    id_value,
                    json!({
                        "content": [{ "type": "text", "text": e }],
                        "isError": true,
                    }),
                )),
            }
        }

        _ => {
            if is_notification {
                None
            } else {
                Some(error_response(
                    id_value,
                    -32601,
                    &format!("Method not found: {method}"),
                ))
            }
        }
    }
}

fn success_response(id: Value, result: Value) -> String {
    serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    }))
    .unwrap_or_default()
}

fn error_response(id: Value, code: i32, message: &str) -> String {
    serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    }))
    .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Tool dispatch
// ---------------------------------------------------------------------------

fn dispatch(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "kala_lamport_tick" => tool_lamport_tick(args),
        "kala_lamport_merge" => tool_lamport_merge(args),
        "kala_lamport_compare" => tool_lamport_compare(args),
        "kala_hlc_send" => tool_hlc_send(args),
        "kala_hlc_recv" => tool_hlc_recv(args),
        "kala_hlc_compare" => tool_hlc_compare(args),
        "kala_vector_tick" => tool_vector_tick(args),
        "kala_vector_merge" => tool_vector_merge(args),
        "kala_vector_compare" => tool_vector_compare(args),
        "kala_inspect" => tool_inspect(args),
        "kala_reset" => tool_reset(args),
        _ => Err(format!("Unknown tool: {name}")),
    }
}

// ---------- helpers ----------

fn get_clock_id(args: &Value) -> Result<String, String> {
    args["clock_id"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "Missing 'clock_id' parameter (expected a string)".to_string())
}

fn get_u64(args: &Value, key: &str) -> Result<u64, String> {
    args[key]
        .as_u64()
        .ok_or_else(|| format!("Missing or non-integer '{key}' parameter"))
}

fn get_usize(args: &Value, key: &str) -> Result<usize, String> {
    let v = get_u64(args, key)?;
    usize::try_from(v).map_err(|_| format!("'{key}' is too large for usize"))
}

fn get_array_u64(args: &Value, key: &str) -> Result<Vec<u64>, String> {
    args[key]
        .as_array()
        .ok_or_else(|| format!("Missing '{key}' parameter (expected array of integers)"))?
        .iter()
        .map(|v| {
            v.as_u64()
                .ok_or_else(|| format!("'{key}' contains a non-integer"))
        })
        .collect()
}

fn ordering_str(o: std::cmp::Ordering) -> &'static str {
    match o {
        std::cmp::Ordering::Less => "less",
        std::cmp::Ordering::Equal => "equal",
        std::cmp::Ordering::Greater => "greater",
    }
}

fn type_mismatch(expected: &str, got: &str, clock_id: &str) -> String {
    format!("clock_id '{clock_id}' already exists with kind '{got}'; tool expected '{expected}'")
}

// ---------- Lamport ----------

fn tool_lamport_tick(args: &Value) -> Result<Value, String> {
    let id = get_clock_id(args)?;
    let mut reg = registry().lock().unwrap();
    let entry = reg
        .entry(id.clone())
        .or_insert(ClockEntry::Lamport(Lamport::new()));
    match entry {
        ClockEntry::Lamport(c) => {
            let new = c.tick();
            Ok(json!({ "clock_id": id, "value": new.raw() }))
        }
        other => Err(type_mismatch("lamport", other.kind(), &id)),
    }
}

fn tool_lamport_merge(args: &Value) -> Result<Value, String> {
    let id = get_clock_id(args)?;
    let received = get_u64(args, "received")?;
    let mut reg = registry().lock().unwrap();
    let entry = reg
        .entry(id.clone())
        .or_insert(ClockEntry::Lamport(Lamport::new()));
    match entry {
        ClockEntry::Lamport(c) => {
            let new = c.merge(Lamport::from_raw(received));
            Ok(json!({ "clock_id": id, "value": new.raw() }))
        }
        other => Err(type_mismatch("lamport", other.kind(), &id)),
    }
}

fn tool_lamport_compare(args: &Value) -> Result<Value, String> {
    let a = get_u64(args, "a")?;
    let b = get_u64(args, "b")?;
    let ord = a.cmp(&b);
    Ok(json!({
        "ordering": ordering_str(ord),
        "happens_before_implication": "a < b means 'a happens-before b' is consistent (Lamport gives a total order extending happens-before; equal values mean concurrent or tie)",
    }))
}

// ---------- HLC ----------

fn parse_hlc(v: &Value, key: &str) -> Result<Hlc, String> {
    let pt = v[key]["pt"]
        .as_u64()
        .ok_or_else(|| format!("'{key}.pt' missing or non-integer"))?;
    let l_u64 = v[key]["l"]
        .as_u64()
        .ok_or_else(|| format!("'{key}.l' missing or non-integer"))?;
    let l = u16::try_from(l_u64).map_err(|_| format!("'{key}.l' must fit in u16"))?;
    let mut h = Hlc::new(pt);
    h.l = l;
    Ok(h)
}

fn hlc_to_json(h: Hlc) -> Value {
    json!({ "pt": h.pt, "l": h.l })
}

fn tool_hlc_send(args: &Value) -> Result<Value, String> {
    let id = get_clock_id(args)?;
    let now = get_u64(args, "now")?;
    let mut reg = registry().lock().unwrap();
    let entry = reg
        .entry(id.clone())
        .or_insert(ClockEntry::Hlc(Hlc::new(0)));
    match entry {
        ClockEntry::Hlc(c) => {
            let new = c.send(now);
            Ok(json!({ "clock_id": id, "stamp": hlc_to_json(new) }))
        }
        other => Err(type_mismatch("hlc", other.kind(), &id)),
    }
}

fn tool_hlc_recv(args: &Value) -> Result<Value, String> {
    let id = get_clock_id(args)?;
    let now = get_u64(args, "now")?;
    let received = parse_hlc(args, "received")?;
    let mut reg = registry().lock().unwrap();
    let entry = reg
        .entry(id.clone())
        .or_insert(ClockEntry::Hlc(Hlc::new(0)));
    match entry {
        ClockEntry::Hlc(c) => {
            let new = c.recv(received, now);
            Ok(json!({ "clock_id": id, "stamp": hlc_to_json(new) }))
        }
        other => Err(type_mismatch("hlc", other.kind(), &id)),
    }
}

fn tool_hlc_compare(args: &Value) -> Result<Value, String> {
    let a = parse_hlc(args, "a")?;
    let b = parse_hlc(args, "b")?;
    Ok(json!({ "ordering": ordering_str(a.cmp(&b)) }))
}

// ---------- Vector ----------

fn tool_vector_tick(args: &Value) -> Result<Value, String> {
    let id = get_clock_id(args)?;
    let node = get_usize(args, "node")?;
    let n_nodes_opt = args
        .get("n_nodes")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);

    let mut reg = registry().lock().unwrap();
    if !reg.contains_key(&id) {
        let n = n_nodes_opt.ok_or_else(|| {
            format!("clock_id '{id}' is new; you must pass 'n_nodes' the first time")
        })?;
        if node >= n {
            return Err(format!("'node' ({node}) must be < n_nodes ({n})"));
        }
        reg.insert(id.clone(), ClockEntry::Vector(VectorClock::new(n)));
    }
    let entry = reg.get_mut(&id).unwrap();
    match entry {
        ClockEntry::Vector(c) => {
            if node >= c.n_nodes() {
                return Err(format!(
                    "'node' ({node}) must be < this clock's n_nodes ({})",
                    c.n_nodes()
                ));
            }
            c.tick(node);
            let counts: Vec<u64> = (0..c.n_nodes()).map(|i| c.get(i)).collect();
            Ok(json!({ "clock_id": id, "counts": counts, "n_nodes": c.n_nodes() }))
        }
        other => Err(type_mismatch("vector", other.kind(), &id)),
    }
}

fn tool_vector_merge(args: &Value) -> Result<Value, String> {
    let id = get_clock_id(args)?;
    let local = get_usize(args, "local")?;
    let received = get_array_u64(args, "received")?;

    let mut reg = registry().lock().unwrap();
    if !reg.contains_key(&id) {
        reg.insert(
            id.clone(),
            ClockEntry::Vector(VectorClock::new(received.len())),
        );
    }
    let entry = reg.get_mut(&id).unwrap();
    match entry {
        ClockEntry::Vector(c) => {
            if c.n_nodes() != received.len() {
                return Err(format!(
                    "size mismatch: clock has n_nodes={}, received has length {}",
                    c.n_nodes(),
                    received.len()
                ));
            }
            if local >= c.n_nodes() {
                return Err(format!(
                    "'local' ({local}) must be < n_nodes ({})",
                    c.n_nodes()
                ));
            }
            let other = VectorClock::from_counts(received);
            c.merge(&other, local);
            let counts: Vec<u64> = (0..c.n_nodes()).map(|i| c.get(i)).collect();
            Ok(json!({ "clock_id": id, "counts": counts, "n_nodes": c.n_nodes() }))
        }
        other => Err(type_mismatch("vector", other.kind(), &id)),
    }
}

fn tool_vector_compare(args: &Value) -> Result<Value, String> {
    let a = get_array_u64(args, "a")?;
    let b = get_array_u64(args, "b")?;
    if a.len() != b.len() {
        return Err(format!(
            "size mismatch: a has length {}, b has length {}",
            a.len(),
            b.len()
        ));
    }
    let va = VectorClock::from_counts(a);
    let vb = VectorClock::from_counts(b);
    let ordering = match va.partial_cmp(&vb) {
        Some(o) => ordering_str(o),
        None => "concurrent",
    };
    Ok(json!({ "ordering": ordering }))
}

// ---------- inspection / management ----------

fn tool_inspect(args: &Value) -> Result<Value, String> {
    let reg = registry().lock().unwrap();
    if let Some(id_val) = args.get("clock_id") {
        let id = id_val
            .as_str()
            .ok_or_else(|| "'clock_id' must be a string when provided".to_string())?;
        match reg.get(id) {
            Some(c) => Ok(json!({ "clock_id": id, "state": c.to_json() })),
            None => Err(format!("clock_id '{id}' not found")),
        }
    } else {
        let mut clocks: Vec<Value> = reg
            .iter()
            .map(|(id, c)| json!({ "clock_id": id, "state": c.to_json() }))
            .collect();
        clocks.sort_by(|a, b| a["clock_id"].as_str().cmp(&b["clock_id"].as_str()));
        Ok(json!({ "clocks": clocks, "count": reg.len() }))
    }
}

fn tool_reset(args: &Value) -> Result<Value, String> {
    let mut reg = registry().lock().unwrap();
    if let Some(id_val) = args.get("clock_id") {
        let id = id_val
            .as_str()
            .ok_or_else(|| "'clock_id' must be a string when provided".to_string())?;
        let removed = reg.remove(id).is_some();
        Ok(json!({ "removed": removed, "clock_id": id }))
    } else {
        let count = reg.len();
        reg.clear();
        Ok(json!({ "removed_count": count }))
    }
}

// ---------------------------------------------------------------------------
// Tool definitions for `tools/list`
// ---------------------------------------------------------------------------

fn clock_id_prop() -> Value {
    json!({ "type": "string", "description": "Identifier for a named clock in the per-process registry. Created on first use." })
}

fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "kala_lamport_tick",
            "description": "Tick a named Lamport scalar clock (created on first use). Returns the new u64 value. Use this for local events.",
            "inputSchema": {
                "type": "object",
                "properties": { "clock_id": clock_id_prop() },
                "required": ["clock_id"],
            }
        }),
        json!({
            "name": "kala_lamport_merge",
            "description": "Merge a received Lamport timestamp into a named clock: local = max(local, received) + 1. Use this on message receipt.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clock_id": clock_id_prop(),
                    "received": { "type": "integer", "minimum": 0, "description": "Lamport timestamp attached to the incoming message." },
                },
                "required": ["clock_id", "received"],
            }
        }),
        json!({
            "name": "kala_lamport_compare",
            "description": "Compare two Lamport timestamps. Returns 'less' / 'equal' / 'greater'. Stateless. Lamport gives a total order extending happens-before; 'equal' on different events means concurrent.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "a": { "type": "integer", "minimum": 0 },
                    "b": { "type": "integer", "minimum": 0 },
                },
                "required": ["a", "b"],
            }
        }),
        json!({
            "name": "kala_hlc_send",
            "description": "Local / send event on a named Hybrid Logical Clock. `now` is the current wall-clock reading (any monotonic unit; ms by convention). Returns the new HLC stamp { pt, l }.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clock_id": clock_id_prop(),
                    "now": { "type": "integer", "minimum": 0, "description": "Wall-clock reading at the local node (e.g. ms since epoch)." },
                },
                "required": ["clock_id", "now"],
            }
        }),
        json!({
            "name": "kala_hlc_recv",
            "description": "Receipt of a remote HLC stamp on a named clock. `now` is the local wall clock; `received` is the incoming stamp. Returns the new HLC stamp.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clock_id": clock_id_prop(),
                    "now": { "type": "integer", "minimum": 0 },
                    "received": {
                        "type": "object",
                        "properties": {
                            "pt": { "type": "integer", "minimum": 0 },
                            "l":  { "type": "integer", "minimum": 0, "maximum": 65535 },
                        },
                        "required": ["pt", "l"],
                    },
                },
                "required": ["clock_id", "now", "received"],
            }
        }),
        json!({
            "name": "kala_hlc_compare",
            "description": "Compare two HLC stamps. Returns 'less' / 'equal' / 'greater'. Stateless. Order is lexicographic on (pt, l).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "a": { "type": "object", "properties": { "pt": {"type":"integer"}, "l": {"type":"integer"} }, "required": ["pt","l"] },
                    "b": { "type": "object", "properties": { "pt": {"type":"integer"}, "l": {"type":"integer"} }, "required": ["pt","l"] },
                },
                "required": ["a", "b"],
            }
        }),
        json!({
            "name": "kala_vector_tick",
            "description": "Tick a named vector clock at `node`. The first call to a clock_id must include `n_nodes` (the fixed group size). Returns the new counts array.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clock_id": clock_id_prop(),
                    "node": { "type": "integer", "minimum": 0, "description": "Index of the local node within the fixed group." },
                    "n_nodes": { "type": "integer", "minimum": 1, "description": "Group size. Required on first use of clock_id; ignored thereafter." },
                },
                "required": ["clock_id", "node"],
            }
        }),
        json!({
            "name": "kala_vector_merge",
            "description": "Merge a received vector into a named clock: pointwise max, then tick at `local`. Sizes must match. The clock is created if new (sized from `received`).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clock_id": clock_id_prop(),
                    "received": { "type": "array", "items": {"type":"integer", "minimum": 0}, "description": "Incoming vector." },
                    "local": { "type": "integer", "minimum": 0, "description": "Index of the local node (the one receiving the message)." },
                },
                "required": ["clock_id", "received", "local"],
            }
        }),
        json!({
            "name": "kala_vector_compare",
            "description": "Compare two vector clocks of equal length. Returns 'less' / 'equal' / 'greater' / 'concurrent'. Stateless. Vector clocks make happens-before a true partial order: 'concurrent' means neither dominates.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "a": { "type": "array", "items": {"type":"integer", "minimum": 0} },
                    "b": { "type": "array", "items": {"type":"integer", "minimum": 0} },
                },
                "required": ["a", "b"],
            }
        }),
        json!({
            "name": "kala_inspect",
            "description": "Inspect the registry. Pass `clock_id` to inspect one clock; omit it to list all named clocks with their current state.",
            "inputSchema": {
                "type": "object",
                "properties": { "clock_id": { "type": "string" } },
            }
        }),
        json!({
            "name": "kala_reset",
            "description": "Drop named clocks from the registry. Pass `clock_id` to drop one; omit it to drop everything.",
            "inputSchema": {
                "type": "object",
                "properties": { "clock_id": { "type": "string" } },
            }
        }),
    ]
}

#[cfg(test)]
#[doc(hidden)]
pub fn _reset_for_test() {
    registry().lock().unwrap().clear();
}
