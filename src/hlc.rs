//! Hybrid Logical Clock (Kulkarni, Demirbas, Madappa, Avva, Leone - 2014).
//!
//! Combines a physical timestamp `pt` with a logical counter `l` to preserve
//! happens-before across causally related events while staying close to
//! wall-clock time, even under bounded clock skew.
//!
//! The update rules - given local clock `(pt, l)`, a wall-clock reading `now`,
//! and (on receive) a remote clock `(pt', l')`:
//!
//! ```text
//! send/local event:
//!     pt_new = max(pt, now)
//!     l_new  = l + 1   if pt_new == pt
//!            = 0       otherwise
//!
//! receive event:
//!     pt_new = max(pt, pt', now)
//!     l_new  = max(l, l') + 1     if pt_new == pt == pt'
//!            = l + 1               if pt_new == pt
//!            = l' + 1              if pt_new == pt'
//!            = 0                   otherwise
//! ```
//!
//! Properties:
//! - Monotonic: `clock` returns strictly increasing values across calls on a node.
//! - Causal: if `a → b` then `hlc(a) < hlc(b)`.
//! - Bounded drift: `pt` never lags behind wall-clock by more than a bounded
//!   delta determined by message latency.

use std::cmp::Ordering;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Hlc {
    /// Physical-time component (milliseconds since epoch, by convention).
    pub pt: u64,
    /// Logical counter. Resets to zero whenever `pt` strictly advances.
    pub l: u16,
}

impl Hlc {
    pub const fn new(pt: u64) -> Self {
        Self { pt, l: 0 }
    }

    /// Local / send event with wall-clock reading `now`.
    pub fn send(&mut self, now: u64) -> Self {
        let pt_new = self.pt.max(now);
        if pt_new == self.pt {
            self.l = self
                .l
                .checked_add(1)
                .expect("HLC logical counter overflow - clock has stalled for too long");
        } else {
            self.l = 0;
        }
        self.pt = pt_new;
        *self
    }

    /// Receipt of remote HLC `recv` with local wall-clock reading `now`.
    pub fn recv(&mut self, recv: Hlc, now: u64) -> Self {
        let pt_old = self.pt;
        let pt_new = pt_old.max(recv.pt).max(now);

        let l_new = if pt_new == pt_old && pt_new == recv.pt {
            self.l.max(recv.l) + 1
        } else if pt_new == pt_old {
            self.l + 1
        } else if pt_new == recv.pt {
            recv.l + 1
        } else {
            0
        };

        self.pt = pt_new;
        self.l = l_new;
        *self
    }
}

impl PartialOrd for Hlc {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Hlc {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.pt.cmp(&other.pt) {
            Ordering::Equal => self.l.cmp(&other.l),
            o => o,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_under_advancing_wallclock_resets_logical() {
        let mut c = Hlc::new(100);
        c.send(100); // same pt - l ticks
        assert_eq!(c.l, 1);
        c.send(200); // pt advances - l resets
        assert_eq!(c.pt, 200);
        assert_eq!(c.l, 0);
    }

    #[test]
    fn send_under_stalled_wallclock_increments_logical() {
        let mut c = Hlc::new(500);
        c.send(400); // wall-clock behind - pt unchanged, l ticks
        c.send(400);
        assert_eq!(c.pt, 500);
        assert_eq!(c.l, 2);
    }

    #[test]
    fn recv_takes_max_pt_then_breaks_tie_with_logical() {
        let mut local = Hlc { pt: 100, l: 5 };
        let remote = Hlc { pt: 100, l: 9 };
        local.recv(remote, 50);
        assert_eq!(local.pt, 100);
        assert_eq!(local.l, 10);
    }

    #[test]
    fn recv_with_higher_remote_pt_takes_remote_logical_plus_one() {
        let mut local = Hlc { pt: 100, l: 5 };
        let remote = Hlc { pt: 200, l: 3 };
        local.recv(remote, 50);
        assert_eq!(local.pt, 200);
        assert_eq!(local.l, 4);
    }

    #[test]
    fn recv_with_higher_now_resets_logical() {
        let mut local = Hlc { pt: 100, l: 5 };
        let remote = Hlc { pt: 150, l: 3 };
        local.recv(remote, 200);
        assert_eq!(local.pt, 200);
        assert_eq!(local.l, 0);
    }

    #[test]
    fn happens_before_implies_less_than() {
        // Two-node simulation under skewed wall clocks.
        let mut p = Hlc::new(0);
        let mut q = Hlc::new(0);

        let send = p.send(100); // p ahead
        q.recv(send, 50); // q behind; receives from p
        let recv = q.send(50);

        assert!(send < recv);
    }
}
