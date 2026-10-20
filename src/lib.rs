//! TopHat: provenance tooling for LLM-assisted scientific research projects.
//!
//! The library backs the `tophat` command-line tool. See `rqm/ARCHITECTURE.md` for the overall
//! design and `rqm/` for the requirements of each feature.

pub mod check;
pub mod cli;
pub mod data;
mod fsutil;
mod markdown;
pub mod question;
mod report;
pub mod study;
