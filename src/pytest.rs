//! Running pytest — gerenuk's third and last impure seam.
//!
//! [`crate::tyf::Runner::run`] and [`crate::git::Git`]'s private `output` were
//! the only two places in the crate that spawned a process; this is the third,
//! and it is deliberately shaped so that it cannot grow into anything. See
//! `docs/adr/0011-a-third-seam-that-only-execs.md`.
//!
//! **Exec-and-replace, not run-and-parse.** On Unix the final act is
//! [`std::os::unix::process::CommandExt::exec`]: gerenuk's process *becomes*
//! pytest. There is no output capture, no TTY mediation, no signal forwarding
//! and no progress reinterpretation, because after the exec there is no gerenuk
//! left to do any of it — Ctrl-C, colours and the exit code are pytest's own.
//! (Windows has no `exec`, so it spawns, waits and propagates the code; the
//! observable behaviour is the same, and the `#[cfg]` is the honest way to say
//! so.)
//!
//! The same seam execs the configured fallback command when the outcome is
//! `run_all` (see [`crate::fallback`]). It is the same shape — exec, never
//! spawn-and-wait — with one addition: a [`Handoff`] of bytes for the child's
//! stdin and variables for its environment. The bytes go into an unlinked
//! temporary file that becomes the child's fd 0, not a pipe, so a child that
//! never reads them cannot block and nothing of gerenuk's lingers to write
//! them. See `docs/adr/0014-run-all-delegates-to-a-fallback.md`.
//!
//! **pytest does not inherit the hook's git.** A pre-commit hook runs with
//! git's repository-local variables exported — `GIT_DIR`, `GIT_INDEX_FILE` and
//! the rest of [`LOCAL_GIT_ENV`] — so that every git the hook spawns targets
//! the repository being committed. A test that creates a repository of its own
//! and inherits them operates on the outer one instead, so by default
//! ([`GitEnv::Isolate`]) the [`Handoff`] for pytest removes them. gerenuk's
//! own diff was taken before, in the hook's context, and the fallback inherits
//! everything: it is the repository's own script and may need that index. See
//! `docs/adr/0020-pytest-does-not-inherit-the-hooks-git.md`.
//!
//! Everything else here — resolving the runner, assembling the argv, deciding
//! which variables go, writing the one-line summary — is pure and unit-tested
//! with no spawn anywhere near it.

use std::ffi::OsString;
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::process::Stdio;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::fallback::Plan;
use crate::select::{Decision, DropReason, Selection};

/// Binary name looked up on `PATH` when nothing else names one.
pub const DEFAULT_PYTEST_BIN: &str = "pytest";

/// Environment variable that overrides which pytest is run.
pub const PYTEST_BIN_ENV: &str = "GERENUK_PYTEST";

/// The variables git exports to a hook so that every git the hook spawns
/// targets the repository being operated on — `git rev-parse
/// --local-env-vars`, in git's own order.
///
/// A copy rather than a query, so the rule stays pure; `tests/run.rs` checks
/// it against the installed git's answer, which is where drift would show.
pub const LOCAL_GIT_ENV: &[&str] = &[
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
];

/// Whether pytest inherits git's repository-local environment.
///
/// `[tool.gerenuk] git-env` and `run --git-env`. The fallback command is not
/// governed by it: it inherits everything, always.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum GitEnv {
    /// Remove git's repository-local variables (`GIT_DIR`, `GIT_INDEX_FILE`, ...)
    /// from pytest's environment
    #[default]
    Isolate,
    /// Hand pytest gerenuk's environment as it is
    Inherit,
}

impl GitEnv {
    /// The variables pytest will not inherit: under `Isolate`, every one of
    /// [`LOCAL_GIT_ENV`] that `is_set` says is present; under `Inherit`, none.
    ///
    /// Only the set ones, so the dry run can say exactly what a hook exported.
    /// `is_set` arrives as an argument rather than being `std::env::var_os`
    /// here, so this stays a pure function of what it is given.
    #[must_use]
    pub fn removals(self, is_set: impl Fn(&str) -> bool) -> Vec<&'static str> {
        match self {
            Self::Isolate => LOCAL_GIT_ENV.iter().copied().filter(|name| is_set(name)).collect(),
            Self::Inherit => Vec::new(),
        }
    }

    /// The value as `pyproject.toml` and `--git-env` spell it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Isolate => "isolate",
            Self::Inherit => "inherit",
        }
    }
}

