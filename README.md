<h1 align="center">kala</h1>

<p align="center"><em>काल - time itself, as a Rust crate.</em></p>

---

Distributed-systems logical-time primitives. The full small-zoo: scalar Lamport clocks, Hybrid Logical Clocks, Interval Tree Clocks. One trait surface, three semantics, all with the property you actually want - *if `a` happens-before `b`, then `clock(a) < clock(b)`*.

## Why a small zoo

![tests](https://img.shields.io/badge/tests-81%20passing-yellowgreen)

There is no one right clock. The right clock is determined by what you are willing to give up:

| Clock                 | Size        | Captures HB | Detects concurrency | Wall-clock aware |
|-----------------------|-------------|-------------|---------------------|------------------|
| **Lamport scalar**    | 8 bytes     | yes (one-way) | no                | no               |
| **Hybrid Logical**    | 10 bytes    | yes (one-way) | no                | yes              |
| **Vector clock**      | O(n) bytes  | yes (iff)   | yes                 | no               |
| **Interval Tree**     | O(log n)    | yes (iff)   | yes                 | no               |

Lamport is the substrate (Lamport, 1978). HLC is what you actually want for replicated databases (Kulkarni et al., 2014 - used in CockroachDB and YugabyteDB). ITC is the answer to "vector clocks but with reasonable size in dynamic membership" (Almeida et al., 2008).

This crate ships them carefully - same trait, same testing rigor, same `Ord` semantics - so you can pick by use case without rewriting anything around them.

## Status

| v   | Surface                                          | Status |
|-----|--------------------------------------------------|--------|
| 0.0 | Lamport + HLC, full `Ord`, monotonicity tests    | **shipped** |
| 0.1 | Vector clock with `PartialOrd::None` for concurrent stamps | **shipped** |
| 0.2 | Interval Tree Clock (fork / event / join / leq)  | **shipped** |
| 0.3 | Cost-balanced grow heuristic (logarithmic event-tree depth) | **shipped** |
| 0.4 | std::thread + mpsc concurrent-handshake tests across all clock types | **shipped** |
| 0.5 | Worked CRDT example: ITC-stamped LWW register with concurrent-write tiebreak | **shipped** |
| 0.6 | Wire-format serialization for every clock type incl. recursive ITC trees | **shipped** |
| 0.7 | `Replica<T>` + `Network<T>` causal-broadcast simulator over ITC stamps | **shipped** |
| 0.8 | Loom-checked monotonicity under racy concurrent ops | next |
| 0.9 | TLA+ proof obligations + machine-checked proofs  |        |

## Use

```rust
use kala::{Hlc, Lamport, VectorClock};

// Lamport - happens-before via send / merge.
let mut p = Lamport::new();
let mut q = Lamport::new();
let stamp = p.tick();
q.merge(stamp);
assert!(stamp < q.tick());

// HLC - wall-clock-aware, survives skew.
let mut clock = Hlc::new(now_ms());
let send = clock.send(now_ms());
clock.recv(remote_hlc, now_ms());

// Vector clock - concurrent events are *incomparable*, not tie-broken.
let mut p = VectorClock::new(2);
let mut q = VectorClock::new(2);
p.tick(0);
q.tick(1);
assert_eq!(p.partial_cmp(&q), None);  // concurrent

// ITC - vector clocks for dynamic membership.
use kala::Stamp;
let s = Stamp::seed();
let (alice, bob) = s.fork();
let alice = alice.event();
let (alice, msg) = alice.send();
let bob = bob.receive(msg);
assert_eq!(alice.partial_cmp(&bob), Some(std::cmp::Ordering::Less));
```

## MCP Server

`kala` ships a [Model Context Protocol](https://modelcontextprotocol.io/) server so any MCP-compatible agent ([Claude Code](https://docs.anthropic.com/en/docs/claude-code), [Cursor](https://cursor.sh/), [Windsurf](https://codeium.com/windsurf), [Continue](https://continue.dev/), …) can drive the clocks as tools. Useful for "is event A before event B?", "merge these two vector clocks", "did this protocol preserve causality?" without writing a custom adapter.

The server is gated behind the `mcp` Cargo feature so the library's dep tree stays empty by default. Build the binary once:

```bash
cargo build --release --features mcp --bin kala-mcp
```

Register with Claude Code:

```bash
claude mcp add --scope user kala "$(pwd)/target/release/kala-mcp"
claude mcp list   # kala: ... - ✓ Connected
```

For Cursor / Windsurf / any other MCP client, drop this into their `mcpServers` config:

```json
{
  "mcpServers": {
    "kala": {
      "command": "/abs/path/to/kala/target/release/kala-mcp"
    }
  }
}
```

The server keeps a per-process registry of named clocks (`clock_id -> Lamport | Hlc | Vector`) so multi-step simulations across tool calls share state. The registry resets when the MCP client session ends.

### Tools

| Tool | What it does |
|------|--------------|
| `kala_lamport_tick` | Tick a named Lamport clock (created on first use); returns the new u64. |
| `kala_lamport_merge` | Merge a received Lamport timestamp: `local = max(local, received) + 1`. |
| `kala_lamport_compare` | Stateless compare of two Lamport values: `less` / `equal` / `greater`. |
| `kala_hlc_send` | Local / send event on a named HLC; takes `now`, returns `{pt, l}`. |
| `kala_hlc_recv` | Receipt on a named HLC; takes `now` + `received: {pt, l}`, returns the new stamp. |
| `kala_hlc_compare` | Stateless lexicographic compare on `(pt, l)`. |
| `kala_vector_tick` | Tick a named vector clock at `node`; first call requires `n_nodes`. |
| `kala_vector_merge` | Pointwise-max merge then tick at `local`. Sizes must match. |
| `kala_vector_compare` | Stateless compare; returns `less` / `equal` / `greater` / `concurrent`. |
| `kala_inspect` | Inspect one (`clock_id`) or all named clocks in the registry. |
| `kala_reset` | Drop one (`clock_id`) or all clocks from the registry. |

### Using it

In Claude Code, after registering, talk to Claude in English:

> **You:** Use kala to walk me through what happens when Alice sends a message to Bob, then Bob replies, with Lamport clocks.
>
> *(Claude calls `kala_lamport_tick` for Alice's send, `kala_lamport_merge` for Bob's receive, then again for Alice's reply, and reads back the timestamps showing the happens-before chain.)*
>
> **You:** Now repeat with vector clocks across three nodes, and tell me which pairs of events end up concurrent.
>
> *(Claude builds a `kala_vector_tick` / `kala_vector_merge` story, then `kala_vector_compare`s pairs to spot `"concurrent"` results — the thing scalar Lamport clocks cannot express.)*

You can force a tool ("use `kala_hlc_compare`…") but normally describing the scenario in English is faster.

ITC stamps and the `Wire`/`Replica`/`Network` types are not exposed as MCP tools yet — they're recursive enums whose JSON representation needs more design. They remain available via the library API.

## License

MIT.

---

<p align="center"><a href="https://x.com/protosphinx">@protosphinx</a></p>
