//! Command-line surface and command implementations.
//!
//! `main.rs` stays thin: it parses [`Cli`] and calls [`Cli::run`]. Each command
//! does its I/O here, then hands pure data to [`crate::analyze`] and
//! [`crate::report`].

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};

use crate::analyze::{audit, auditable_symbols, outline_size, SymbolUsage};
use crate::changed::{
    analyze as analyze_changes, untracked_change, ChangedSymbols, FileChange, GitSources,
};
use crate::closure::{Reason, Verdict};
use crate::config::Config;
use crate::diff;
use crate::git::{Base, Git};
use crate::impact::{self, Budgets, FsIndex, ImpactReport, TyfRefs};
use crate::model::relative_display;
use crate::pytest;
use crate::report::{Format, Report};
use crate::select::{self, Decision, Selection};
use crate::tyf::Runner;
use crate::workspace::detect_root;

const ABOUT: &str = "Symbol-level Python code intelligence, powered by ty-find.";

const LONG_ABOUT: &str = "\
gerenuk asks `tyf` (from ty-find) about the symbols in your Python project and \
reports what it finds: symbols nothing references, and symbols only your tests \
reach.

It shells out to `tyf --format json`, so `tyf` must be on PATH — install it \
with `uv add --dev ty-find`, or point GERENUK_TYF at the binary.

`changed-symbols` needs none of that: it uses `git` alone, taken from PATH \
unless GERENUK_GIT names a binary.";

const AFTER_LONG_HELP: &str = "\
Getting started:

  1. `gerenuk doctor`             — check that tyf and the workspace resolve.
  2. `gerenuk audit pkg/*.py`     — report unreferenced and test-only symbols.
  3. `gerenuk audit --format json` — same, as JSON for scripting.
  4. `gerenuk changed-symbols`    — which symbols the working tree changed.
  5. `gerenuk impacted-tests`     — which tests those changed symbols reach.
  6. `gerenuk run -- -x`          — run exactly those tests under pytest.

Exit codes:

  0  no findings
  1  findings reported
  2  the run could not complete (tyf missing, bad workspace, ...)

`run` is the exception: once pytest starts, the exit code is pytest's own, because gerenuk's process has become pytest.

`changed-symbols` and `impacted-tests` never return 1: they are inventories \
rather than verdicts. When `impacted-tests` cannot trust its own answer it \
says so in the report (`verdict: run_all`) and still exits 0, because \
\"run everything\" is a usable answer for a pre-commit hook.";

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

/// The three walk budgets, shared by `impacted-tests` and `run`.
#[derive(Args, Debug, Default)]
pub struct BudgetFlags {
    /// BFS levels to walk before giving up and saying `run_all`.
    #[arg(long, value_name = "N")]
    pub max_depth: Option<u32>,

    /// Symbols to visit before giving up and saying `run_all`.
    #[arg(long, value_name = "N")]
    pub max_symbols: Option<usize>,

    /// Wall-clock budget for the walk. `0` disables it.
    #[arg(long, value_name = "MS")]
    pub budget_ms: Option<u64>,
}