/// What the child receives besides its argv.
///
/// pytest gets no stdin and no added variables, and by default loses the
/// hook's git ones; the fallback gets its payload and its reason, and loses
/// nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Handoff {
    /// Bytes the child finds on its stdin. Delivered from an unlinked
    /// temporary file rather than a pipe: there is no writer left after the
    /// exec, and a child that ignores its stdin must not hang or fail.
    pub stdin: Option<Vec<u8>>,
    /// Variables added to the environment the child inherits. Applied after
    /// `remove`, so an addition always wins.
    pub env: Vec<(OsString, OsString)>,
    /// Variables removed from the environment the child inherits.
    pub remove: Vec<OsString>,
}

impl Handoff {
    /// pytest's handoff: nothing added, `removed` taken away.
    #[must_use]
    pub fn for_pytest(removed: &[&str]) -> Self {
        Self { remove: removed.iter().map(OsString::from).collect(), ..Self::default() }
    }
}

/// What `--dry-run` reports about the child's git environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GitEnvPlan {
    pub policy: GitEnv,
    /// The variables that are set right now and would be removed.
    pub removed: Vec<&'static str>,
}

impl GitEnvPlan {
    #[must_use]
    pub fn new(policy: GitEnv, is_set: impl Fn(&str) -> bool) -> Self {
        Self { policy, removed: policy.removals(is_set) }
    }

    /// The fallback's: it inherits everything, whatever the policy says.
    #[must_use]
    pub const fn inherited() -> Self {
        Self { policy: GitEnv::Inherit, removed: Vec::new() }
    }

    /// The line the human dry run prints for it.
    #[must_use]
    pub fn render_human(&self) -> String {
        match (self.policy, self.removed.is_empty()) {
            (GitEnv::Inherit, _) => "git env: inherit".to_string(),
            (GitEnv::Isolate, true) => {
                "git env: isolate — no repository-local git variable is set".to_string()
            }
            (GitEnv::Isolate, false) => {
                format!("git env: isolate — removes {}", self.removed.join(", "))
            }
        }
    }
}

/// The pytest invocation gerenuk will exec, minus the node ids.
///
/// An argv rather than a path, because the common real-world value is a
/// multi-word runner: `uv run pytest`, `poetry run pytest`, `hatch test`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runner {
    command: Vec<OsString>,
    root: PathBuf,
}

impl Runner {
    /// Resolve the runner: `override_bin`, then `pytest-command` from
    /// `pyproject.toml`, then `pytest` on `PATH`.
    ///
    /// The override arrives as an argument rather than being read from
    /// `GERENUK_PYTEST` here: reading the environment mid-resolution would make
    /// this function's own tests depend on the machine running them, and a
    /// developer who exports the variable this feature documents could not run
    /// `cargo test`. [`crate::cli`] does the lookup, the way every other seam
    /// keeps its impurity at the edge.
    pub fn resolve(
        override_bin: Option<OsString>,
        config: &Config,
        root: impl Into<PathBuf>,
    ) -> Result<Self> {
        let root = root.into();

        if let Some(explicit) = override_bin {
            return Ok(Self { command: vec![explicit], root });
        }
        // An empty list in the config is a mistake, not a request to run
        // nothing; falling through is kinder than exec'ing the empty string.
        if !config.pytest_command.is_empty() {
            let command = config.pytest_command.iter().map(OsString::from).collect();
            return Ok(Self { command, root });
        }

        let bin = which::which(DEFAULT_PYTEST_BIN).with_context(|| {
            format!(
                "`{DEFAULT_PYTEST_BIN}` not found on PATH. Set `pytest-command` under \
                 [tool.gerenuk] in pyproject.toml, or point {PYTEST_BIN_ENV} at the binary."
            )
        })?;
        Ok(Self { command: vec![bin.into_os_string()], root })
    }

