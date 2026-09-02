//! What `gerenuk run` delegates to when the outcome is `run_all`.
//!
//! A `run_all` outcome means gerenuk could not bound the impact of a change.
//! The default answer is the whole suite under pytest; a repository with its
//! own way of narrowing work — a script that maps changed files to
//! sub-projects, say — can name it as a **fallback command**, and `run` execs
//! that instead. The `selected` and `nothing` outcomes never come here.
//!
//! Everything in this module is pure: resolving the command from its three
//! possible sources, deciding where its program lives, and building the
//! payload it is handed. The exec itself goes through the one existing seam,
//! [`crate::pytest::Runner::exec`], because the fallback is subject to exactly
//! the same contract as pytest — gerenuk's process *becomes* it, and its exit
//! code is the hook's. See `docs/adr/0014-run-all-delegates-to-a-fallback.md`.
//!
//! **Delegation only.** The fallback receives context and owns the run from
//! there; it has no way to hand a selection back. A two-way protocol would be
//! a new, versioned mechanism, not a widening of this one.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::changed::ChangedSymbols;
use crate::closure::Reason;
use crate::config::Config;
use crate::pytest::Handoff;

/// Environment variable naming the fallback, as a JSON array of strings.
pub const FALLBACK_ENV: &str = "GERENUK_FALLBACK";

/// Environment variable the fallback finds the reason in, so a shell script can
/// branch on it without parsing the payload.
pub const REASON_ENV: &str = "GERENUK_FALLBACK_REASON";

/// The version stamped into every payload. Adding a field or a reason variant
/// keeps it; renaming or removing one bumps it.
pub const PAYLOAD_VERSION: u32 = 1;

/// The reason name when there is none: a replayed `run_all` report whose
/// `reason` is `null`.
pub const UNSPECIFIED_REASON: &str = "unspecified";

/// Where a fallback command came from, highest precedence first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// `--fallback-command`.
    Flag,
    /// [`FALLBACK_ENV`].
    Env,
    /// `fallback-command` under `[tool.gerenuk]`.
    Config,
}

impl Source {
    /// How an error message names the source.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Flag => "--fallback-command",
            Self::Env => FALLBACK_ENV,
            Self::Config => "fallback-command in pyproject.toml",
        }
    }
}

/// A resolved fallback: the argv to exec, and where it was configured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fallback {
    argv: Vec<OsString>,
    source: Source,
}

impl Fallback {
    fn new(argv: Vec<String>, source: Source, root: &Path) -> Self {
        let mut argv = argv.into_iter().map(OsString::from).collect::<Vec<_>>();
        if let Some(program) = argv.first_mut() {
            *program = resolve_program(&program.to_string_lossy(), root).into_os_string();
        }
        Self { argv, source }
    }

    /// The argv, with its program already resolved. Never empty.
    #[must_use]
    pub fn argv(&self) -> &[OsString] {
        &self.argv
    }

    /// The program that will be exec'd, as resolved.
    #[must_use]
    pub fn program(&self) -> &Path {
        self.argv.first().map_or_else(|| Path::new(""), Path::new)
    }

    #[must_use]
    pub const fn source(&self) -> Source {
        self.source
    }

    /// The argv as printable strings. Lossy, and only ever for display.
    #[must_use]
    pub fn rendered_argv(&self) -> Vec<String> {
        self.argv.iter().map(|part| part.to_string_lossy().into_owned()).collect()
    }

    /// One line for a log: the argv as a JSON array, and its source.
    #[must_use]
    pub fn describe(&self) -> String {
        format!("{} (from {})", json_array(&self.rendered_argv()), self.source.label())
    }
}

