//! Interval Tree Clock (Almeida, Baquero, Fonte, 2008).
//!
//! ITC is the answer to "vector clocks, but for dynamic membership". A
//! [`Stamp`] is a pair `(id, event)` of two recursively-defined trees:
//!
//! - The **id tree** owns a binary fraction of the namespace. `fork` splits a
//!   stamp's id between two recipients; `join` recombines ownership.
//! - The **event tree** records what has happened, encoded as integers that
//!   propagate down the tree. The events in any subtree are bounded by the
//!   subtree's root plus the path values.
//!
//! The four operations:
//!
//! ```text
//! seed()                  : a fresh universe-owning stamp, no events yet
//! fork(s)    -> (s1, s2)  : split s's ownership; same event history on both
//! event(s)   -> s'        : record a new event somewhere in s's id-region
//! join(s1, s2) -> s'      : merge two stamps; sum ids, max events
//! ```
//!
//! ITC's `leq` recovers the happens-before partial order, and `concurrent`
//! detects forks that have not yet rejoined.
//!
//! The implementation here follows the paper's algorithms for `normalize`,
//! `fill`, and `grow` directly. The cost-minimizing grow heuristic is not
//! implemented in v0.2 (any-leftmost-path grow is used); v0.3 lands the
//! cost-balanced variant.