    /// Build a runner from an explicit argv, skipping every lookup.
    pub fn with_command<I, S>(command: I, root: impl Into<PathBuf>) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        Self { command: command.into_iter().map(Into::into).collect(), root: root.into() }
    }

    /// The full argv for a selection: the runner, the node ids, the passthrough.
    ///
    /// A `run_all` selection carries no node ids, so it comes out as the bare
    /// runner — which is exactly "the whole suite". The empty selection never
    /// reaches here: an empty argv would mean the same thing, and it must not.
    #[must_use]
    pub fn argv(&self, selection: &Selection, passthrough: &[OsString]) -> Vec<OsString> {
        debug_assert!(
            selection.decision != Decision::Nothing,
            "an empty selection must short-circuit before an argv is built"
        );
        let mut argv = self.command.clone();
        argv.extend(selection.ids().map(OsString::from));
        argv.extend(passthrough.iter().cloned());
        argv
    }

    /// Replace this process with `argv` — pytest, or the fallback command.
    ///
    /// On Unix this only ever returns an error: on success there is no caller
    /// left to return to.
    pub fn exec(&self, argv: &[OsString], handoff: Handoff) -> Result<u8> {
        let (program, rest) =
            argv.split_first().context("the command resolved to an empty argv")?;

        let mut command = std::process::Command::new(program);
        command.args(rest).current_dir(&self.root);
        for name in &handoff.remove {
            command.env_remove(name);
        }
        command.envs(handoff.env);

        if let Some(bytes) = handoff.stdin {
            // A file, not a pipe: the payload is complete before the child
            // exists, the child reads it or not at its leisure, and it is
            // already unlinked, so nothing is left behind either way.
            let mut file = tempfile::tempfile().context("could not create a file for stdin")?;
            file.write_all(&bytes).context("could not write the payload for stdin")?;
            file.seek(SeekFrom::Start(0)).context("could not rewind the payload for stdin")?;
            command.stdin(Stdio::from(file));
        }

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Returns only on failure; on success this process is now pytest.
            let err = command.exec();
            Err(anyhow::Error::new(err)
                .context(format!("failed to exec `{}`", render_argv(argv).join(" "))))
        }
        #[cfg(not(unix))]
        {
            let status = command
                .status()
                .with_context(|| format!("failed to run `{}`", render_argv(argv).join(" ")))?;
            // Two cases land here with no verdict of pytest's to report: a
            // signal-terminated child, which has no code at all, and a code
            // outside a byte — Windows exit codes are a full `i32`, so a
            // crashed pytest arrives as something like `0xC0000005`. Both are
            // operational failures rather than a pytest result, which is
            // exactly what `2` means; it does alias gerenuk's own failure code,
            // and ADR 0011 records that.
            Ok(status.code().and_then(|code| u8::try_from(code).ok()).unwrap_or(2))
        }
    }
}

/// The single line gerenuk says for itself, before pytest takes the terminal.
#[must_use]
pub fn summary(selection: &Selection, elapsed_ms: u128) -> String {
    match selection.decision {
        Decision::Selected => format!(
            "{} node id(s) from {} origin(s) in {elapsed_ms} ms — details: gerenuk impacted-tests",
            selection.node_ids.len(),
            selection.origins(),
        ),
        Decision::RunAll => format!(
            "full suite — {}",
            selection.reason.map_or("no reason given", crate::closure::Reason::label)
        ),
        Decision::Nothing => "no tests impacted — nothing to run".to_string(),
    }
}

/// What `--dry-run` prints instead of spawning.
///
/// The human form states the decision and lists the argv one element per line:
/// legible, and deliberately not something a shell can interpolate back into a
/// `pytest $(...)`. The JSON form is the phase-3 schema.
pub struct DryRun<'a> {
    pub selection: &'a Selection,
    pub argv: Vec<OsString>,
    pub elapsed_ms: u128,
    /// The fallback that would have been exec'd: set only when the decision is
    /// `run_all` and one is configured. Then `argv` is its argv.
    pub fallback: Option<Plan<'a>>,
    /// What the child would and would not inherit of git's environment.
    pub git_env: GitEnvPlan,
}