/// Resolve the fallback from its three sources, highest precedence first.
///
/// `--fallback-command`, then [`FALLBACK_ENV`], then `fallback-command` in
/// `pyproject.toml`. `None` means none was configured anywhere, and the
/// default — pytest, full suite — applies.
///
/// Every layer is validated, not just the winning one: an empty argv anywhere
/// is a configuration error, found here at startup rather than on the day the
/// bail-out first happens. The flag and the variable arrive as arguments, the
/// way [`crate::pytest::Runner::resolve`] takes its override, so this stays a
/// pure function of what it is given.
pub fn resolve(
    flag: Option<&str>,
    env: Option<&str>,
    config: &Config,
    root: &Path,
) -> Result<Option<Fallback>> {
    let flag = flag.map(|text| parse_json_argv(text, Source::Flag)).transpose()?;
    let env = env.map(|text| parse_json_argv(text, Source::Env)).transpose()?;
    let config =
        config.fallback_command.clone().map(|argv| non_empty(argv, Source::Config)).transpose()?;

    let chosen = [(flag, Source::Flag), (env, Source::Env), (config, Source::Config)]
        .into_iter()
        .find_map(|(argv, source)| argv.map(|argv| (argv, source)));
    Ok(chosen.map(|(argv, source)| Fallback::new(argv, source, root)))
}

fn parse_json_argv(text: &str, source: Source) -> Result<Vec<String>> {
    let argv: Vec<String> = serde_json::from_str(text).with_context(|| {
        format!(
            "{} must be a JSON array of strings, e.g. [\"scripts/pick.sh\", \"--from-gerenuk\"]; \
             got: {text}",
            source.label()
        )
    })?;
    non_empty(argv, source)
}

fn non_empty(argv: Vec<String>, source: Source) -> Result<Vec<String>> {
    if argv.is_empty() {
        bail!(
            "{} is an empty argv; name a command to run, or remove it to keep the default \
             (the full suite under pytest)",
            source.label()
        );
    }
    Ok(argv)
}

/// Where the program is looked for.
///
/// A bare name is left for the exec to find on `PATH`, like any other
/// program. Anything with a path component in it is a path, and a relative one
/// is relative to the **repository root** — the same root `pyproject.toml`
/// lives in — never to the directory gerenuk happened to be run from.
#[must_use]
pub fn resolve_program(program: &str, root: &Path) -> PathBuf {
    let path = Path::new(program);
    if path.is_absolute() || path.components().count() > 1 {
        root.join(path)
    } else {
        path.to_path_buf()
    }
}

/// The stable name of a `run_all` reason, or [`UNSPECIFIED_REASON`].
#[must_use]
pub fn reason_name(reason: Option<Reason>) -> &'static str {
    reason.map_or(UNSPECIFIED_REASON, Reason::name)
}

/// What the fallback reads on stdin. This is an external contract.
///
/// `report` is the phase-1 `changed-symbols` report the run was computed from
/// — the changed symbols, module-level changes, non-Python changes, changed
/// test files and parse errors, in exactly the shape `changed-symbols --format
/// json` prints. It is `null` when `run --impact` replayed a saved report and
/// never diffed the tree, rather than a fabricated empty report that would
/// read as "nothing changed".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Payload<'a> {
    pub gerenuk_fallback_payload_version: u32,
    /// [`Reason::name`], or [`UNSPECIFIED_REASON`].
    pub reason: &'static str,
    pub report: Option<&'a ChangedSymbols>,
}

impl<'a> Payload<'a> {
    #[must_use]
    pub fn new(reason: Option<Reason>, report: Option<&'a ChangedSymbols>) -> Self {
        Self {
            gerenuk_fallback_payload_version: PAYLOAD_VERSION,
            reason: reason_name(reason),
            report,
        }
    }

    /// The bytes written to the fallback's stdin.
    pub fn render(&self) -> Result<Vec<u8>> {
        let mut bytes =
            serde_json::to_vec_pretty(self).context("could not serialise the fallback payload")?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// Everything the fallback receives besides its argv: the payload on
    /// stdin and the reason in its environment.
    pub fn handoff(&self) -> Result<Handoff> {
        Ok(Handoff {
            stdin: Some(self.render()?),
            env: vec![(OsString::from(REASON_ENV), OsString::from(self.reason))],
        })
    }
}

/// What `--dry-run` reports about the fallback it would have exec'd.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Plan<'a> {
    pub argv: Vec<String>,
    pub source: Source,
    pub reason: &'static str,
    pub payload: Payload<'a>,
}

