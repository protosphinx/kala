//! Vector clock (Fidge, 1988; Mattern, 1988).
//!
//! Each of `n` nodes maintains a `Vec<u64>` of length `n`: entry `i` is the
//! count of events node `i` has ever observed (directly or transitively). The
//! update rules are exactly those of Lamport, lifted to the vector:
//!
//! ```text
//! local event on node i:    v[i] += 1
//! receive remote vector w:  v[k] = max(v[k], w[k])  for all k
//!                           v[local] += 1
//! ```
//!
//! The point of paying `O(n)` per stamp: vector clocks capture happens-before
//! *exactly*. Two stamps `a` and `b` are concurrent iff neither dominates the
//! other component-wise — and in that case [`PartialOrd::partial_cmp`]
//! returns `None`, which is the cleanest possible API for "we cannot order
//! these events causally."
//!
//! v0.1 ships fixed-size vector clocks (the membership list is decided at
//! construction time). Dynamic membership lands at v0.2 via Interval Tree
//! Clocks, which are the right tool when nodes join and leave.

use std::cmp::Ordering;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VectorClock {
    counts: Vec<u64>,
}

impl VectorClock {
    /// Construct a zero clock for an `n`-node group.
    pub fn new(n_nodes: usize) -> Self {
        Self {
            counts: vec![0; n_nodes],
        }
    }

    pub fn n_nodes(&self) -> usize {
        self.counts.len()
    }

    pub fn get(&self, node: usize) -> u64 {
        self.counts[node]
    }

    /// Local event on `node`. Returns `&self` for chaining.
    pub fn tick(&mut self, node: usize) -> &Self {
        self.counts[node] += 1;
        self
    }

    /// Receipt of a remote vector. Takes pointwise max, then ticks `local`.
    /// Both clocks must have been constructed with the same `n_nodes`.
    pub fn merge(&mut self, other: &Self, local: usize) -> &Self {
        assert_eq!(
            self.counts.len(),
            other.counts.len(),
            "vector-clock size mismatch: {} vs {}",
            self.counts.len(),
            other.counts.len()
        );
        for (s, o) in self.counts.iter_mut().zip(other.counts.iter()) {
            *s = (*s).max(*o);
        }
        self.counts[local] += 1;
        self
    }

    /// Read-only pointwise-max merge for use cases where no local tick should
    /// be applied (e.g. observer combining gossiped vectors).
    pub fn join_no_tick(&mut self, other: &Self) -> &Self {
        assert_eq!(self.counts.len(), other.counts.len());
        for (s, o) in self.counts.iter_mut().zip(other.counts.iter()) {
            *s = (*s).max(*o);
        }
        self
    }
}

impl PartialOrd for VectorClock {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        assert_eq!(
            self.counts.len(),
            other.counts.len(),
            "cannot order vector clocks of different membership size"
        );
        let mut le = true;
        let mut ge = true;
        for (a, b) in self.counts.iter().zip(other.counts.iter()) {
            if a < b {
                ge = false;
            }
            if a > b {
                le = false;
            }
            if !le && !ge {
                return None;
            }
        }
        match (le, ge) {
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
    fn ticks_advance_only_local_entry() {
        let mut c = VectorClock::new(3);
        c.tick(1);
        c.tick(1);
        assert_eq!(c.get(0), 0);
        assert_eq!(c.get(1), 2);
        assert_eq!(c.get(2), 0);
    }

    #[test]
    fn merge_takes_pointwise_max_and_ticks_local() {
        let mut a = VectorClock::new(3);
        a.tick(0);
        a.tick(0);
        a.tick(0); // [3, 0, 0]
        let mut b = VectorClock::new(3);
        b.tick(1);
        b.tick(1); // [0, 2, 0]
        b.merge(&a, 1); // [3, 3, 0]
        assert_eq!(b.get(0), 3);
        assert_eq!(b.get(1), 3);
        assert_eq!(b.get(2), 0);
    }

    #[test]
    fn happens_before_implies_less_than() {
        let mut p = VectorClock::new(2);
        let mut q = VectorClock::new(2);
        p.tick(0); // p = [1, 0]
        let send = p.clone();
        q.merge(&send, 1); // q = [1, 1]
        let recv = q.clone();
        assert_eq!(send.partial_cmp(&recv), Some(Ordering::Less));
    }

    #[test]
    fn concurrent_events_are_incomparable() {
        // Two nodes, each ticks locally without communication.
        let mut p = VectorClock::new(2);
        let mut q = VectorClock::new(2);
        p.tick(0); // p = [1, 0]
        q.tick(1); // q = [0, 1]
        assert_eq!(p.partial_cmp(&q), None);
        assert_eq!(q.partial_cmp(&p), None);
    }

    #[test]
    fn equal_vectors_compare_equal() {
        let mut p = VectorClock::new(3);
        let mut q = VectorClock::new(3);
        p.tick(0);
        p.tick(2);
        q.tick(0);
        q.tick(2);
        assert_eq!(p.partial_cmp(&q), Some(Ordering::Equal));
    }

    #[test]
    fn three_node_concurrency_pattern() {
        // Node 0 sends to 1; meanwhile 2 ticks alone.
        let mut p0 = VectorClock::new(3);
        let mut p1 = VectorClock::new(3);
        let mut p2 = VectorClock::new(3);

        p0.tick(0);
        let send_0_to_1 = p0.clone();
        p1.merge(&send_0_to_1, 1); // p1 has seen p0's event
        p2.tick(2); // p2 ticks independently

        // p1 and p2 never communicated → concurrent.
        assert_eq!(p1.partial_cmp(&p2), None);
        // p0's send is strictly before p1's post-merge state.
        assert_eq!(send_0_to_1.partial_cmp(&p1), Some(Ordering::Less));
    }

    #[test]
    fn join_no_tick_does_not_increment_counter() {
        let mut a = VectorClock::new(2);
        let mut b = VectorClock::new(2);
        a.tick(0);
        b.tick(1);
        a.join_no_tick(&b);
        assert_eq!(a.get(0), 1);
        assert_eq!(a.get(1), 1);
    }
}