impl DryRun<'_> {
    #[must_use]
    pub fn render_human(&self) -> String {
        use std::fmt::Write as _;

        let mut out = String::new();
        let _ = writeln!(out, "decision: {}", decision_label(self.selection.decision));
        let _ = writeln!(out, "gerenuk: {}", summary(self.selection, self.elapsed_ms));
        if self.selection.decision != Decision::Nothing {
            let _ = writeln!(out, "{}", self.git_env.render_human());
        }

        if !self.selection.node_ids.is_empty() {
            let _ = writeln!(out);
            for selected in &self.selection.node_ids {
                let _ = writeln!(out, "{}", selected.node_id);
                let chain: Vec<&str> = selected
                    .via
                    .iter()
                    .map(String::as_str)
                    .chain(selected.origin.as_deref())
                    .collect();
                if !chain.is_empty() {
                    let _ = writeln!(out, "  ← {}", chain.join(" ← "));
                }
            }
        }

        for expansion in &self.selection.expanded {
            let _ = writeln!(
                out,
                "\nexpanded {} ({}) → {} node id(s)",
                expansion.from,
                expansion.kind.label(),
                expansion.into.len()
            );
        }

        // Said explicitly: a reader who counted node ids in `impacted-tests`
        // and sees whole files here would otherwise take the collapse for a
        // bug. It is the rule — a whole-file entry supersedes the per-test ids
        // inside it, so pytest is handed each file once.
        let superseded =
            self.selection.dropped.iter().filter(|d| d.why == DropReason::Superseded).count();
        if superseded > 0 {
            let whole =
                self.selection.node_ids.iter().filter(|s| !s.node_id.contains("::")).count();
            let _ = writeln!(
                out,
                "\n{superseded} per-test node id(s) folded into {whole} whole file(s): a file \
                 selected wholesale supersedes the node ids inside it"
            );
        }

        if !self.selection.dropped.is_empty() {
            let _ = writeln!(out, "\ndropped ({})", self.selection.dropped.len());
            for dropped in &self.selection.dropped {
                let _ = writeln!(out, "  {}  — {}", dropped.entry, dropped.why.label());
            }
        }

        if let Some(plan) = &self.fallback {
            let _ = writeln!(out);
            let _ = write!(out, "{}", plan.render_human());
        }

        if self.argv.is_empty() {
            // An empty argv means two different things, and the decision line
            // above says which: nothing was impacted, or pytest could not be
            // resolved to say what would have run.
            let _ = match self.selection.decision {
                Decision::Nothing => writeln!(out, "\nargv: none — nothing would be run"),
                _ => writeln!(out, "\nargv: unknown — pytest could not be resolved"),
            };
        } else {
            let _ = writeln!(out, "\nargv");
            for part in render_argv(&self.argv) {
                let _ = writeln!(out, "  {part}");
            }
        }
        out
    }

    pub fn render_json(&self) -> Result<String> {
        // Built by hand rather than through a `#[serde(flatten)]` wrapper: a
        // wrapper would need `Selection` to be `Deserialize` too, which it has
        // no reason to be.
        let mut value =
            serde_json::to_value(self.selection).context("could not serialise the selection")?;
        if let Some(object) = value.as_object_mut() {
            let argv = serde_json::to_value(render_argv(&self.argv))
                .context("could not serialise the pytest argv")?;
            object.insert("argv".to_string(), argv);
            // Always present, so the shape does not depend on the config.
            let fallback = serde_json::to_value(&self.fallback)
                .context("could not serialise the fallback plan")?;
            object.insert("fallback".to_string(), fallback);
            let git_env = serde_json::to_value(&self.git_env)
                .context("could not serialise the git environment plan")?;
            object.insert("git_env".to_string(), git_env);
        }
        serde_json::to_string_pretty(&value).context("could not render the dry run as JSON")
    }
}

const fn decision_label(decision: Decision) -> &'static str {
    match decision {
        Decision::Selected => "selected",
        Decision::RunAll => "run_all",
        Decision::Nothing => "nothing",
    }
}