impl<'a> Plan<'a> {
    #[must_use]
    pub fn new(fallback: &Fallback, payload: Payload<'a>) -> Self {
        Self {
            argv: fallback.rendered_argv(),
            source: fallback.source(),
            reason: payload.reason,
            payload,
        }
    }

    /// The line the human dry run prints for it.
    #[must_use]
    pub fn render_human(&self) -> String {
        format!(
            "would exec fallback: {} (reason: {})\n  from: {}\n",
            json_array(&self.argv),
            self.reason,
            self.source.label()
        )
    }
}

fn json_array(argv: &[String]) -> String {
    serde_json::to_string(argv).unwrap_or_else(|_| format!("{argv:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "/repo";

    fn root() -> &'static Path {
        Path::new(ROOT)
    }

    fn config(argv: Option<&[&str]>) -> Config {
        Config {
            fallback_command: argv.map(|argv| argv.iter().map(ToString::to_string).collect()),
            ..Config::default()
        }
    }

    fn argv_of(fallback: &Fallback) -> Vec<String> {
        fallback.rendered_argv()
    }

    #[test]
    fn nothing_configured_anywhere_means_the_default() {
        let resolved = resolve(None, None, &config(None), root()).expect("absent is fine");
        assert_eq!(resolved, None, "no fallback means pytest runs the full suite");
    }

    #[test]
    fn the_config_is_used_when_nothing_overrides_it() {
        let fallback = resolve(None, None, &config(Some(&["scripts/pick.sh", "-v"])), root())
            .expect("valid")
            .expect("configured");
        assert_eq!(fallback.source(), Source::Config);
        assert_eq!(
            argv_of(&fallback),
            vec!["/repo/scripts/pick.sh", "-v"],
            "the program is resolved against the root, the arguments are untouched"
        );
    }

    #[test]
    fn the_env_beats_the_config_and_the_flag_beats_the_env() {
        let config = config(Some(&["from-config"]));

        let from_env =
            resolve(None, Some(r#"["from-env"]"#), &config, root()).expect("valid").expect("set");
        assert_eq!((from_env.source(), argv_of(&from_env)), (Source::Env, vec!["from-env".into()]));

        let from_flag = resolve(Some(r#"["from-flag"]"#), Some(r#"["from-env"]"#), &config, root())
            .expect("valid")
            .expect("set");
        assert_eq!(
            (from_flag.source(), argv_of(&from_flag)),
            (Source::Flag, vec!["from-flag".into()])
        );
    }

    #[test]
    fn an_empty_argv_is_an_error_at_whichever_layer_it_sits() {
        // Even when a higher layer would win: a config error is a config error.
        let err = resolve(Some(r#"["from-flag"]"#), None, &config(Some(&[])), root())
            .expect_err("the config is empty");
        assert!(format!("{err:#}").contains("fallback-command in pyproject.toml"), "got: {err:#}");
        assert!(format!("{err:#}").contains("empty"), "got: {err:#}");

        let err = resolve(None, Some("[]"), &config(None), root()).expect_err("the env is empty");
        assert!(format!("{err:#}").contains(FALLBACK_ENV), "got: {err:#}");

        let err = resolve(Some("[]"), None, &config(None), root()).expect_err("the flag is empty");
        assert!(format!("{err:#}").contains("--fallback-command"), "got: {err:#}");
    }

    #[test]
    fn a_shell_string_is_refused_and_the_error_names_the_source() {
        let err = resolve(Some("scripts/pick.sh -v"), None, &config(None), root())
            .expect_err("not a JSON array");
        let text = format!("{err:#}");
        assert!(text.contains("--fallback-command"), "got: {text}");
        assert!(text.contains("JSON array"), "got: {text}");

        let err = resolve(None, Some(r"[1, 2]"), &config(None), root())
            .expect_err("an array, but not of strings");
        assert!(format!("{err:#}").contains(FALLBACK_ENV), "got: {err:#}");
    }

    #[test]
    fn a_bare_name_is_left_for_path_and_a_path_is_rooted() {
        assert_eq!(resolve_program("pick", root()), PathBuf::from("pick"), "PATH lookup");
        assert_eq!(
            resolve_program("scripts/pick.sh", root()),
            PathBuf::from("/repo/scripts/pick.sh"),
            "relative to the repo root, not the cwd"
        );
        assert_eq!(
            resolve_program("./pick.sh", root()),
            PathBuf::from("/repo/./pick.sh"),
            "a `./` prefix is a path too"
        );
        assert_eq!(
            resolve_program("/opt/bin/pick", root()),
            PathBuf::from("/opt/bin/pick"),
            "absolute stays absolute"
        );
    }

    #[test]
    fn the_reason_name_is_the_json_name_or_unspecified() {
        assert_eq!(reason_name(Some(Reason::MaxDepth)), "max_depth");
        assert_eq!(reason_name(None), "unspecified", "a replayed report may carry no reason");
    }

    #[test]
    fn the_payload_has_exactly_the_pinned_shape() {
        let report = ChangedSymbols {
            base: "main".into(),
            merge_base: "abc".into(),
            non_python_changes: vec!["requirements.txt".into()],
            ..ChangedSymbols::default()
        };
        let payload = Payload::new(Some(Reason::NonPythonChanges), Some(&report));
        let value: serde_json::Value =
            serde_json::from_slice(&payload.render().expect("serialises")).expect("JSON");

        let keys: Vec<&str> =
            value.as_object().expect("an object").keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            vec!["gerenuk_fallback_payload_version", "reason", "report"],
            "three keys, and no more: this is the external contract"
        );
        assert_eq!(value["gerenuk_fallback_payload_version"], 1);
        assert_eq!(value["reason"], "non_python_changes");
        assert_eq!(
            value["report"]["non_python_changes"][0], "requirements.txt",
            "the report is the changed-symbols schema, not a copy of it"
        );
        assert_eq!(value["report"]["base"], "main");
    }

    #[test]
    fn a_replayed_run_has_no_report_to_hand_over() {
        let value = serde_json::to_value(Payload::new(None, None)).expect("serialises");
        assert!(value["report"].is_null(), "null, never a fabricated empty report");
        assert_eq!(value["reason"], "unspecified");
    }

    #[test]
    fn the_handoff_carries_the_payload_on_stdin_and_the_reason_in_the_env() {
        let payload = Payload::new(Some(Reason::Budget), None);
        let handoff = payload.handoff().expect("serialises");
        let stdin = handoff.stdin.expect("the payload is delivered on stdin");
        assert!(stdin.ends_with(b"\n"), "newline-terminated, for `cat`-shaped scripts");
        let value: serde_json::Value = serde_json::from_slice(&stdin).expect("JSON");
        assert_eq!(value["reason"], "budget");
        assert_eq!(
            handoff.env,
            vec![(OsString::from(REASON_ENV), OsString::from("budget"))],
            "the reason alone, so a shell script can branch without parsing JSON"
        );
    }

    #[test]
    fn the_plan_names_the_argv_the_source_and_the_reason() {
        let fallback = resolve(None, None, &config(Some(&["scripts/pick.sh"])), root())
            .expect("valid")
            .expect("set");
        let plan = Plan::new(&fallback, Payload::new(Some(Reason::TyfUnavailable), None));
        let text = plan.render_human();
        assert_eq!(
            text,
            "would exec fallback: [\"/repo/scripts/pick.sh\"] (reason: tyf_unavailable)\n  \
             from: fallback-command in pyproject.toml\n"
        );
        let value = serde_json::to_value(&plan).expect("serialises");
        assert_eq!(value["source"], "config");
        assert_eq!(value["argv"][0], "/repo/scripts/pick.sh");
        assert_eq!(value["payload"]["reason"], "tyf_unavailable", "the full payload is included");
    }

    #[test]
    fn the_description_is_the_argv_and_its_source() {
        let fallback = resolve(Some(r#"["pick", "--from-gerenuk"]"#), None, &config(None), root())
            .expect("valid")
            .expect("set");
        assert_eq!(fallback.describe(), "[\"pick\",\"--from-gerenuk\"] (from --fallback-command)");
        assert_eq!(fallback.program(), Path::new("pick"));
    }
}
