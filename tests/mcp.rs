//! Integration tests for the `mcp` module.
//!
//! Run with `cargo test --features mcp --test mcp`.
//!
//! Tests share the global REGISTRY, so the suite uses unique
//! `clock_id`s per test (we never assume the registry starts empty).

#![cfg(feature = "mcp")]

use kala::mcp::handle_request;
use serde_json::{json, Value};

fn req(method: &str, id: u64, params: Value) -> String {
    serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    }))
    .unwrap()
}

fn notif(method: &str) -> String {
    serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "method": method,
    }))
    .unwrap()
}

fn call(name: &str, args: Value) -> Value {
    let raw = handle_request(&req(
        "tools/call",
        1,
        json!({ "name": name, "arguments": args }),
    ))
    .expect("tools/call always responds");
    serde_json::from_str(&raw).expect("valid JSON")
}

fn ok(name: &str, args: Value) -> Value {
    let resp = call(name, args);
    let res = &resp["result"];
    assert!(
        !res["isError"].as_bool().unwrap_or(false),
        "tool '{}' returned isError: {}",
        name,
        res["content"][0]["text"]
    );
    let text = res["content"][0]["text"]
        .as_str()
        .expect("content[0].text is a string");
    serde_json::from_str(text).expect("tool returns JSON inside text")
}

fn err(name: &str, args: Value) -> String {
    let resp = call(name, args);
    let res = &resp["result"];
    assert_eq!(res["isError"], true, "expected isError=true for {name}");
    res["content"][0]["text"].as_str().unwrap().to_string()
}

// ---------- protocol envelope ----------

#[test]
fn initialize_returns_protocol_version_and_server_info() {
    let raw = handle_request(&req(
        "initialize",
        1,
        json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "tester", "version": "0.1" },
        }),
    ))
    .unwrap();
    let v: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(v["result"]["serverInfo"]["name"], "kala");
    assert!(v["result"]["capabilities"]["tools"].is_object());
}

#[test]
fn notifications_initialized_yields_no_response() {
    assert!(handle_request(&notif("notifications/initialized")).is_none());
}

#[test]
fn ping_returns_empty_result() {
    let raw = handle_request(&req("ping", 2, json!({}))).unwrap();
    let v: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["result"], json!({}));
}

#[test]
fn tools_list_returns_eleven_tools_with_object_schemas() {
    let raw = handle_request(&req("tools/list", 3, json!({}))).unwrap();
    let v: Value = serde_json::from_str(&raw).unwrap();
    let tools = v["result"]["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 11, "expected 11 tools, got {}", tools.len());

    let expected: std::collections::HashSet<&str> = [
        "kala_lamport_tick",
        "kala_lamport_merge",
        "kala_lamport_compare",
        "kala_hlc_send",
        "kala_hlc_recv",
        "kala_hlc_compare",
        "kala_vector_tick",
        "kala_vector_merge",
        "kala_vector_compare",
        "kala_inspect",
        "kala_reset",
    ]
    .into_iter()
    .collect();
    let got: std::collections::HashSet<&str> =
        tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(got, expected);

    for t in tools {
        assert_eq!(t["inputSchema"]["type"], "object", "tool {}", t["name"]);
    }
}

#[test]
fn malformed_json_yields_parse_error() {
    let raw = handle_request("{not json").unwrap();
    let v: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["error"]["code"], -32700);
}

#[test]
fn unknown_method_yields_method_not_found() {
    let raw = handle_request(&req("totally/made/up", 9, json!({}))).unwrap();
    let v: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["error"]["code"], -32601);
}

#[test]
fn unknown_tool_yields_is_error_content() {
    let msg = err("kala_nonexistent", json!({}));
    assert!(msg.contains("Unknown tool"));
}

// ---------- Lamport ----------

#[test]
fn lamport_tick_creates_then_increments() {
    let a = ok(
        "kala_lamport_tick",
        json!({ "clock_id": "lamport_tick_test_a" }),
    );
    let b = ok(
        "kala_lamport_tick",
        json!({ "clock_id": "lamport_tick_test_a" }),
    );
    assert_eq!(a["value"], 1);
    assert_eq!(b["value"], 2);
}

#[test]
fn lamport_merge_jumps_above_received_and_increments() {
    let _ = ok(
        "kala_lamport_tick",
        json!({ "clock_id": "lamport_merge_test" }),
    );
    let v = ok(
        "kala_lamport_merge",
        json!({ "clock_id": "lamport_merge_test", "received": 10 }),
    );
    assert_eq!(v["value"], 11);
}

#[test]
fn lamport_compare_returns_less_equal_greater() {
    assert_eq!(
        ok("kala_lamport_compare", json!({ "a": 1, "b": 2 }))["ordering"],
        "less"
    );
    assert_eq!(
        ok("kala_lamport_compare", json!({ "a": 5, "b": 5 }))["ordering"],
        "equal"
    );
    assert_eq!(
        ok("kala_lamport_compare", json!({ "a": 9, "b": 2 }))["ordering"],
        "greater"
    );
}

#[test]
fn lamport_missing_clock_id_yields_is_error() {
    let msg = err("kala_lamport_tick", json!({}));
    assert!(msg.contains("clock_id"));
}

// ---------- HLC ----------

#[test]
fn hlc_send_advances_pt_when_now_strictly_advances() {
    let s1 = ok(
        "kala_hlc_send",
        json!({ "clock_id": "hlc_send_test", "now": 100 }),
    );
    let s2 = ok(
        "kala_hlc_send",
        json!({ "clock_id": "hlc_send_test", "now": 200 }),
    );
    assert_eq!(s1["stamp"]["pt"], 100);
    assert_eq!(s1["stamp"]["l"], 0);
    assert_eq!(s2["stamp"]["pt"], 200);
    assert_eq!(s2["stamp"]["l"], 0);
}

