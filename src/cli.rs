//! Command-line surface and command implementations.
//!
//! `main.rs` stays thin: it parses [`Cli`] and calls [`Cli::run`]. Each command
//! does its I/O here, then hands pure data to [`crate::analyze`] and
//! [`crate::report`].

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::analyze::{audit, auditable_symbols, outline_size, SymbolUsage};
use crate::model::relative_display;
use crate::report::{Format, Report};
use crate::tyf::Runner;
use crate::workspace::detect_root;

const ABOUT: &str = "Symbol-level Python code intelligence, powered by ty-find.";

const LONG_ABOUT: &str = "\
gerenuk asks `tyf` (from ty-find) about the symbols in your Python project and \
reports what it finds: symbols nothing references, and symbols only your tests \
reach.

It shells out to `tyf --format json`, so `tyf` must be on PATH — install it \
with `uv add --dev ty-find`, or point GERENUK_TYF at the binary.";

const AFTER_LONG_HELP: &str = "\
Getting started:

  1. `gerenuk doctor`             — check that tyf and the workspace resolve.
  2. `gerenuk audit pkg/*.py`     — report unreferenced and test-only symbols.
  3. `gerenuk audit --format json` — same, as JSON for scripting.

Exit codes:

  0  no findings
  1  findings reported
  2  the run could not complete (tyf missing, bad workspace, ...)";

#[derive(Parser, Debug)]
#[command(
    name = "gerenuk",
    version,
    about = ABOUT,
    long_about = LONG_ABOUT,
    after_long_help = AFTER_LONG_HELP,
)]
pub struct Cli {
    /// Project root (default: auto-detect upward from the current directory).
    #[arg(long, global = true, value_name = "PATH")]
    pub workspace: Option<PathBuf>,

    /// Output shape.
    #[arg(long, global = true, value_enum, default_value_t = Format::Human)]
    pub format: Format,

    /// Log at debug level to stderr.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Report symbols that nothing references, and symbols only tests reach.
    Audit {
        /// Python files to audit. Defaults to nothing — pass the files you care about.
        #[arg(value_name = "FILE", required = true)]
        files: Vec<PathBuf>,
    },

    /// Check that `tyf` and the workspace resolve, without running an analysis.
    Doctor,
}

/// Process exit code. `main` maps this to [`std::process::ExitCode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Clean,
    FindingsReported,
}

impl Cli {
    /// Execute the parsed command, writing output to `out`.
    pub fn run(self, out: &mut impl Write) -> Result<Outcome> {
        let root = match self.workspace {
            Some(explicit) => explicit
                .canonicalize()
                .with_context(|| format!("cannot resolve --workspace {}", explicit.display()))?,
            None => {
                detect_root(&std::env::current_dir().context("cannot read current directory")?)?
            }
        };

        let runner = Runner::discover(&root)?;

        match self.command {
            Command::Doctor => {
                writeln!(out, "workspace: {}", root.display())?;
                writeln!(out, "tyf:       {}", runner.binary().display())?;
                Ok(Outcome::Clean)
            }
            Command::Audit { files } => {
                let report = run_audit(&runner, &root, &files)?;
                write!(out, "{}", report.render(self.format, &root)?)?;
                Ok(if report.is_clean() { Outcome::Clean } else { Outcome::FindingsReported })
            }
        }
    }
}

/// Outline each file, ask `tyf refs` about every auditable symbol, then apply
/// the rules in [`crate::analyze`].
pub fn run_audit(runner: &Runner, root: &Path, files: &[PathBuf]) -> Result<Report> {
    let mut findings = Vec::new();
    let mut audited = Vec::new();
    let mut symbols_checked = 0;

    for file in files {
        let outline =
            runner.list(file).with_context(|| format!("could not outline {}", file.display()))?;
        symbols_checked += outline_size(&outline);

        let mut usages = Vec::new();
        for (name, kind, line) in auditable_symbols(&outline) {
            let refs = runner
                .refs(&name)
                .with_context(|| format!("could not resolve references for `{name}`"))?;
            usages.push(SymbolUsage { name, kind, line, refs });
        }

        findings.extend(audit(file, root, &usages));
        audited.push(relative_display(file, root));
    }

    Ok(Report::new(audited, symbols_checked, findings))
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn cli_definition_is_internally_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn audit_parses_files_and_global_flags() {
        let cli = Cli::try_parse_from([
            "gerenuk",
            "--workspace",
            "/proj",
            "--format",
            "json",
            "audit",
            "a.py",
            "b.py",
        ])
        .expect("valid invocation should parse");

        assert_eq!(cli.workspace.as_deref(), Some(Path::new("/proj")));
        assert_eq!(cli.format, Format::Json, "--format json should select JSON output");
        match cli.command {
            Command::Audit { files } => {
                assert_eq!(
                    files,
                    vec![PathBuf::from("a.py"), PathBuf::from("b.py")],
                    "both files parse"
                );
            }
            other @ Command::Doctor => panic!("expected an audit command, got {other:?}"),
        }
    }

    #[test]
    fn global_flags_are_accepted_after_the_subcommand() {
        let cli = Cli::try_parse_from(["gerenuk", "audit", "a.py", "--format", "json"])
            .expect("global flags should work in trailing position");
        assert_eq!(cli.format, Format::Json, "trailing --format must still apply");
    }

    #[test]
    fn human_is_the_default_format() {
        let cli = Cli::try_parse_from(["gerenuk", "doctor"]).expect("doctor takes no arguments");
        assert_eq!(cli.format, Format::Human, "human output is the default");
        assert!(!cli.verbose, "verbose is off unless asked for");
    }

    #[test]
    fn audit_without_files_is_rejected() {
        let err = Cli::try_parse_from(["gerenuk", "audit"])
            .expect_err("audit requires at least one file");
        assert_eq!(
            err.kind(),
            clap::error::ErrorKind::MissingRequiredArgument,
            "clap should report the missing FILE argument"
        );
    }

    #[test]
    fn an_unknown_format_is_rejected() {
        let err = Cli::try_parse_from(["gerenuk", "--format", "yaml", "doctor"])
            .expect_err("yaml is not a format");
        assert_eq!(
            err.kind(),
            clap::error::ErrorKind::InvalidValue,
            "clap should reject unsupported --format values"
        );
    }
}
