# GOALS - kala

Sequenced milestones to a complete distributed-time toolkit.

## v0.0 - Lamport + HLC ✦ **shipped**

- `Lamport`: tick / merge / `Ord`. Tests for strict monotonicity and happens-before.
- `Hlc`: send / recv / `Ord`. Tests for the four merge-rule branches (eq pt; local-wins; remote-wins; now-wins).

## v0.1 - Vector clock ✦ **shipped**

- `VectorClock` over a fixed-size group: tick / merge / `join_no_tick`
- `PartialOrd::partial_cmp` returns `None` for concurrent stamps
- Tests: pointwise-max merge, happens-before across two and three nodes,
  detection of concurrent ticks across non-communicating processes

## v0.2 - Interval Tree Clock ✦ **shipped**

- `Id` enum (Zero / One / Node) with split, sum, normalize
- `Event` enum (Leaf / Node) with min-lift normalize, lift / sink, join, leq
- `Stamp` with seed / fork / event / join / send / receive
- `fill` and (leftmost-path) `grow` for the event() operation
- Tests: fork-then-sum recovers full id, event increases max-value, join is
  commutative and idempotent in event, happens-before implies less-than,
  concurrent forks are incomparable, send/receive preserves causality

## v0.3 - cost-balanced grow ✦ **shipped**

- Cost-tracking `grow` from Almeida, Baquero, Fonte (2008, §6)
- Each recursive call returns `(event, cost)`; the lower-cost subtree wins
- Leaf-to-node expansion penalty keeps the grower in existing structure
- Tests: 32 sequential events under id=One produce a tree of depth <=6;
  16 events under a forked id stay balanced

## v0.4 - concurrent handshake tests ✦ **shipped**

- Lamport: two-thread send/reply chain; transitive happens-before holds
- HLC: skewed-wall-clock two-thread handshake preserves causality
- VectorClock: three-node chain via mpsc preserves partial order; disjoint
  workers on shared Mutex<VectorClock> remain incomparable

## v0.5 - worked CRDT example ✦ **shipped**

- `LwwRegister<T>` over an ITC `Stamp` with `new`, `fork`, `write`, `merge`
- Concurrent writes resolved via user-supplied tiebreak closure
- Tests: linear chain (later wins), one-sided write (later stamp dominates),
  concurrent writes (tiebreak fires), associative merge under max tiebreak,
  post-merge write strictly dominates both inputs

## v0.6 - wire-format serialization ✦ **shipped**

- `Wire` trait with `encode -> Vec<u8>` and
  `decode(&[u8]) -> Option<(Self, &[u8])>`
- Implementations for `Lamport`, `Hlc`, `VectorClock`, `Id`, `Event`, `Stamp`
- Recursive ITC trees use tagged-union encoding
- Tests: round-trip for every type incl. zero, max, post-fork stamps;
  truncated input rejected; invalid tag rejected; concatenated decodes
  leave correct remainder

## v0.7 - racy concurrency proofs

- `loom`-checked tests: HLC monotonicity under concurrent send/recv from multiple threads on the same clock
- The "logical counter overflow" path under degenerate clock stalls

## v0.8 - formal proofs

- TLA+ specification of HLC matching the implementation
- Machine-checked proof of the four properties: monotonicity, causality, bounded drift, eventual convergence
- A separate `proofs/` directory; `cargo test` and `tlc` both gate releases

## Non-goals

- Physical clock synchronization (NTP, PTP) - kala consumes a `now: u64`, never produces one
- TrueTime / Spanner-style commit-wait - out of scope; consumers compose this on top
- `serde` integration - the hand-rolled `Wire` format is what kala ships;
  callers who want serde can wrap `encode`/`decode`