impl From<&BudgetFlags> for Budgets {
    fn from(flags: &BudgetFlags) -> Self {
        Self {
            max_depth: flags.max_depth,
            max_symbols: flags.max_symbols,
            budget_ms: flags.budget_ms,
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Report symbols that nothing references, and symbols only tests reach.
    Audit {
        /// Python files to audit. Defaults to nothing — pass the files you care about.
        #[arg(value_name = "FILE", required = true)]
        files: Vec<PathBuf>,
    },

    /// Report the Python symbols the working tree changed against a base ref.
    ///
    /// Needs only `git` — no `tyf`, no `ty`, no Python environment.
    ChangedSymbols {
        /// Base ref to diff against. Default: origin/main, then main, then master.
        #[arg(long, value_name = "REF")]
        base: Option<String>,
    },

    /// Report the tests the working tree's changed symbols can reach.
    ///
    /// Walks the reverse reference graph from `changed-symbols` outwards until
    /// it reaches test code. Needs `git` and `tyf`.
    ImpactedTests {
        /// Base ref to diff against. Default: origin/main, then main, then master.
        #[arg(long, value_name = "REF")]
        base: Option<String>,

        /// Replay a saved `changed-symbols --format json` report instead of
        /// diffing the working tree.
        #[arg(long, value_name = "FILE", conflicts_with = "base")]
        changed: Option<PathBuf>,

        #[command(flatten)]
        budgets: BudgetFlags,
    },

    /// Run pytest on exactly the tests the working tree's changes impact.
    ///
    /// Computes the impact report in-process — never as a subprocess of
    /// itself — maps it to pytest node ids, and then *becomes* pytest. The
    /// exit code from there on is pytest's own.
    ///
    /// Three outcomes, only two of which an argument list can express: run
    /// these tests, run the whole suite, or run nothing at all. The last is why
    /// this is a command and not a list of node ids for a shell to interpolate.
    Run {
        /// Base ref to diff against. Default: origin/main, then main, then master.
        #[arg(long, value_name = "REF")]
        base: Option<String>,

        /// Replay a saved `impacted-tests --format json` report instead of
        /// walking the working tree.
        #[arg(
            long,
            value_name = "FILE",
            conflicts_with_all = ["base", "max_depth", "max_symbols", "budget_ms"],
        )]
        impact: Option<PathBuf>,

        #[command(flatten)]
        budgets: BudgetFlags,

        /// Print the decision and the exact argv; spawn nothing.
        #[arg(long)]
        dry_run: bool,

        /// Everything after `--`, appended to the pytest argv verbatim.
        #[arg(last = true, value_name = "PYTEST_ARGS", allow_hyphen_values = true)]
        pytest_args: Vec<OsString>,
    },

    /// Check that `tyf` and the workspace resolve, without running an analysis.
    Doctor,
}

/// Process exit code. `main` maps this to [`std::process::ExitCode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Clean,
    FindingsReported,
    /// A child process's exit code, propagated verbatim.
    ///
    /// Only reachable where [`pytest::Runner::exec`] cannot replace the
    /// process — on Unix it never returns on success, because there is no
    /// gerenuk left for it to return to.
    Code(u8),
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

        // `tyf` is discovered per command, not up front: `changed-symbols`
        // must work in a checkout that has never had `ty` installed.
        match self.command {
            Command::Doctor => {
                let runner = Runner::discover(&root)?;
                writeln!(out, "workspace: {}", root.display())?;
                writeln!(out, "tyf:       {}", runner.binary().display())?;
                Ok(Outcome::Clean)
            }
            Command::Audit { files } => {
                let runner = Runner::discover(&root)?;
                let report = run_audit(&runner, &root, &files)?;
                write!(out, "{}", report.render(self.format, &root)?)?;
                Ok(if report.is_clean() { Outcome::Clean } else { Outcome::FindingsReported })
            }
            Command::ChangedSymbols { base } => {
                let report = run_changed_symbols(&root, base.as_deref())?;
                write!(out, "{}", report.render(self.format)?)?;
                // Changed symbols are an inventory, not a verdict: a non-empty
                // report is the normal case, so it must not fail the hook.
                Ok(Outcome::Clean)
            }
            Command::ImpactedTests { base, changed, budgets } => {
                let report = run_impacted_tests(
                    &root,
                    base.as_deref(),
                    changed.as_deref(),
                    Budgets::from(&budgets),
                )?;
                write!(out, "{}", report.render(self.format)?)?;
                // Either verdict is an answer. Only a run that could not
                // produce one at all is a failure, and that arrives as an Err.
                Ok(Outcome::Clean)
            }
            Command::Run { base, impact, budgets, dry_run, pytest_args } => run_pytest(
                out,
                &root,
                &RunOptions {
                    base: base.as_deref(),
                    impact: impact.as_deref(),
                    budgets: Budgets::from(&budgets),
                    dry_run,
                    format: self.format,
                    pytest_args: &pytest_args,
                },
            ),
        }
    }
}

