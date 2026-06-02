//! `llmprobe` core library.
//!
//! A thin binary (`main.rs`) sits over this testable core. The measurement
//! modules (`client`, `runner`, `metrics`, `types`) know nothing about
//! terminals or argv; `report` and `tui` are presentation only.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

pub mod cli;
pub mod client;
pub mod config;
pub mod error;
pub mod metrics;
pub mod report;
pub mod runner;
pub mod types;

#[cfg(feature = "tui")]
pub mod tui;
