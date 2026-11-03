// SPDX-License-Identifier: AGPL-3.0-only
pub mod chart;
pub mod config;
mod etag;
pub mod judgement;
pub mod layout;
mod queue;
pub mod render;
pub mod service;
mod storage;
mod worker_limits;
pub type Result<T> = std::result::Result<T, String>;
pub const MAX_INPUT_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_EVENTS: usize = 12_000;
pub const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