/// The argv as printable strings. Lossy, and only ever for display.
fn render_argv(argv: &[OsString]) -> Vec<String> {
    argv.iter().map(|part| part.to_string_lossy().into_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::closure::{Reason, Verdict};
    use crate::select::{DropReason, Dropped, Expansion, ExpansionKind, Selected};

    fn selection(decision: Decision, ids: &[(&str, &str)]) -> Selection {
        Selection {
            verdict: if decision == Decision::RunAll { Verdict::RunAll } else { Verdict::Selected },
            reason: (decision == Decision::RunAll).then_some(Reason::NonPythonChanges),
            decision,
            node_ids: ids
                .iter()
                .map(|(node_id, origin)| Selected {
                    node_id: (*node_id).to_string(),
                    via: Vec::new(),
                    origin: Some((*origin).to_string()),
                })
                .collect(),
            expanded: Vec::new(),
            dropped: Vec::new(),
        }
    }

    fn runner() -> Runner {
        Runner::with_command(["uv", "run", "pytest"], "/repo")
    }

    /// The plan outside a hook: isolating, with nothing to remove.
    fn no_git() -> GitEnvPlan {
        GitEnvPlan::new(GitEnv::Isolate, |_| false)
    }

    fn argv_of(selection: &Selection, passthrough: &[&str]) -> Vec<String> {
        let passthrough: Vec<OsString> = passthrough.iter().map(OsString::from).collect();
        render_argv(&runner().argv(selection, &passthrough))
    }

    #[test]
    fn a_selection_becomes_the_runner_the_node_ids_and_the_passthrough() {
        let selection = selection(
            Decision::Selected,
            &[("tests/test_a.py::test_x", "pkg.a:f"), ("tests/test_b.py", "pkg.a:f")],
        );
        assert_eq!(
            argv_of(&selection, &["-x", "-n", "auto"]),
            vec![
                "uv",
                "run",
                "pytest",
                "tests/test_a.py::test_x",
                "tests/test_b.py",
                "-x",
                "-n",
                "auto"
            ],
            "the passthrough goes last, verbatim"
        );
    }

    #[test]
    fn a_run_all_selection_is_the_bare_runner() {
        assert_eq!(
            argv_of(&selection(Decision::RunAll, &[]), &["-q"]),
            vec!["uv", "run", "pytest", "-q"],
            "no node ids is what `the whole suite` looks like"
        );
    }

    #[test]
    fn the_multi_word_runner_stays_in_order() {
        let runner = Runner::with_command(["poetry", "run", "pytest"], "/repo");
        let argv = runner.argv(&selection(Decision::RunAll, &[]), &[]);
        assert_eq!(render_argv(&argv), vec!["poetry", "run", "pytest"]);
    }

    #[test]
    fn the_summary_line_counts_node_ids_and_origins() {
        let mut selection = selection(
            Decision::Selected,
            &[("tests/test_a.py::test_x", "pkg.a:f"), ("tests/test_b.py::test_y", "pkg.a:f")],
        );
        assert_eq!(
            summary(&selection, 640),
            "2 node id(s) from 1 origin(s) in 640 ms — details: gerenuk impacted-tests"
        );

        selection.node_ids[1].origin = Some("pkg.b:g".to_string());
        assert!(
            summary(&selection, 1).contains("from 2 origin(s)"),
            "distinct origins are counted"
        );
    }

    #[test]
    fn the_summary_line_names_the_reason_for_a_full_suite() {
        assert_eq!(
            summary(&selection(Decision::RunAll, &[]), 5),
            "full suite — non-Python files changed",
            "the hook's log has to say why it went wide"
        );
    }

    #[test]
    fn the_empty_outcome_says_so_rather_than_reporting_zero_tests() {
        assert_eq!(
            summary(&selection(Decision::Nothing, &[]), 5),
            "no tests impacted — nothing to run"
        );
    }

    #[test]
    fn the_dry_run_lists_the_argv_one_element_per_line() {
        let selection = selection(Decision::Selected, &[("tests/test_a.py::test_x", "pkg.a:f")]);
        let argv = runner().argv(&selection, &[]);
        let text = DryRun {
            selection: &selection,
            argv,
            elapsed_ms: 12,
            fallback: None,
            git_env: no_git(),
        }
        .render_human();

        assert!(text.contains("decision: selected"), "got:\n{text}");
        assert!(text.contains("\n  uv\n  run\n  pytest\n"), "not interpolatable, got:\n{text}");
        assert!(
            text.contains("tests/test_a.py::test_x\n  ← pkg.a:f"),
            "the chain names the edge to blame, got:\n{text}"
        );
    }

    #[test]
    fn the_dry_run_of_an_empty_selection_offers_no_argv() {
        let selection = selection(Decision::Nothing, &[]);
        let text = DryRun {
            selection: &selection,
            argv: Vec::new(),
            elapsed_ms: 3,
            fallback: None,
            git_env: no_git(),
        }
        .render_human();
        assert!(text.contains("decision: nothing"), "got:\n{text}");
        assert!(text.contains("argv: none"), "there is nothing to run, got:\n{text}");
    }

    #[test]
    fn the_dry_run_reports_expansions_and_drops() {
        let mut selection = selection(Decision::Selected, &[("tests/test_a.py", "pkg.a:f")]);
        selection.expanded = vec![Expansion {
            from: "tests.conftest:shelter".to_string(),
            kind: ExpansionKind::Fixture,
            into: vec!["tests/test_a.py".to_string()],
        }];
        selection.dropped = vec![Dropped {
            entry: "tests/test_a.py::test_x".to_string(),
            why: DropReason::Superseded,
        }];

        let text = DryRun {
            selection: &selection,
            argv: Vec::new(),
            elapsed_ms: 0,
            fallback: None,
            git_env: no_git(),
        }
        .render_human();
        assert!(text.contains("expanded tests.conftest:shelter (fixture)"), "got:\n{text}");
        assert!(text.contains("covered by the whole file"), "got:\n{text}");
        assert!(
            text.contains("1 per-test node id(s) folded into 1 whole file(s)"),
            "the collapse to file granularity is announced, got:\n{text}"
        );
    }

    #[test]
    fn the_dry_run_says_nothing_about_folding_when_nothing_folded() {
        let selection = selection(Decision::Selected, &[("tests/test_a.py::test_x", "pkg.a:f")]);
        let text = DryRun {
            selection: &selection,
            argv: Vec::new(),
            elapsed_ms: 0,
            fallback: None,
            git_env: no_git(),
        }
        .render_human();
        assert!(!text.contains("folded"), "got:\n{text}");
    }

    #[test]
    fn the_dry_run_json_carries_the_argv_alongside_the_selection() {
        let selection = selection(Decision::Selected, &[("tests/test_a.py::test_x", "pkg.a:f")]);
        let argv = runner().argv(&selection, &["-q".into()]);
        let text = DryRun {
            selection: &selection,
            argv,
            elapsed_ms: 0,
            fallback: None,
            git_env: no_git(),
        }
        .render_json()
        .expect("the selection serialises");
        let value: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");

        assert_eq!(value["decision"], "selected");
        assert_eq!(value["verdict"], "selected", "the impact verdict is carried through");
        assert_eq!(value["node_ids"][0]["node_id"], "tests/test_a.py::test_x");
        assert_eq!(value["argv"][0], "uv");
        assert_eq!(value["argv"][4], "-q", "the passthrough is part of the pinned shape");
        assert!(value["fallback"].is_null(), "present and null when none applies: {value}");
    }

    #[test]
    fn the_dry_run_of_a_delegated_run_all_names_the_fallback_in_both_forms() {
        use crate::fallback::{self, Payload};

        let selection = selection(Decision::RunAll, &[]);
        let config = Config {
            fallback_command: Some(vec!["scripts/pick.sh".into(), "--from-gerenuk".into()]),
            ..Config::default()
        };
        let fallback = fallback::resolve(None, None, &config, std::path::Path::new("/repo"))
            .expect("valid")
            .expect("configured");
        let plan = fallback::Plan::new(&fallback, Payload::new(selection.reason, None));
        let report = DryRun {
            selection: &selection,
            argv: fallback.argv().to_vec(),
            elapsed_ms: 0,
            fallback: Some(plan),
            git_env: GitEnvPlan::inherited(),
        };

        let text = report.render_human();
        assert!(text.contains("decision: run_all"), "got:\n{text}");
        assert!(
            text.contains(
                "would exec fallback: [\"/repo/scripts/pick.sh\",\"--from-gerenuk\"] \
                 (reason: non_python_changes)"
            ),
            "got:\n{text}"
        );
        assert!(text.contains("\n  /repo/scripts/pick.sh\n  --from-gerenuk\n"), "got:\n{text}");

        let value: serde_json::Value =
            serde_json::from_str(&report.render_json().expect("serialises")).expect("JSON");
        assert_eq!(value["argv"][0], "/repo/scripts/pick.sh", "the argv is the fallback's");
        assert_eq!(value["fallback"]["source"], "config");
        assert_eq!(value["fallback"]["reason"], "non_python_changes");
        assert_eq!(value["fallback"]["payload"]["gerenuk_fallback_payload_version"], 1);
        assert!(value["fallback"]["payload"]["report"].is_null());
    }

    #[test]
    fn the_override_beats_the_config_and_the_path() {
        let config = Config { pytest_command: vec!["hatch".to_string()], ..Config::default() };
        let runner = Runner::resolve(Some("/usr/bin/pytest".into()), &config, "/repo")
            .expect("the override names a command");
        assert_eq!(
            render_argv(&runner.command),
            vec!["/usr/bin/pytest"],
            "the override is consulted before the config"
        );
    }

    #[test]
    fn the_configured_command_is_used_before_path() {
        let config = Config { pytest_command: vec!["hatch".to_string()], ..Config::default() };
        let runner = Runner::resolve(None, &config, "/repo").expect("the config names a command");
        assert_eq!(
            render_argv(&runner.command),
            vec!["hatch"],
            "the config is consulted before PATH"
        );
    }

    #[test]
    fn an_empty_config_command_falls_through_rather_than_exec_ing_nothing() {
        let config = Config { pytest_command: Vec::new(), ..Config::default() };
        // Resolution may or may not find pytest on this machine; either way it
        // must never resolve to the empty argv, and a failure has to say what
        // the user could set instead.
        match Runner::resolve(None, &config, "/repo") {
            Ok(runner) => assert!(!runner.command.is_empty(), "an empty argv is not runnable"),
            Err(err) => {
                let text = format!("{err:#}");
                assert!(text.contains("PATH"), "got: {text}");
                assert!(text.contains("pytest-command"), "got: {text}");
                assert!(text.contains(PYTEST_BIN_ENV), "got: {text}");
            }
        }
    }

    #[test]
    fn an_empty_argv_is_refused_rather_than_spawned() {
        let runner = Runner::with_command(Vec::<OsString>::new(), "/repo");
        let err = runner.exec(&[], Handoff::default()).expect_err("there is no program to run");
        assert!(format!("{err:#}").contains("empty argv"), "got: {err:#}");
    }

    // --- The git environment ---

    #[test]
    fn the_local_git_variables_are_the_ones_a_hook_exports() {
        for name in ["GIT_DIR", "GIT_INDEX_FILE", "GIT_WORK_TREE", "GIT_PREFIX", "GIT_COMMON_DIR"] {
            assert!(LOCAL_GIT_ENV.contains(&name), "`{name}` is what the incidents were about");
        }
        for name in ["GIT_AUTHOR_NAME", "GIT_EDITOR", "GIT_CONFIG_GLOBAL", "PATH"] {
            assert!(!LOCAL_GIT_ENV.contains(&name), "`{name}` is not repository-local");
        }
    }

    #[test]
    fn isolating_removes_only_the_variables_that_are_set() {
        let set = |name: &str| name == "GIT_DIR" || name == "GIT_INDEX_FILE";
        assert_eq!(
            GitEnv::Isolate.removals(set),
            vec!["GIT_DIR", "GIT_INDEX_FILE"],
            "in git's own order, and nothing that is not there"
        );
        assert_eq!(GitEnv::Isolate.removals(|_| false), Vec::<&str>::new(), "outside a hook");
    }

    #[test]
    fn inheriting_removes_nothing_even_when_everything_is_set() {
        assert_eq!(GitEnv::Inherit.removals(|_| true), Vec::<&str>::new());
    }

    #[test]
    fn the_default_policy_is_to_isolate() {
        assert_eq!(GitEnv::default(), GitEnv::Isolate, "the safe default needs no config");
        assert_eq!(GitEnv::Isolate.name(), "isolate");
        assert_eq!(GitEnv::Inherit.name(), "inherit");
    }

    #[test]
    fn the_policy_names_are_what_serde_reads_and_writes() {
        for policy in [GitEnv::Isolate, GitEnv::Inherit] {
            let json = serde_json::to_string(&policy).expect("serialises");
            assert_eq!(json, format!("\"{}\"", policy.name()), "`name` pins the wire form");
            let back: GitEnv = serde_json::from_str(&json).expect("round-trips");
            assert_eq!(back, policy);
        }
        assert!(serde_json::from_str::<GitEnv>("\"strip\"").is_err(), "no third value");
    }

    #[test]
    fn pytests_handoff_removes_and_adds_nothing_else() {
        let handoff = Handoff::for_pytest(&["GIT_DIR", "GIT_PREFIX"]);
        assert_eq!(handoff.remove, vec![OsString::from("GIT_DIR"), OsString::from("GIT_PREFIX")]);
        assert_eq!(handoff.stdin, None, "pytest reads nothing from gerenuk");
        assert!(handoff.env.is_empty(), "and gets no variable of gerenuk's");
        assert_eq!(Handoff::default().remove, Vec::<OsString>::new(), "the fallback's loses none");
    }

    #[test]
    fn the_git_env_plan_says_what_would_go() {
        let plan =
            GitEnvPlan::new(GitEnv::Isolate, |name| name == "GIT_DIR" || name == "GIT_PREFIX");
        assert_eq!(plan.render_human(), "git env: isolate — removes GIT_DIR, GIT_PREFIX");
        assert_eq!(
            no_git().render_human(),
            "git env: isolate — no repository-local git variable is set",
            "outside a hook the policy is still stated, so a reader knows it applies"
        );
        assert_eq!(GitEnvPlan::new(GitEnv::Inherit, |_| true).render_human(), "git env: inherit");
        assert_eq!(
            GitEnvPlan::inherited(),
            GitEnvPlan { policy: GitEnv::Inherit, removed: vec![] }
        );

        let value = serde_json::to_value(&plan).expect("serialises");
        assert_eq!(value["policy"], "isolate");
        assert_eq!(value["removed"][1], "GIT_PREFIX");
    }

    #[test]
    fn the_dry_run_states_the_git_env_for_every_outcome_that_runs_something() {
        let selection = selection(Decision::Selected, &[("tests/test_a.py::test_x", "pkg.a:f")]);
        let git_env = GitEnvPlan::new(GitEnv::Isolate, |name| name == "GIT_INDEX_FILE");
        let report = DryRun {
            selection: &selection,
            argv: Vec::new(),
            elapsed_ms: 0,
            fallback: None,
            git_env,
        };
        let text = report.render_human();
        assert!(text.contains("\ngit env: isolate — removes GIT_INDEX_FILE\n"), "got:\n{text}");
        let value: serde_json::Value =
            serde_json::from_str(&report.render_json().expect("serialises")).expect("JSON");
        assert_eq!(value["git_env"]["policy"], "isolate");
        assert_eq!(value["git_env"]["removed"][0], "GIT_INDEX_FILE");

        let nothing = self::selection(Decision::Nothing, &[]);
        let report = DryRun {
            selection: &nothing,
            argv: Vec::new(),
            elapsed_ms: 0,
            fallback: None,
            git_env: no_git(),
        };
        assert!(
            !report.render_human().contains("git env"),
            "nothing runs, so there is no child to describe"
        );
    }
}