#[test]
fn hlc_send_increments_l_when_now_does_not_advance() {
    let _ = ok(
        "kala_hlc_send",
        json!({ "clock_id": "hlc_l_test", "now": 100 }),
    );
    let s2 = ok(
        "kala_hlc_send",
        json!({ "clock_id": "hlc_l_test", "now": 100 }),
    );
    assert_eq!(s2["stamp"]["pt"], 100);
    assert_eq!(s2["stamp"]["l"], 1);
}

#[test]
fn hlc_recv_takes_max_of_pt_then_handles_l() {
    let _ = ok(
        "kala_hlc_send",
        json!({ "clock_id": "hlc_recv_test", "now": 100 }),
    );
    let s = ok(
        "kala_hlc_recv",
        json!({
            "clock_id": "hlc_recv_test",
            "now": 50,
            "received": { "pt": 200, "l": 5 },
        }),
    );
    assert_eq!(s["stamp"]["pt"], 200);
    assert_eq!(s["stamp"]["l"], 6);
}

#[test]
fn hlc_compare_orders_lexicographically() {
    assert_eq!(
        ok(
            "kala_hlc_compare",
            json!({ "a": { "pt": 100, "l": 5 }, "b": { "pt": 100, "l": 6 } })
        )["ordering"],
        "less"
    );
    assert_eq!(
        ok(
            "kala_hlc_compare",
            json!({ "a": { "pt": 100, "l": 5 }, "b": { "pt": 99,  "l": 9 } })
        )["ordering"],
        "greater"
    );
    assert_eq!(
        ok(
            "kala_hlc_compare",
            json!({ "a": { "pt": 100, "l": 5 }, "b": { "pt": 100, "l": 5 } })
        )["ordering"],
        "equal"
    );
}

// ---------- Vector ----------

#[test]
fn vector_tick_requires_n_nodes_on_first_use() {
    let msg = err(
        "kala_vector_tick",
        json!({ "clock_id": "vector_first_test", "node": 0 }),
    );
    assert!(msg.contains("n_nodes"));
}

#[test]
fn vector_tick_then_tick_increments_one_index() {
    let v1 = ok(
        "kala_vector_tick",
        json!({ "clock_id": "vector_tick_test", "node": 1, "n_nodes": 3 }),
    );
    let v2 = ok(
        "kala_vector_tick",
        json!({ "clock_id": "vector_tick_test", "node": 1 }),
    );
    assert_eq!(v1["counts"], json!([0, 1, 0]));
    assert_eq!(v2["counts"], json!([0, 2, 0]));
}

#[test]
fn vector_merge_takes_pointwise_max_then_ticks_local() {
    let _ = ok(
        "kala_vector_tick",
        json!({ "clock_id": "vector_merge_test", "node": 0, "n_nodes": 3 }),
    );
    let merged = ok(
        "kala_vector_merge",
        json!({
            "clock_id": "vector_merge_test",
            "received": [0, 5, 2],
            "local": 0,
        }),
    );
    // pointwise max([1,0,0], [0,5,2]) = [1,5,2], then tick at 0 -> [2,5,2]
    assert_eq!(merged["counts"], json!([2, 5, 2]));
}

#[test]
fn vector_compare_detects_concurrent_stamps() {
    let r = ok(
        "kala_vector_compare",
        json!({ "a": [1, 0, 0], "b": [0, 1, 0] }),
    );
    assert_eq!(r["ordering"], "concurrent");
}

#[test]
fn vector_compare_orders_when_one_dominates() {
    let r = ok(
        "kala_vector_compare",
        json!({ "a": [1, 1, 0], "b": [2, 1, 0] }),
    );
    assert_eq!(r["ordering"], "less");
}

#[test]
fn vector_compare_size_mismatch_returns_is_error() {
    let msg = err(
        "kala_vector_compare",
        json!({ "a": [1, 0], "b": [0, 1, 0] }),
    );
    assert!(msg.contains("size mismatch"));
}

// ---------- registry ----------

#[test]
fn type_collision_on_clock_id_returns_clear_error() {
    let _ = ok(
        "kala_lamport_tick",
        json!({ "clock_id": "collision_test_id" }),
    );
    let msg = err(
        "kala_hlc_send",
        json!({ "clock_id": "collision_test_id", "now": 100 }),
    );
    assert!(
        msg.contains("already exists with kind 'lamport'"),
        "got: {msg}"
    );
}

#[test]
fn inspect_specific_clock_returns_state() {
    let _ = ok(
        "kala_lamport_tick",
        json!({ "clock_id": "inspect_test_id" }),
    );
    let r = ok("kala_inspect", json!({ "clock_id": "inspect_test_id" }));
    assert_eq!(r["state"]["kind"], "lamport");
    assert_eq!(r["state"]["value"], 1);
}

#[test]
fn inspect_unknown_clock_returns_is_error() {
    let msg = err(
        "kala_inspect",
        json!({ "clock_id": "definitely_not_real_xyz" }),
    );
    assert!(msg.contains("not found"));
}

#[test]
fn reset_specific_clock_removes_it() {
    let _ = ok(
        "kala_lamport_tick",
        json!({ "clock_id": "reset_specific_test" }),
    );
    let r = ok("kala_reset", json!({ "clock_id": "reset_specific_test" }));
    assert_eq!(r["removed"], true);
    let r2 = ok("kala_reset", json!({ "clock_id": "reset_specific_test" }));
    assert_eq!(r2["removed"], false);
}
