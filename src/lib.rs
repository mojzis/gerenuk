//! gerenuk — symbol-level Python code intelligence built on top of `ty-find`.
//!
//! Three commands over two external tools:
//!
//! * `audit` asks [`tyf`] which symbols nothing references.
//! * `changed-symbols` asks [`git`] what the working tree moved, and
//!   [`pysource`] which symbol owns each moved line.
//! * `impacted-tests` walks [`closure`] outwards from those symbols to the
//!   tests that reach them.
//!
//! Supporting them: [`model`] (wire types for `tyf`'s JSON), [`workspace`]
//! (project root, test-path heuristic), [`analyze`] (the audit rules),
//! [`diff`], [`modpath`], [`config`], [`changed`], [`impact`] and [`report`].
//!
//! [`tyf::Runner::run`] and [`git::Git`]'s private `output` are the only two
//! places in the crate that spawn a process — every `git` call in it goes
//! through [`git::Git::run`] or [`git::Git::try_run`], and everything
//! downstream of both seams takes already-parsed data.
//! That is what makes the rules and the renderers unit-testable with no `tyf`,
//! no `ty`, no `git` and no Python. See `docs/dev/ARCHITECTURE.md`, and
//! `docs/adr/` for why the shape is what it is.

pub mod analyze;
pub mod changed;
pub mod cli;
pub mod closure;
pub mod config;
pub mod diff;
pub mod git;
pub mod impact;
pub mod model;
pub mod modpath;
pub mod pysource;
pub mod report;
pub mod tyf;
pub mod workspace;
