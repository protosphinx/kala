//! Compact binary wire format for every clock type in this crate.
//!
//! Two operations:
//!
//! - `encode -> Vec<u8>` produces a byte string.
//! - `decode(bytes) -> Option<(Self, &[u8])>` parses one value off the
//!   front and returns the remainder. Returns `None` on truncation or
//!   malformed input.
//!
//! The format is little-endian fixed-width for primitives and
//! tagged-union for the recursive ITC types.
//!
//! v0.6 ships the no-allocation, no-dependency baseline. v0.7 adds
//! length-prefix framing (so multiple stamps can be concatenated and
//! parsed back unambiguously) and a checksum byte.

use crate::hlc::Hlc;
use crate::itc::{Event, Id, Stamp};
use crate::lamport::Lamport;
use crate::vector::VectorClock;

pub trait Wire: Sized {
    fn encode(&self) -> Vec<u8>;
    fn decode(bytes: &[u8]) -> Option<(Self, &[u8])>;
}

fn read_u8(bytes: &[u8]) -> Option<(u8, &[u8])> {
    bytes.split_first().map(|(b, rest)| (*b, rest))
}

fn read_u16(bytes: &[u8]) -> Option<(u16, &[u8])> {
    if bytes.len() < 2 {
        return None;
    }
    let v = u16::from_le_bytes([bytes[0], bytes[1]]);
    Some((v, &bytes[2..]))
}

fn read_u32(bytes: &[u8]) -> Option<(u32, &[u8])> {
    if bytes.len() < 4 {
        return None;
    }
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&bytes[..4]);
    Some((u32::from_le_bytes(buf), &bytes[4..]))
}

fn read_u64(bytes: &[u8]) -> Option<(u64, &[u8])> {
    if bytes.len() < 8 {
        return None;
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&bytes[..8]);
    Some((u64::from_le_bytes(buf), &bytes[8..]))
}

// ---------------------------------------------------------------------------
// Lamport: 8 bytes, little-endian.
// ---------------------------------------------------------------------------

impl Wire for Lamport {
    fn encode(&self) -> Vec<u8> {
        self.raw().to_le_bytes().to_vec()
    }

    fn decode(bytes: &[u8]) -> Option<(Self, &[u8])> {
        let (raw, rest) = read_u64(bytes)?;
        Some((Lamport::from_raw(raw), rest))
    }
}

// ---------------------------------------------------------------------------
// HLC: 10 bytes (u64 pt, u16 l), little-endian.
// ---------------------------------------------------------------------------

impl Wire for Hlc {
    fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(10);
        v.extend_from_slice(&self.pt.to_le_bytes());
        v.extend_from_slice(&self.l.to_le_bytes());
        v
    }

    fn decode(bytes: &[u8]) -> Option<(Self, &[u8])> {
        let (pt, rest) = read_u64(bytes)?;
        let (l, rest) = read_u16(rest)?;
        Some((Hlc { pt, l }, rest))
    }
}

// ---------------------------------------------------------------------------
// VectorClock: u32 length, then u64 entries.
// ---------------------------------------------------------------------------

impl Wire for VectorClock {
    fn encode(&self) -> Vec<u8> {
        let n = self.n_nodes() as u32;
        let mut v = Vec::with_capacity(4 + 8 * n as usize);
        v.extend_from_slice(&n.to_le_bytes());
        for i in 0..self.n_nodes() {
            v.extend_from_slice(&self.get(i).to_le_bytes());
        }
        v
    }

    fn decode(bytes: &[u8]) -> Option<(Self, &[u8])> {
        let (n, mut rest) = read_u32(bytes)?;
        let n = n as usize;
        let mut counts = Vec::with_capacity(n);
        for _ in 0..n {
            let (v, r) = read_u64(rest)?;
            rest = r;
            counts.push(v);
        }
        Some((VectorClock::from_counts(counts), rest))
    }
}

// ---------------------------------------------------------------------------
// Id: tagged union (0 = Zero, 1 = One, 2 = Node l r).
// ---------------------------------------------------------------------------

impl Wire for Id {
    fn encode(&self) -> Vec<u8> {
        let mut v = Vec::new();
        encode_id(self, &mut v);
        v
    }

    fn decode(bytes: &[u8]) -> Option<(Self, &[u8])> {
        decode_id(bytes)
    }
}

fn encode_id(id: &Id, out: &mut Vec<u8>) {
    match id {
        Id::Zero => out.push(0),
        Id::One => out.push(1),
        Id::Node(l, r) => {
            out.push(2);
            encode_id(l, out);
            encode_id(r, out);
        }
    }
}