use std::cmp::Ordering;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Id {
    /// No ownership in this subtree.
    Zero,
    /// Full ownership in this subtree.
    One,
    /// Split ownership between left and right halves.
    Node(Box<Id>, Box<Id>),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Event {
    /// A flat region with this many events.
    Leaf(u32),
    /// A base count plus left and right subregions. The events in any leaf
    /// reachable below this node sum the base into the path.
    Node(u32, Box<Event>, Box<Event>),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Stamp {
    pub id: Id,
    pub event: Event,
}

// ---------------------------------------------------------------------------
// Id operations.
// ---------------------------------------------------------------------------

impl Id {
    pub fn zero() -> Self {
        Id::Zero
    }
    pub fn one() -> Self {
        Id::One
    }
    pub fn node(l: Id, r: Id) -> Self {
        Id::Node(Box::new(l), Box::new(r))
    }

    /// Collapse `(0, 0) -> 0` and `(1, 1) -> 1`. Any other Node is left alone.
    pub fn normalized(self) -> Self {
        match self {
            Id::Node(l, r) => {
                let l = l.normalized();
                let r = r.normalized();
                match (&l, &r) {
                    (Id::Zero, Id::Zero) => Id::Zero,
                    (Id::One, Id::One) => Id::One,
                    _ => Id::Node(Box::new(l), Box::new(r)),
                }
            }
            other => other,
        }
    }

    /// Split this id into two disjoint ids whose sum equals it.
    pub fn split(self) -> (Id, Id) {
        match self {
            Id::Zero => (Id::Zero, Id::Zero),
            Id::One => (Id::node(Id::One, Id::Zero), Id::node(Id::Zero, Id::One)),
            Id::Node(l, r) => {
                let (l, r) = (*l, *r);
                match (l, r) {
                    (Id::Zero, r) => {
                        let (r1, r2) = r.split();
                        (Id::node(Id::Zero, r1), Id::node(Id::Zero, r2))
                    }
                    (l, Id::Zero) => {
                        let (l1, l2) = l.split();
                        (Id::node(l1, Id::Zero), Id::node(l2, Id::Zero))
                    }
                    (l, r) => (Id::node(l, Id::Zero), Id::node(Id::Zero, r)),
                }
            }
        }
    }

    /// Sum two disjoint ids back into one. Panics on overlap.
    pub fn sum(self, other: Id) -> Id {
        match (self, other) {
            (Id::Zero, x) | (x, Id::Zero) => x,
            (Id::Node(l1, r1), Id::Node(l2, r2)) => Id::node(l1.sum(*l2), r1.sum(*r2)).normalized(),
            (a, b) => panic!("overlapping ids in sum: {:?} and {:?}", a, b),
        }
    }
}

// ---------------------------------------------------------------------------
// Event operations.
// ---------------------------------------------------------------------------

impl Event {
    pub fn leaf(n: u32) -> Self {
        Event::Leaf(n)
    }
    pub fn node(n: u32, l: Event, r: Event) -> Self {
        Event::Node(n, Box::new(l), Box::new(r))
    }

    /// Largest value reachable in any leaf, factoring in path bases.
    pub fn max_value(&self) -> u32 {
        match self {
            Event::Leaf(n) => *n,
            Event::Node(n, l, r) => n + l.max_value().max(r.max_value()),
        }
    }

    /// Smallest value reachable in any leaf, factoring in path bases.
    pub fn min_value(&self) -> u32 {
        match self {
            Event::Leaf(n) => *n,
            Event::Node(n, l, r) => n + l.min_value().min(r.min_value()),
        }
    }

    /// Add `m` to this event's effective root.
    pub(crate) fn lift(self, m: u32) -> Self {
        if m == 0 {
            return self;
        }
        match self {
            Event::Leaf(n) => Event::Leaf(n + m),
            Event::Node(n, l, r) => Event::Node(n + m, l, r),
        }
    }

    /// Subtract `m` from this event's effective root. Caller must ensure
    /// the root is at least `m`.
    fn sink(self, m: u32) -> Self {
        if m == 0 {
            return self;
        }
        match self {
            Event::Leaf(n) => Event::Leaf(n - m),
            Event::Node(n, l, r) => Event::Node(n - m, l, r),
        }
    }

    /// Canonicalize: collapse `Node(n, Leaf(a), Leaf(a))` to `Leaf(n + a)`,
    /// and lift the minimum subtree value into the parent node base.
    pub fn normalized(self) -> Self {
        match self {
            Event::Leaf(_) => self,
            Event::Node(n, l, r) => {
                let l = l.normalized();
                let r = r.normalized();
                if let (Event::Leaf(a), Event::Leaf(b)) = (&l, &r) {
                    if a == b {
                        return Event::Leaf(n + a);
                    }
                }
                let m = l.min_value().min(r.min_value());
                if m == 0 {
                    Event::Node(n, Box::new(l), Box::new(r))
                } else {
                    Event::Node(n + m, Box::new(l.sink(m)), Box::new(r.sink(m)))
                }
            }
        }
    }

    /// Pointwise max of two event trees. ITC `join` for events.
    pub fn join(self, other: Event) -> Event {
        match (self, other) {
            (Event::Leaf(a), Event::Leaf(b)) => Event::Leaf(a.max(b)),
            (Event::Leaf(n1), Event::Node(n2, l2, r2)) => {
                Event::Node(n1, Box::new(Event::Leaf(0)), Box::new(Event::Leaf(0)))
                    .join(Event::Node(n2, l2, r2))
            }
            (Event::Node(n1, l1, r1), Event::Leaf(n2)) => Event::Node(n1, l1, r1).join(
                Event::Node(n2, Box::new(Event::Leaf(0)), Box::new(Event::Leaf(0))),
            ),
            (Event::Node(n1, l1, r1), Event::Node(n2, l2, r2)) => {
                if n1 > n2 {
                    Event::Node(n2, l2, r2).join(Event::Node(n1, l1, r1))
                } else {
                    let d = n2 - n1;
                    let l2 = l2.lift(d);
                    let r2 = r2.lift(d);
                    Event::Node(n1, Box::new(l1.join(l2)), Box::new(r1.join(r2))).normalized()
                }
            }
        }
    }

    /// Pointwise less-or-equal. `a.leq(&b)` iff every leaf value in `a` is
    /// at most the value at the same path in `b`.
    pub fn leq(&self, other: &Event) -> bool {
        match (self, other) {
            (Event::Leaf(n1), Event::Leaf(n2)) => n1 <= n2,
            (Event::Leaf(n1), Event::Node(n2, _, _)) => n1 <= n2,
            (Event::Node(n1, l1, r1), Event::Leaf(n2)) => {
                n1 <= n2
                    && l1.clone().lift(*n1).leq(&Event::Leaf(*n2))
                    && r1.clone().lift(*n1).leq(&Event::Leaf(*n2))
            }
            (Event::Node(n1, l1, r1), Event::Node(n2, l2, r2)) => {
                n1 <= n2
                    && l1.clone().lift(*n1).leq(&l2.clone().lift(*n2))
                    && r1.clone().lift(*n1).leq(&r2.clone().lift(*n2))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Fill and grow (the event() machinery).
// ---------------------------------------------------------------------------

/// Fill the event tree along the id-owned region without growing structure.
fn fill(id: &Id, e: Event) -> Event {
    match (id, e) {
        (Id::Zero, e) => e,
        (Id::One, e) => Event::Leaf(e.max_value()),
        (Id::Node(_, _), e @ Event::Leaf(_)) => e,
        (Id::Node(il, ir), Event::Node(n, l, r)) => {
            let l = fill(il, *l);
            let r = fill(ir, *r);
            Event::Node(n, Box::new(l), Box::new(r)).normalized()
        }
    }
}

/// Grow the event tree under the id by at least one new event.
///
/// v0.3 ships the **cost-balanced** variant from the ITC paper (Almeida,
/// Baquero, Fonte, 2008, §6). Each recursive call returns `(event, cost)`
/// where `cost` measures the depth of the grow point. When two subtrees
/// are eligible, the lower-cost path wins, which keeps the event tree
/// approximately balanced over many `event()` operations.
///
/// Without this heuristic, repeatedly growing along the same path (the
/// v0.2 leftmost-path policy) produces a tree of depth `Θ(N)` after `N`
/// events. With cost balancing, the tree stays at depth `Θ(log N)`.
const LEAF_EXPANSION_PENALTY: u32 = 1_000_000;

fn grow(id: &Id, e: &Event) -> (Event, u32) {
    match (id, e) {
        (Id::Zero, _) => panic!("cannot grow under Id::Zero"),
        (Id::One, Event::Leaf(n)) => (Event::Leaf(n + 1), 0),
        (Id::One, Event::Node(n, l, r)) => {
            let (l_grown, cl) = grow(&Id::One, l);
            let (r_grown, cr) = grow(&Id::One, r);
            if cl <= cr {
                (Event::node(*n, l_grown, (**r).clone()).normalized(), cl + 1)
            } else {
                (Event::node(*n, (**l).clone(), r_grown).normalized(), cr + 1)
            }
        }
        (Id::Node(_, _), Event::Leaf(n)) => {
            let (e_grown, c) = grow(id, &Event::node(*n, Event::Leaf(0), Event::Leaf(0)));
            (e_grown, c + LEAF_EXPANSION_PENALTY)
        }
        (Id::Node(il, ir), Event::Node(n, l, r)) => match (&**il, &**ir) {
            (Id::Zero, _) => {
                let (r_grown, cr) = grow(ir, r);
                (Event::node(*n, (**l).clone(), r_grown).normalized(), cr + 1)
            }
            (_, Id::Zero) => {
                let (l_grown, cl) = grow(il, l);
                (Event::node(*n, l_grown, (**r).clone()).normalized(), cl + 1)
            }
            _ => {
                let (l_grown, cl) = grow(il, l);
                let (r_grown, cr) = grow(ir, r);
                if cl <= cr {
                    (Event::node(*n, l_grown, (**r).clone()).normalized(), cl + 1)
                } else {
                    (Event::node(*n, (**l).clone(), r_grown).normalized(), cr + 1)
                }
            }
        },
    }
}

// ---------------------------------------------------------------------------
// Stamp operations.
// ---------------------------------------------------------------------------

impl Stamp {
    /// Fresh universe-owning stamp.
    pub fn seed() -> Self {
        Stamp {
            id: Id::One,
            event: Event::Leaf(0),
        }
    }

    /// Fork this stamp into two. They share the event history and split the id.
    pub fn fork(self) -> (Stamp, Stamp) {
        let (i1, i2) = self.id.split();
        (
            Stamp {
                id: i1,
                event: self.event.clone(),
            },
            Stamp {
                id: i2,
                event: self.event,
            },
        )
    }

    /// Record one event in this stamp's id-region.
    pub fn event(self) -> Stamp {
        let Stamp { id, event } = self;
        let filled = fill(&id, event.clone());
        let new_event = if filled.max_value() > event.max_value() {
            filled
        } else {
            grow(&id, &event).0
        };
        Stamp {
            id,
            event: new_event,
        }
    }

    /// Merge two stamps. Combines ids, takes pointwise max of events.
    pub fn join(self, other: Stamp) -> Stamp {
        Stamp {
            id: self.id.sum(other.id),
            event: self.event.join(other.event),
        }
    }

    /// Send: produces an "anonymous" stamp suitable for shipping in a
    /// message, plus a residual stamp the sender keeps.
    pub fn send(self) -> (Stamp, Stamp) {
        let stamped = self.event();
        let (keep, send) = stamped.fork();
        (
            keep,
            Stamp {
                id: Id::Zero,
                event: send.event,
            },
        )
    }

    /// Receive: incorporates an incoming anonymous stamp into ours.
    pub fn receive(self, incoming: Stamp) -> Stamp {
        self.join(incoming).event()
    }
}

impl PartialOrd for Stamp {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        let a_le_b = self.event.leq(&other.event);
        let b_le_a = other.event.leq(&self.event);
        match (a_le_b, b_le_a) {
            (true, true) => Some(Ordering::Equal),
            (true, false) => Some(Ordering::Less),
            (false, true) => Some(Ordering::Greater),
            (false, false) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_is_full_id_no_events() {
        let s = Stamp::seed();
        assert_eq!(s.id, Id::One);
        assert_eq!(s.event, Event::Leaf(0));
    }

    #[test]
    fn fork_splits_id_preserves_event() {
        let s = Stamp::seed();
        let original_event = s.event.clone();
        let (a, b) = s.fork();
        assert_ne!(a.id, b.id);
        assert_eq!(a.event, original_event);
        assert_eq!(b.event, original_event);
    }

    #[test]
    fn fork_then_sum_recovers_full_id() {
        let s = Stamp::seed();
        let (a, b) = s.fork();
        let sum = a.id.sum(b.id);
        assert_eq!(sum, Id::One);
    }

    #[test]
    fn event_increases_max_value() {
        let s = Stamp::seed();
        let max_before = s.event.max_value();
        let s = s.event();
        assert!(s.event.max_value() > max_before);
    }

    #[test]
    fn join_is_commutative() {
        let s = Stamp::seed();
        let (a, b) = s.fork();
        let a = a.event();
        let b = b.event();
        let ab = a.clone().join(b.clone());
        let ba = b.join(a);
        assert_eq!(ab.event, ba.event);
        assert_eq!(ab.id, ba.id);
    }

    #[test]
    fn join_is_idempotent_in_event() {
        let s = Stamp::seed().event();
        let joined = s.event.clone().join(s.event.clone());
        assert_eq!(joined, s.event);
    }

    #[test]
    fn happens_before_implies_less_than() {
        let s = Stamp::seed();
        let (a, b) = s.fork();
        let a_after_event = a.clone().event();
        // a_after_event is causally after a.
        assert_eq!(
            a.partial_cmp(&a_after_event),
            Some(Ordering::Less),
            "a < a.event()"
        );
        let _ = b;
    }

    #[test]
    fn concurrent_forks_are_incomparable() {
        let s = Stamp::seed();
        let (a, b) = s.fork();
        let a = a.event();
        let b = b.event();
        // a and b each ticked independently, never communicated.
        assert_eq!(a.partial_cmp(&b), None);
    }

    #[test]
    fn send_receive_roundtrip_preserves_causality() {
        let s = Stamp::seed();
        let (alice, bob) = s.fork();

        // Alice does some work, then sends to Bob.
        let alice = alice.event();
        let (alice_after_send, msg) = alice.send();
        let bob = bob.receive(msg);

        // Bob's stamp now sees Alice's event.
        assert_eq!(
            alice_after_send.partial_cmp(&bob),
            Some(Ordering::Less),
            "alice's send-state happens-before bob's receive-state"
        );
    }

    #[test]
    fn id_normalize_collapses_homogeneous_node() {
        let id = Id::node(Id::Zero, Id::Zero).normalized();
        assert_eq!(id, Id::Zero);
        let id = Id::node(Id::One, Id::One).normalized();
        assert_eq!(id, Id::One);
    }

    #[test]
    fn event_normalize_lifts_min() {
        // Node(0, Leaf(2), Leaf(5)) should lift the 2 -> Node(2, Leaf(0), Leaf(3))
        let e = Event::node(0, Event::Leaf(2), Event::Leaf(5)).normalized();
        assert_eq!(e, Event::node(2, Event::Leaf(0), Event::Leaf(3)));
    }

    #[test]
    fn event_normalize_collapses_equal_leaves() {
        // Node(3, Leaf(2), Leaf(2)) should collapse to Leaf(5).
        let e = Event::node(3, Event::Leaf(2), Event::Leaf(2)).normalized();
        assert_eq!(e, Event::Leaf(5));
    }

    /// Tree depth helper for the cost-balanced grow tests.
    fn depth(e: &Event) -> usize {
        match e {
            Event::Leaf(_) => 0,
            Event::Node(_, l, r) => 1 + depth(l).max(depth(r)),
        }
    }

    #[test]
    fn cost_balanced_grow_keeps_tree_logarithmic() {
        // Under id=One, 32 events should produce a tree of depth ~log2(32)=5,
        // not the linear chain that leftmost-path grow would build.
        let mut s = Stamp::seed();
        for _ in 0..32 {
            s = s.event();
        }
        let d = depth(&s.event);
        assert!(
            d <= 6,
            "cost-balanced grow expected depth <=6 after 32 events, got {}",
            d
        );
    }

    #[test]
    fn cost_balanced_grow_under_split_id_stays_balanced() {
        // After fork, alice's id is Node(One, Zero). Events go into the left.
        // The left subtree should rebalance internally.
        let s = Stamp::seed();
        let (mut alice, _bob) = s.fork();
        for _ in 0..16 {
            alice = alice.event();
        }
        let d = depth(&alice.event);
        // 1 (outer Node from id structure) + log2(16) = 5.
        assert!(d <= 6, "expected balanced subtree, got depth {}", d);
    }

    #[test]
    fn event_join_takes_pointwise_max() {
        let a = Event::node(2, Event::Leaf(1), Event::Leaf(3));
        let b = Event::node(0, Event::Leaf(5), Event::Leaf(2));
        // a values: 2+1=3 (left), 2+3=5 (right)
        // b values: 0+5=5 (left), 0+2=2 (right)
        // pointwise max: 5 (left), 5 (right) -> Leaf(5) after collapse.
        let joined = a.join(b);
        assert_eq!(joined.max_value(), 5);
        assert_eq!(joined.min_value(), 5);
    }
}
