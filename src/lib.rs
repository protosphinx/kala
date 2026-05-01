//! kala - काल - time itself, as a Rust crate.
//!
//! Distributed-systems logical-time primitives. Three time models, one trait:
//!
//! - [`Lamport`] - scalar logical clock (Lamport, 1978). Total order via
//!   tie-break, captures happens-before across messaging.
//! - [`Hlc`] - Hybrid Logical Clock (Kulkarni et al., 2014). Wall-clock-aware
//!   while preserving causality even under bounded skew.
//! - [`VectorClock`] - per-node counters (Fidge, Mattern, 1988). The first
//!   clock in this zoo whose `PartialOrd` actually returns `None` for
//!   *concurrent* stamps - happens-before becomes a real partial order, not a
//!   total one with tie-breaks.
//! - [`Stamp`] - Interval Tree Clock (Almeida, Baquero, Fonte, 2008). The
//!   answer to "vector clocks, but for dynamic membership". Stamps fork,
//!   record events, and join, all without a fixed node list.
//!
//! Each clock exposes a `tick`/`send` (local event) and a `merge`/`recv`
//! (incoming event) and orders consistently with happens-before.

pub mod hlc;
pub mod itc;
pub mod lamport;
pub mod lww;
pub mod vector;

pub use hlc::Hlc;
pub use itc::{Event, Id, Stamp};
pub use lamport::Lamport;
pub use lww::LwwRegister;
pub use vector::VectorClock;
