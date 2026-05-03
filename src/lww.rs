//! Last-writer-wins register over an ITC [`Stamp`](crate::Stamp).
//!
//! A worked example of building a CRDT on top of the time primitives in
//! this crate. The register holds a value and an ITC stamp. Replicas can
//! be forked freely; writes record events; merge resolves divergent writes
//! by stamp comparison.
//!
//! Three cases for `merge(a, b)`:
//!
//! - `a < b` (a happens-before b): b wins.
//! - `a > b` (b happens-before a): a wins.
//! - `a == b` or concurrent: a user-supplied tiebreak resolves the value.
//!
//! The result's stamp is always `a.stamp.join(b.stamp)`, so subsequent
//! writes from the merged replica are causally after both inputs.

use crate::itc::Stamp;
use std::cmp::Ordering;

#[derive(Clone, Debug)]
pub struct LwwRegister<T: Clone> {
    pub stamp: Stamp,
    pub value: T,
}

impl<T: Clone> LwwRegister<T> {
    /// Create a fresh register with `value`. Holds the universe-owning seed
    /// stamp; fork it before handing replicas to other operators.
    pub fn new(value: T) -> Self {
        Self {
            stamp: Stamp::seed(),
            value,
        }
    }

    /// Fork the register into two replicas. Both observe the same value;
    /// the stamp is split so subsequent writes do not collide.
    pub fn fork(self) -> (Self, Self) {
        let value = self.value;
        let (s1, s2) = self.stamp.fork();
        (
            Self {
                stamp: s1,
                value: value.clone(),
            },
            Self { stamp: s2, value },
        )
    }

    /// Record a write of `new_value`, advancing the stamp by one event.
    pub fn write(self, new_value: T) -> Self {
        Self {
            stamp: self.stamp.event(),
            value: new_value,
        }
    }

    /// Merge two replicas. If one stamp strictly precedes the other, the
    /// later writer wins. Otherwise (concurrent or equal), `tiebreak`
    /// chooses.
    pub fn merge<F>(self, other: Self, tiebreak: F) -> Self
    where
        F: FnOnce(T, T) -> T,
    {
        let cmp = self.stamp.partial_cmp(&other.stamp);
        let merged_stamp = self.stamp.clone().join(other.stamp.clone());
        match cmp {
            Some(Ordering::Less) => Self {
                stamp: merged_stamp,
                value: other.value,
            },
            Some(Ordering::Greater) => Self {
                stamp: merged_stamp,
                value: self.value,
            },
            Some(Ordering::Equal) => Self {
                stamp: merged_stamp,
                value: self.value,
            },
            None => Self {
                stamp: merged_stamp,
                value: tiebreak(self.value, other.value),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_chain_later_writer_wins() {
        let r = LwwRegister::new("initial");
        let r = r.write("a");
        let r = r.write("b");
        // b is the latest write.
        assert_eq!(r.value, "b");
    }

    #[test]
    fn merge_after_one_writer_picks_later_value() {
        let r = LwwRegister::new("seed");
        let (left, right) = r.fork();
        let left = left.write("from-left");
        // right never wrote, so left's stamp is strictly later than right's.
        let merged = left.merge(right, |_, b| b);
        assert_eq!(merged.value, "from-left");
    }

    #[test]
    fn concurrent_writes_resolved_by_tiebreak() {
        let r = LwwRegister::new("seed");
        let (left, right) = r.fork();
        let left = left.write("left-1");
        let right = right.write("right-1");
        // Stamps incomparable; tiebreak picks deterministically.
        let merged = left.merge(right, |a, b| if a < b { a } else { b });
        // alphabetical min: "left-1" < "right-1" so result is "left-1".
        assert_eq!(merged.value, "left-1");
    }

    #[test]
    fn merge_is_associative() {
        // For three concurrent writes, merging in any order gives the same value
        // (when the tiebreak itself is associative).
        let r = LwwRegister::new(0i32);
        let (r1, rest) = r.fork();
        let (r2, r3) = rest.fork();
        let r1 = r1.write(10);
        let r2 = r2.write(20);
        let r3 = r3.write(30);

        let max_tie = |a: i32, b: i32| a.max(b);

        let by_left = r1
            .clone()
            .merge(r2.clone(), max_tie)
            .merge(r3.clone(), max_tie);
        let by_right = r1.merge(r2.merge(r3, max_tie), max_tie);
        assert_eq!(by_left.value, by_right.value);
        assert_eq!(by_left.value, 30);
    }

    #[test]
    fn write_after_merge_dominates_both_inputs() {
        let r = LwwRegister::new(0i32);
        let (left, right) = r.fork();
        let left = left.write(10);
        let right = right.write(20);

        // Merge picks 20 via max tiebreak.
        let merged = left.clone().merge(right.clone(), |a, b| a.max(b));
        // Now write a fresh value through the merged replica.
        let final_state = merged.write(99);
        // The fresh stamp strictly dominates both originals.
        assert!(left.stamp < final_state.stamp);
        assert!(right.stamp < final_state.stamp);
        assert_eq!(final_state.value, 99);
    }
}
