//! kala — काल — time itself, as a Rust crate.
//!
//! Distributed-systems logical-time primitives. Three time models, one trait:
//!
//! - [`Lamport`] — scalar logical clock (Lamport, 1978). Total order via
//!   tie-break, captures happens-before across messaging.
//! - [`Hlc`] — Hybrid Logical Clock (Kulkarni et al., 2014). Wall-clock-aware
//!   while preserving causality even under bounded skew.
//! - `Itc` — Interval Tree Clock (Almeida et al., 2008). v0.1.
//!
//! Every clock here exposes `send` (local event), `recv` (incoming event),
//! and `Ord` such that `a → b ⇒ a < b` (happens-before implies less-than).

pub mod hlc;
pub mod lamport;

pub use hlc::Hlc;
pub use lamport::Lamport;
