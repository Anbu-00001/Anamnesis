//! Anamnesis — an instrument against self-deception.
//!
//! Plato's *anamnesis* is "un-forgetting": the recollection of what the soul
//! actually knew before it was born into forgetting. This crate is a small,
//! local-first ledger for the everyday version of that problem. You record what
//! you believe, *how sure* you are, and *why* — timestamped before the outcome
//! is known. Later, when reality has spoken, the engine here confronts you with
//! the true shape of your judgement: where you are overconfident, whether you
//! can tell truth from falsehood at all, and how honestly you change your mind.
//!
//! The scoring engine ([`scoring`]) is pure mathematics over `std` — no I/O, no
//! network, no model. The rest is storage ([`store`]), the domain model
//! ([`model`]), and human-readable reporting ([`report`]).

pub mod mcp;
pub mod model;
pub mod report;
pub mod scoring;
pub mod store;

pub use model::{gen_id, Claim, ClaimKind, Forecast, Ledger, NumericForecast, Outcome, Resolution};
pub use scoring::{NumericSample, Sample};