fn decode_id(bytes: &[u8]) -> Option<(Id, &[u8])> {
    let (tag, rest) = read_u8(bytes)?;
    match tag {
        0 => Some((Id::Zero, rest)),
        1 => Some((Id::One, rest)),
        2 => {
            let (l, rest) = decode_id(rest)?;
            let (r, rest) = decode_id(rest)?;
            Some((Id::node(l, r), rest))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Event: tagged union (0 = Leaf u32, 1 = Node u32 l r).
// ---------------------------------------------------------------------------

impl Wire for Event {
    fn encode(&self) -> Vec<u8> {
        let mut v = Vec::new();
        encode_event(self, &mut v);
        v
    }

    fn decode(bytes: &[u8]) -> Option<(Self, &[u8])> {
        decode_event(bytes)
    }
}

fn encode_event(e: &Event, out: &mut Vec<u8>) {
    match e {
        Event::Leaf(n) => {
            out.push(0);
            out.extend_from_slice(&n.to_le_bytes());
        }
        Event::Node(n, l, r) => {
            out.push(1);
            out.extend_from_slice(&n.to_le_bytes());
            encode_event(l, out);
            encode_event(r, out);
        }
    }
}

fn decode_event(bytes: &[u8]) -> Option<(Event, &[u8])> {
    let (tag, rest) = read_u8(bytes)?;
    match tag {
        0 => {
            let (n, rest) = read_u32(rest)?;
            Some((Event::Leaf(n), rest))
        }
        1 => {
            let (n, rest) = read_u32(rest)?;
            let (l, rest) = decode_event(rest)?;
            let (r, rest) = decode_event(rest)?;
            Some((Event::node(n, l, r), rest))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Stamp: encode(id) || encode(event).
// ---------------------------------------------------------------------------

impl Wire for Stamp {
    fn encode(&self) -> Vec<u8> {
        let mut v = self.id.encode();
        v.extend(self.event.encode());
        v
    }

    fn decode(bytes: &[u8]) -> Option<(Self, &[u8])> {
        let (id, rest) = Id::decode(bytes)?;
        let (event, rest) = Event::decode(rest)?;
        Some((Stamp { id, event }, rest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip<T: Wire + PartialEq + std::fmt::Debug>(v: T) {
        let bytes = v.encode();
        let (decoded, rest) = T::decode(&bytes).expect("must decode");
        assert!(rest.is_empty(), "trailing bytes after decode");
        assert_eq!(decoded, v, "round trip");
    }

    #[test]
    fn lamport_round_trip_zero() {
        round_trip(Lamport::new());
    }

    #[test]
    fn lamport_round_trip_after_ticks() {
        let mut c = Lamport::new();
        for _ in 0..1000 {
            c.tick();
        }
        round_trip(c);
    }

    #[test]
    fn hlc_round_trip_basic() {
        round_trip(Hlc { pt: 1234567, l: 42 });
        round_trip(Hlc {
            pt: u64::MAX,
            l: u16::MAX,
        });
        round_trip(Hlc::new(0));
    }

    #[test]
    fn vector_clock_round_trip() {
        let mut c = VectorClock::new(5);
        c.tick(0);
        c.tick(2);
        c.tick(2);
        c.tick(4);
        round_trip(c);
    }

    #[test]
    fn id_round_trip_zero_one_node() {
        round_trip(Id::Zero);
        round_trip(Id::One);
        round_trip(Id::node(Id::One, Id::Zero));
        round_trip(Id::node(
            Id::node(Id::One, Id::Zero),
            Id::node(Id::Zero, Id::One),
        ));
    }

    #[test]
    fn event_round_trip_leaf_and_node() {
        round_trip(Event::Leaf(42));
        round_trip(Event::node(3, Event::Leaf(0), Event::Leaf(7)));
        round_trip(Event::node(
            1,
            Event::node(2, Event::Leaf(0), Event::Leaf(0)),
            Event::Leaf(5),
        ));
    }

    #[test]
    fn stamp_round_trip_seed() {
        round_trip(Stamp::seed());
    }

    #[test]
    fn stamp_round_trip_after_fork_and_event() {
        let s = Stamp::seed();
        let (mut alice, _bob) = s.fork();
        for _ in 0..10 {
            alice = alice.event();
        }
        round_trip(alice);
    }

    #[test]
    fn truncated_input_returns_none() {
        // HLC needs 10 bytes; give it 9.
        let mut bytes = Hlc { pt: 1, l: 1 }.encode();
        bytes.pop();
        assert!(Hlc::decode(&bytes).is_none());
    }

    #[test]
    fn invalid_tag_returns_none() {
        assert!(Id::decode(&[42]).is_none());
        assert!(Event::decode(&[42, 0, 0, 0, 0]).is_none());
    }

    #[test]
    fn concatenated_decodes_leave_remainder() {
        let a = Lamport::new();
        let b = {
            let mut c = Lamport::new();
            c.tick();
            c
        };
        let mut bytes = a.encode();
        bytes.extend(b.encode());
        let (a_decoded, rest) = Lamport::decode(&bytes).unwrap();
        assert_eq!(a_decoded, a);
        let (b_decoded, rest) = Lamport::decode(rest).unwrap();
        assert_eq!(b_decoded, b);
        assert!(rest.is_empty());
    }
}