/// Diff the working tree against a base ref and map the result to symbols.
///
/// Everything is resolved against the *git* top level rather than `workspace`,
/// because diff paths are repository-relative — and because the pitch puts
/// `[tool.gerenuk]` in the repo-root `pyproject.toml`.
pub fn run_changed_symbols(workspace: &Path, base: Option<&str>) -> Result<ChangedSymbols> {
    let repo = repo_context(workspace)?;
    let base = repo.git.resolve_base(base)?;
    let untracked = repo.git.untracked()?;
    changed_report(&repo.git, &repo.root, &base, &repo.config, &untracked)
}

/// The repository the command runs against, plus its configuration.
struct Repo {
    git: Git,
    /// Absolute path of the git top level. Diff paths are relative to it, and
    /// so is `[tool.gerenuk]` in the repo-root `pyproject.toml`.
    root: PathBuf,
    config: Config,
}

fn repo_context(workspace: &Path) -> Result<Repo> {
    let git = Git::discover(workspace)?;
    let root = git.top_level()?;
    let git = git.rebind(&root);
    let config = Config::load(&root)?;
    Ok(Repo { git, root, config })
}

/// Diff the working tree against `base` and map the result to symbols.
///
/// `untracked` is passed in rather than fetched, because `impacted-tests` needs
/// the same list again for its file index and one `git ls-files --others` per
/// run is enough.
fn changed_report(
    git: &Git,
    root: &Path,
    base: &Base,
    config: &Config,
    untracked: &[PathBuf],
) -> Result<ChangedSymbols> {
    let raw = git.diff(&base.merge_base)?;
    let mut changes: Vec<FileChange> =
        diff::parse(&raw).into_iter().map(FileChange::from).collect();
    changes.extend(untracked.iter().cloned().map(untracked_change));

    let sources = GitSources::new(git, root, &base.merge_base);
    analyze_changes(base, &changes, root, &sources, config)
}

/// Walk from the changed symbols to the tests that reach them.
///
/// Reads like a series of gates, and the order is the point: the verdicts that
/// need nothing but the phase-1 report are settled before `tyf` is looked for,
/// so a diff of `pyproject.toml` alone answers in a checkout with no `ty`
/// installed. Past that gate, anything that goes wrong degrades to `run_all`
/// rather than failing — see `docs/adr/0009-run-all-is-a-success.md`.
pub fn run_impacted_tests(
    workspace: &Path,
    base: Option<&str>,
    changed_file: Option<&Path>,
    budgets: Budgets,
) -> Result<ImpactReport> {
    Ok(impacted_run(&repo_context(workspace)?, base, changed_file, budgets)?.report)
}

/// One impact run, plus the working-tree file list it happened to need.
///
/// `gerenuk run` maps the report onto node ids against the same tree, and one
/// `git ls-files` per invocation is enough.
struct ImpactRun {
    report: ImpactReport,
    /// Every path in the repository, or `None` when the run answered before it
    /// had to ask.
    files: Option<Vec<PathBuf>>,
}

fn impacted_run(
    repo: &Repo,
    base: Option<&str>,
    changed_file: Option<&Path>,
    budgets: Budgets,
) -> Result<ImpactRun> {
    let started = Instant::now();
    let Repo { git, root, config } = repo;
    let root = root.as_path();
    let limits = impact::resolve_limits(budgets, config, started);

    // Kept from the phase-1 diff when there was one: the file index below needs
    // the same list, and asking git twice is a second process for one answer.
    let mut untracked = None;
    let changed = if let Some(path) = changed_file {
        load_report::<ChangedSymbols>(path, "changed", "a changed-symbols report")?
    } else {
        let listed = git.untracked()?;
        let report = changed_report(git, root, &git.resolve_base(base)?, config, &listed)?;
        untracked = Some(listed);
        report
    };

    let unwalked = |reason, errors| ImpactRun {
        report: impact::run_all(&changed, reason, errors),
        files: None,
    };

    if let Some(reason) = impact::upfront_reason(&changed) {
        return Ok(unwalked(reason, Vec::new()));
    }

    let runner = match Runner::discover(root) {
        Ok(runner) => runner,
        Err(err) => return Ok(unwalked(Reason::TyfUnavailable, vec![format!("{err:#}")])),
    };

    // Past the gate every failure is a verdict, not an exit code: a repository
    // that stops answering here still gets `run_all`.
    // See `docs/adr/0009-run-all-is-a-success.md`.
    let files = match workspace_files(git, untracked) {
        Ok(files) => files,
        Err(err) => return Ok(unwalked(Reason::IndexFailed, vec![format!("{err:#}")])),
    };

    let index = FsIndex::new(root, &files);
    let refs = TyfRefs::new(&runner, root);
    let report = impact::analyze(&changed, &refs, &index, config, &limits);
    Ok(ImpactRun { report, files: Some(files) })
}

