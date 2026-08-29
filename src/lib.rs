//! gerenuk — symbol-level Python code intelligence built on top of `ty-find`.
//!
//! The crate is a thin, testable layer over the `tyf` binary:
//!
//! * [`tyf`] spawns `tyf --format json <cmd>` and captures stdout.
//! * [`model`] holds the wire types that `tyf` emits (LSP-shaped locations,
//!   reference lists, document outlines).
//! * [`workspace`] locates the Python project root that a command runs against.
//! * [`analyze`] turns those raw answers into gerenuk's own findings.
//! * [`report`] renders findings as human text or JSON.
//!
//! Everything except [`tyf::Runner::run`] is pure, so the analysis and rendering
//! layers are unit-testable without `tyf` (or `ty`) installed.

pub mod analyze;
pub mod changed;
pub mod cli;
pub mod config;
pub mod diff;
pub mod git;
pub mod model;
pub mod modpath;
pub mod pysource;
pub mod report;
pub mod tyf;
pub mod workspace;
