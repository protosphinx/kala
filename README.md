<h1 align="center">kala</h1>

<p align="center"><em>काल - time itself, as a Rust crate.</em></p>

---

Distributed-systems logical-time primitives. The full small-zoo: scalar Lamport clocks, Hybrid Logical Clocks, Interval Tree Clocks. One trait surface, three semantics, all with the property you actually want - *if `a` happens-before `b`, then `clock(a) < clock(b)`*.

## Why a small zoo

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
| 0.2 | Interval Tree Clock (fork / event / join)        | next   |
| 0.3 | Loom-checked monotonicity under concurrent ops   |        |
| 0.4 | TLA+ proof obligations + machine-checked proofs  |        |

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
```

## License

MIT.

---

<p align="center"><a href="https://x.com/protosphinx">@protosphinx</a></p>