/// Everything `gerenuk run` was asked for.
struct RunOptions<'a> {
    base: Option<&'a str>,
    impact: Option<&'a Path>,
    budgets: Budgets,
    dry_run: bool,
    format: Format,
    pytest_args: &'a [OsString],
}

/// Map the impact report onto pytest node ids, then become pytest.
///
/// The order is the contract: the selection is built and announced *before*
/// anything is spawned, so a run that decides nothing is impacted exits `0`
/// having started no process at all — and after the exec there is no gerenuk
/// left to report anything anyway.
fn run_pytest(out: &mut impl Write, workspace: &Path, options: &RunOptions) -> Result<Outcome> {
    let started = Instant::now();
    let repo = repo_context(workspace)?;

    let ImpactRun { mut report, files } = match options.impact {
        Some(path) => ImpactRun {
            report: load_report::<ImpactReport>(path, "impact", "an impacted-tests report")?,
            files: None,
        },
        None => impacted_run(&repo, options.base, None, options.budgets)?,
    };

    // A `run_all` verdict needs no tree: the whole suite runs either way, and
    // listing the repository to prove it would be work for nothing. On the
    // replay path there is no listing yet, and a failure to get one degrades
    // rather than fails — the walk path already treats it that way, and the
    // same broken repository must not answer differently.
    // See `docs/adr/0009-run-all-is-a-success.md`.
    let files = match (files, report.verdict) {
        (Some(files), _) => files,
        (None, Verdict::RunAll) => Vec::new(),
        (None, Verdict::Selected) => match workspace_files(&repo.git, None) {
            Ok(files) => files,
            Err(err) => {
                report.verdict = Verdict::RunAll;
                report.reason = Some(Reason::IndexFailed);
                report.errors.push(format!("{err:#}"));
                Vec::new()
            }
        },
    };
    let index = FsIndex::new(&repo.root, &files);
    let selection = select::select(&report, &index);

    let elapsed = started.elapsed().as_millis();
    if options.dry_run {
        return dry_run(out, &repo, &selection, elapsed, options);
    }

    // Said before the exec, because after it there is no gerenuk to say it.
    eprintln!("gerenuk: {}", pytest::summary(&selection, elapsed));
    if selection.decision == Decision::Nothing {
        // Deliberately indistinguishable from a green suite: for a pre-commit
        // hook, that is exactly what it is.
        return Ok(Outcome::Clean);
    }

    let runner = pytest::Runner::resolve(pytest_override(), &repo.config, &repo.root)?;
    let argv = runner.argv(&selection, options.pytest_args);
    // Nothing of ours may still be buffered: the next call replaces us.
    out.flush().context("could not flush gerenuk's own output before running pytest")?;
    runner.exec(&argv).map(Outcome::Code)
}

/// The `GERENUK_PYTEST` override, read here rather than inside
/// [`pytest::Runner::resolve`] so that resolution stays a pure function of its
/// arguments — and so a developer who exports the variable can still run the
/// test suite.
fn pytest_override() -> Option<std::ffi::OsString> {
    std::env::var_os(pytest::PYTEST_BIN_ENV)
}

/// Print the decision and the argv gerenuk would have run.
fn dry_run(
    out: &mut impl Write,
    repo: &Repo,
    selection: &Selection,
    elapsed_ms: u128,
    options: &RunOptions,
) -> Result<Outcome> {
    // A missing pytest must not fail a dry run: the interesting half of the
    // answer is the selection, and reporting it is more use than an error.
    let resolved = pytest::Runner::resolve(pytest_override(), &repo.config, &repo.root);
    let argv = match (selection.decision, resolved) {
        (Decision::Nothing, _) => Vec::new(),
        (_, Ok(runner)) => runner.argv(selection, options.pytest_args),
        (_, Err(err)) => {
            eprintln!("gerenuk: {err:#}");
            Vec::new()
        }
    };

    let report = pytest::DryRun { selection, argv, elapsed_ms };
    match options.format {
        Format::Human => write!(out, "{}", report.render_human())?,
        Format::Json => write!(out, "{}", report.render_json()?)?,
    }
    Ok(Outcome::Clean)
}

