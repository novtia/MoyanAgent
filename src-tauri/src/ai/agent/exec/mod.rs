//! Execution: drive the model ↔ tool loop.
//!
//! - [`query`]    `QueryEngine` trait + request/result types
//! - [`engine`]   `ProviderEngine` (single-shot) + `ProviderQueryEngine`
//!                (multi-turn tool loop) + `run_chat_request` host entry
//! - [`runner`]   `run_agent` — sets up a child run context and drives the
//!                engine for a (sub-)agent
//!
//! This layer is the *only* place that knows about LLM providers.
//! Everything below it (core / config / memory / tools) is provider-agnostic.

pub mod engine;
pub mod query;
pub mod runner;
