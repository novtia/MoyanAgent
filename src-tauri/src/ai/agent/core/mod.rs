//! Core primitives shared by every agent layer.
//!
//! - [`permission`]   permission mode + resolver
//! - [`context`]      `ToolUseContext` — the per-agent isolation boundary
//! - [`task`]         `Task` / `TaskStore` lifecycle container
//! - [`attachment`]   hidden user-meta messages + `NotificationQueue`
//!
//! These modules have **no dependencies on memory, tools, or exec** —
//! they're the bedrock that every other layer composes against.

pub mod attachment;
pub mod context;
pub mod permission;
pub mod task;