/// Every path in the repository, tracked and untracked alike.
fn workspace_files(git: &Git, untracked: Option<Vec<PathBuf>>) -> Result<Vec<PathBuf>> {
    let mut files = git.ls_files()?;
    match untracked {
        Some(listed) => files.extend(listed),
        None => files.extend(git.untracked()?),
    }
    Ok(files)
}

/// Replay a saved `--format json` report from an earlier phase.
///
/// This is what pins each phase's schema as the interface to the next: a report
/// written by one gerenuk has to be readable by another. Strict, for the same
/// reason the schemas are: a report that half-parses would become a confident
/// selection of the wrong tests, and a wrong selection is the one failure `run`
/// must not have.
/// See `docs/adr/0010-a-replayed-report-is-parsed-strictly.md`.
fn load_report<T: serde::de::DeserializeOwned>(path: &Path, flag: &str, what: &str) -> Result<T> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read --{flag} {}", path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("cannot parse {} as {what}", path.display()))
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
            other => panic!("expected an audit command, got {other:?}"),
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
    fn changed_symbols_defaults_to_no_explicit_base() {
        let cli = Cli::try_parse_from(["gerenuk", "changed-symbols"])
            .expect("the subcommand takes no required arguments");
        match cli.command {
            Command::ChangedSymbols { base } => {
                assert_eq!(base, None, "no --base means the default chain is tried");
            }
            other => panic!("expected changed-symbols, got {other:?}"),
        }
    }

    #[test]
    fn changed_symbols_accepts_an_explicit_base() {
        let cli = Cli::try_parse_from([
            "gerenuk",
            "changed-symbols",
            "--base",
            "upstream/release",
            "--format",
            "json",
        ])
        .expect("valid invocation should parse");

        assert_eq!(cli.format, Format::Json, "--format still applies");
        match cli.command {
            Command::ChangedSymbols { base } => {
                assert_eq!(base.as_deref(), Some("upstream/release"), "the ref is passed through");
            }
            other => panic!("expected changed-symbols, got {other:?}"),
        }
    }

    #[test]
    fn impacted_tests_takes_no_required_arguments() {
        let cli = Cli::try_parse_from(["gerenuk", "impacted-tests"])
            .expect("the subcommand should be usable bare");
        match cli.command {
            Command::ImpactedTests { base, changed, budgets } => {
                assert_eq!(base, None, "the default base chain applies");
                assert_eq!(changed, None, "and the diff is computed, not replayed");
                assert_eq!(
                    (budgets.max_depth, budgets.max_symbols, budgets.budget_ms),
                    (None, None, None),
                    "unset budgets fall through to the config and then the defaults"
                );
            }
            other => panic!("expected impacted-tests, got {other:?}"),
        }
    }

    #[test]
    fn impacted_tests_accepts_every_budget_flag() {
        let cli = Cli::try_parse_from([
            "gerenuk",
            "impacted-tests",
            "--max-depth",
            "3",
            "--max-symbols",
            "40",
            "--budget-ms",
            "1500",
        ])
        .expect("valid invocation should parse");

        match cli.command {
            Command::ImpactedTests { budgets, .. } => {
                assert_eq!(
                    (budgets.max_depth, budgets.max_symbols, budgets.budget_ms),
                    (Some(3), Some(40), Some(1500))
                );
            }
            other => panic!("expected impacted-tests, got {other:?}"),
        }
    }

    #[test]
    fn replaying_a_report_and_naming_a_base_are_mutually_exclusive() {
        // A saved report already records the base it was taken against, so
        // accepting both would silently ignore one of them.
        let err = Cli::try_parse_from([
            "gerenuk",
            "impacted-tests",
            "--changed",
            "report.json",
            "--base",
            "main",
        ])
        .expect_err("the two flags contradict each other");
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
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
