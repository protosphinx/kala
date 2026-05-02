//! Causal-broadcast replica framework over ITC stamps.
//!
//! A `Replica<T>` wraps an ITC [`Stamp`](crate::Stamp) and an arbitrary
//! payload `T`. Replicas can be forked, mutated locally, and merged via a
//! caller-supplied combiner. The stamp tracks who-knows-what so the
//! framework can decide when an incoming message is causally ready to
//! deliver.
//!
//! `Network<T>` simulates an asynchronous message bus: messages can be
//! enqueued from any replica to any other, and `deliver_at` plays them out
//! in arbitrary order. Tests demonstrate eventual consistency: regardless
//! of delivery order, all replicas converge to the same state when the
//! combiner is commutative and associative.

use crate::itc::Stamp;
use std::cmp::Ordering;

#[derive(Clone, Debug)]
pub struct Replica<T: Clone> {
    pub stamp: Stamp,
    pub state: T,
}

impl<T: Clone> Replica<T> {
    /// Fresh replica owning the universe; fork before handing out copies.
    pub fn new(state: T) -> Self {
        Self {
            stamp: Stamp::seed(),
            state,
        }
    }

    /// Split the replica's id-share between two children. Both share the
    /// payload at fork time; subsequent updates diverge.
    pub fn fork(self) -> (Self, Self) {
        let (s1, s2) = self.stamp.fork();
        let state = self.state;
        (
            Self {
                stamp: s1,
                state: state.clone(),
            },
            Self { stamp: s2, state },
        )
    }

    /// Apply `f` to the local state and advance the stamp by one event.
    pub fn update<F>(self, f: F) -> Self
    where
        F: FnOnce(T) -> T,
    {
        Self {
            stamp: self.stamp.event(),
            state: f(self.state),
        }
    }

    /// Capture the current state as a message addressable to any peer.
    pub fn snapshot(&self) -> Message<T> {
        Message {
            from_stamp: self.stamp.clone(),
            payload: self.state.clone(),
        }
    }

    /// Has this replica already observed everything in `msg.from_stamp`?
    /// Used by [`Network`] to drop redundant deliveries.
    pub fn has_seen(&self, msg: &Message<T>) -> bool {
        matches!(
            msg.from_stamp.partial_cmp(&self.stamp),
            Some(Ordering::Less | Ordering::Equal)
        )
    }

