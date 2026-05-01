//! Lamport scalar clock (Lamport, 1978).
//!
//! Rule 1: increment on every local event.
//! Rule 2: on send, attach the current value.
//! Rule 3: on receive, set local to `max(local, received) + 1`.
//!
//! Property: if event `a` happens-before event `b`, then `clock(a) < clock(b)`.
//! The converse does not hold - Lamport is a total order extending the partial
//! happens-before order. Concurrent events get arbitrary tie-breaks.

use std::cmp::Ordering;

#[derive(Copy, Clone, Default, Debug, PartialEq, Eq, Hash)]
pub struct Lamport(u64);

impl Lamport {
    pub const fn new() -> Self {
        Self(0)
    }

    /// Construct directly from a raw u64. Used by serialization; do not
    /// use for ordinary clock state since it bypasses the happens-before
    /// machinery.
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// Local event. Returns the new clock value.
    pub fn tick(&mut self) -> Self {
        self.0 += 1;
        *self
    }

    /// Receipt of a message stamped `other`. Returns the new clock value.
    pub fn merge(&mut self, other: Self) -> Self {
        self.0 = self.0.max(other.0) + 1;
        *self
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl PartialOrd for Lamport {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Lamport {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_is_strictly_monotonic() {
        let mut c = Lamport::new();
        let a = c.tick();
        let b = c.tick();
        assert!(a < b);
    }

    #[test]
    fn merge_jumps_above_received_then_increments() {
        let mut a = Lamport::new();
        let mut b = Lamport::new();
        a.tick();
        a.tick();
        a.tick();
        b.tick();
        b.merge(a);
        assert_eq!(b.raw(), 4);
    }

    #[test]
    fn happens_before_implies_less_than() {
        // Two-node simulation.
        let mut p = Lamport::new();
        let mut q = Lamport::new();
        let send = p.tick();
        q.merge(send);
        let recv = q.tick();
        assert!(send < recv);
    }

    #[test]
    fn concurrent_two_thread_handshake_preserves_happens_before() {
        use std::sync::mpsc::channel;
        use std::thread;

        let (tx_ab, rx_ab) = channel::<Lamport>();
        let (tx_ba, rx_ba) = channel::<Lamport>();

        let h_a = thread::spawn(move || {
            let mut c = Lamport::new();
            let s1 = c.tick();
            tx_ab.send(s1).unwrap();
            let received = rx_ba.recv().unwrap();
            c.merge(received);
            let s2 = c.tick();
            (s1, s2)
        });
        let h_b = thread::spawn(move || {
            let mut c = Lamport::new();
            let received = rx_ab.recv().unwrap();
            c.merge(received);
            let s1 = c.tick();
            tx_ba.send(s1).unwrap();
            let s2 = c.tick();
            (s1, s2)
        });

        let (a1, a2) = h_a.join().unwrap();
        let (b1, b2) = h_b.join().unwrap();

        assert!(a1 < b1, "a1 ({:?}) < b1 ({:?})", a1, b1);
        assert!(b1 < a2, "b1 ({:?}) < a2 ({:?})", b1, a2);
        assert!(a1 < a2);
        assert!(b1 < b2);
    }
}