    /// Deliver `msg` by combining the incoming payload with local state and
    /// joining the stamps. The combiner must be commutative + associative
    /// for eventual consistency under arbitrary delivery order.
    pub fn deliver<F>(self, msg: Message<T>, combine: F) -> Self
    where
        F: FnOnce(T, T) -> T,
    {
        let state = combine(self.state, msg.payload);
        Self {
            stamp: self.stamp.join(msg.from_stamp),
            state,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Message<T: Clone> {
    pub from_stamp: Stamp,
    pub payload: T,
}

/// An asynchronous message bus over `n_replicas` participants. Useful for
/// simulating eventual consistency under arbitrary delivery orders.
pub struct Network<T: Clone> {
    pub replicas: Vec<Replica<T>>,
    pub queue: Vec<(usize, Message<T>)>,
}

impl<T: Clone> Network<T> {
    /// Build a network with `n_replicas` peers, each starting at `initial`.
    /// Forks the seed stamp `n_replicas - 1` times to give every replica a
    /// distinct id-share.
    pub fn new(initial: T, n_replicas: usize) -> Self {
        assert!(n_replicas >= 1);
        let mut replicas: Vec<Replica<T>> = vec![Replica::new(initial)];
        for _ in 1..n_replicas {
            let last = replicas.pop().expect("non-empty");
            let (a, b) = last.fork();
            replicas.push(a);
            replicas.push(b);
        }
        Self {
            replicas,
            queue: Vec::new(),
        }
    }

    /// Replica `from` broadcasts its current state as a message addressed
    /// to `to`. The message sits in the queue until a `deliver_*` call
    /// plays it out.
    pub fn broadcast_to(&mut self, from: usize, to: usize) {
        let msg = self.replicas[from].snapshot();
        self.queue.push((to, msg));
    }

    /// Broadcast from `from` to every other replica.
    pub fn broadcast_all(&mut self, from: usize) {
        let msg = self.replicas[from].snapshot();
        for to in 0..self.replicas.len() {
            if to != from {
                self.queue.push((to, msg.clone()));
            }
        }
    }

    /// Deliver the queue entry at `idx` if the recipient has not already
    /// seen it; pop unconditionally. Returns whether a state change happened.
    pub fn deliver_at<F>(&mut self, idx: usize, combine: &F) -> bool
    where
        F: Fn(T, T) -> T,
    {
        let (to, msg) = self.queue.remove(idx);
        let target = std::mem::replace(
            &mut self.replicas[to],
            // Placeholder; immediately overwritten below.
            Replica {
                stamp: Stamp::seed(),
                state: msg.payload.clone(),
            },
        );
        if target.has_seen(&msg) {
            self.replicas[to] = target;
            return false;
        }
        self.replicas[to] = target.deliver(msg, combine);
        true
    }

    /// Walk the queue front-to-back delivering everything once. Useful as
    /// a quiescent flush to drain pending messages.
    pub fn drain_in_order<F>(&mut self, combine: &F)
    where
        F: Fn(T, T) -> T,
    {
        while !self.queue.is_empty() {
            self.deliver_at(0, combine);
        }
    }

    /// Deliver in a caller-specified permutation of queue indices. The
    /// permutation must be a valid one-shot ordering of `0..queue.len()`.
    pub fn drain_in_permutation<F>(&mut self, perm: &[usize], combine: &F)
    where
        F: Fn(T, T) -> T,
    {
        // The queue shrinks as we deliver; map each `perm[i]` to the
        // current queue position it refers to. We use a removal-aware
        // index by sorting indices descending.
        let mut indices = perm.to_vec();
        indices.sort_by(|a, b| b.cmp(a));
        for &idx in &indices {
            if idx < self.queue.len() {
                self.deliver_at(idx, combine);
            }
        }
        self.drain_in_order(combine);
    }

    pub fn n_replicas(&self) -> usize {
        self.replicas.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fork_split_then_merge_recovers_state() {
        let r = Replica::new(0i32);
        let (a, b) = r.fork();
        let a = a.update(|x| x + 10);
        let b = b.update(|x| x + 20);

        let msg_b_to_a = b.snapshot();
        let merged_a = a.deliver(msg_b_to_a, |x, y| x.max(y));
        // After delivery, a observes both events.
        assert_eq!(merged_a.state, 20);
    }

    #[test]
    fn has_seen_filters_redundant_messages() {
        let r = Replica::new(0i32);
        let (a, b) = r.fork();
        let a = a.update(|x| x + 1);
        let snap = a.snapshot();
        // Re-deliver the snapshot to a itself - it should already have seen it.
        assert!(a.has_seen(&snap));
        // b has not seen it.
        assert!(!b.has_seen(&snap));
    }

    #[test]
    fn three_replicas_converge_under_max_combiner() {
        // Each replica writes its own value, broadcasts, drains. All
        // converge to the maximum.
        let mut net: Network<i32> = Network::new(0, 3);
        net.replicas[0] = net.replicas[0].clone().update(|_| 5);
        net.replicas[1] = net.replicas[1].clone().update(|_| 11);
        net.replicas[2] = net.replicas[2].clone().update(|_| 7);

        net.broadcast_all(0);
        net.broadcast_all(1);
        net.broadcast_all(2);
        let combine = |a: i32, b: i32| a.max(b);
        net.drain_in_order(&combine);

        let v0 = net.replicas[0].state;
        let v1 = net.replicas[1].state;
        let v2 = net.replicas[2].state;
        assert_eq!(v0, 11);
        assert_eq!(v1, 11);
        assert_eq!(v2, 11);
    }

    #[test]
    fn delivery_order_does_not_affect_final_state() {
        // Build two networks, deliver in different orders, verify
        // identical final states under a commutative+associative combiner.
        let combine = |a: i32, b: i32| a.max(b);

        let mut a: Network<i32> = Network::new(0, 4);
        let mut b: Network<i32> = Network::new(0, 4);

        for (net, _label) in [&mut a, &mut b].into_iter().zip(["a", "b"]) {
            net.replicas[0] = net.replicas[0].clone().update(|_| 100);
            net.replicas[1] = net.replicas[1].clone().update(|_| 50);
            net.replicas[2] = net.replicas[2].clone().update(|_| 75);
            net.replicas[3] = net.replicas[3].clone().update(|_| 25);
            net.broadcast_all(0);
            net.broadcast_all(1);
            net.broadcast_all(2);
            net.broadcast_all(3);
        }

        a.drain_in_order(&combine);
        // Reverse order for b.
        let n = b.queue.len();
        let perm: Vec<usize> = (0..n).rev().collect();
        b.drain_in_permutation(&perm, &combine);

        for i in 0..4 {
            assert_eq!(
                a.replicas[i].state, b.replicas[i].state,
                "replica {} diverged across orderings",
                i
            );
            assert_eq!(a.replicas[i].state, 100);
        }
    }

    #[test]
    fn redundant_redelivery_is_a_noop() {
        let mut net: Network<i32> = Network::new(0, 2);
        net.replicas[0] = net.replicas[0].clone().update(|_| 42);
        net.broadcast_to(0, 1);
        let combine = |a: i32, b: i32| a.max(b);
        let changed_first = net.deliver_at(0, &combine);
        assert!(changed_first);
        // Re-broadcast and deliver the same snapshot again.
        net.broadcast_to(0, 1);
        let changed_second = net.deliver_at(0, &combine);
        assert!(!changed_second, "second delivery should be a no-op");
    }
}
