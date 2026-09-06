//! Agent-facing instructions for the three moments someone meets gerenuk.
//!
//! The prose lives in `docs/src/guide/*.md` and is pulled in with
//! [`include_str!`], so the documentation site and the CLI serve the same
//! bytes. There is no guide text in this file, and there must never be: a
//! second copy is a copy that drifts.
//!
//! `guide` is the one command an agent runs *before* anything is set up, so
//! it needs no repository, no `tyf` and no `git`. Topic selection reads one
//! file, `./pyproject.toml`, and nothing else; [`Cli::run`] dispatches it
//! ahead of workspace detection so nothing below that line can run for it.
//!
//! Guide authors have one constraint, enforced by the tests below and in
//! `cli.rs`: every gerenuk invocation a page shows must survive the
//! extraction below. It understands a `|` pipeline, a `>` redirect and
//! `<placeholder>` holes, and nothing else — `&&` and `;` are arguments to
//! it, and a command using them fails the parse check with a message blaming
//! the CLI. Put those outside the backticks.
//!
//! [`Cli::run`]: crate::cli::Cli::run

use std::path::Path;

/// Instructions for a repository gerenuk is not wired into yet.
const SETUP: &str = include_str!("../docs/src/guide/setup.md");
/// Instructions for reading a report, and the `run_all` ladder.
const TRIAGE: &str = include_str!("../docs/src/guide/triage.md");
/// Reference for the base, the budgets, the config keys and the binaries.
const TUNE: &str = include_str!("../docs/src/guide/tune.md");

/// Which set of instructions to print.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Topic {
    /// gerenuk is not wired into this repository yet.
    Setup,
    /// A run said `run_all`, or a selection surprised you.
    Triage,
    /// The base, the budgets, the config keys and the binaries.
    Tune,
}

impl Topic {
    /// The topic's name as written on the command line and in the header.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Triage => "triage",
            Self::Tune => "tune",
        }
    }

    /// The guide text, byte-identical to the docs page it is included from.
    #[must_use]
    pub const fn text(self) -> &'static str {
        match self {
            Self::Setup => SETUP,
            Self::Triage => TRIAGE,
            Self::Tune => TUNE,
        }
    }

    /// Every topic, taken from the `ValueEnum` derive so a fourth variant is
    /// covered by every content check without being listed by hand.
    pub fn all() -> impl Iterator<Item = Self> {
        <Self as clap::ValueEnum>::value_variants().iter().copied()
    }
}

/// What made a directory count as configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    /// A `[tool.madoqua]` table in `pyproject.toml` that names gerenuk as a
    /// step: the hook runs it, so the reader is here to read a report.
    MadoquaStep,
}

impl ConfigSource {
    /// How the header line names this source.
    const fn label(self) -> &'static str {
        match self {
            Self::MadoquaStep => "pyproject.toml [tool.madoqua]",
        }
    }
}

/// How the printed topic was chosen, carried into the header so a reader that
/// passed no topic can see why it got the text it got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    /// The user named the topic on the command line.
    Explicit,
    /// The topic was derived from what was found in the working directory.
    Auto(Option<ConfigSource>),
}

/// Report whether gerenuk is wired into `dir`, and by what.
///
/// Looks at `dir/pyproject.toml` only. Walking up would make the answer depend
/// on where the caller happened to stand; the guide tells its reader to run at
/// the repository root instead. Unreadable or unparseable files are "not
/// configured", not errors: a guide that refuses to print has failed at its
/// one job.
#[must_use]
pub fn detect(dir: &Path) -> Option<ConfigSource> {
    let Ok(contents) = std::fs::read_to_string(dir.join("pyproject.toml")) else {
        return None;
    };
    let Ok(document) = toml::from_str::<toml::Table>(&contents) else {
        return None;
    };
    let madoqua = document.get("tool").and_then(|tool| tool.get("madoqua"))?;
    // Parsed rather than grepped: `gerenuk` in a comment or in another tool's
    // table is not a hook step.
    mentions_gerenuk(madoqua).then_some(ConfigSource::MadoquaStep)
}

/// Whether any string anywhere under `value` contains `gerenuk`.
///
/// A step is `"gerenuk run -- -q"` in string form or `{ cmd = "gerenuk ..." }`
/// in table form, under `check` or `extend_check`; walking every string is
/// simpler than knowing madoqua's schema, and cannot be out of date with it.
fn mentions_gerenuk(value: &toml::Value) -> bool {
    match value {
        toml::Value::String(s) => s.contains("gerenuk"),
        toml::Value::Array(items) => items.iter().any(mentions_gerenuk),
        toml::Value::Table(table) => table.values().any(mentions_gerenuk),
        _ => false,
    }
}

/// The topic to print when the user named none.
///
/// `tune` is never auto-selected: it is a reference, and nothing about a
/// repository's state says "you need the reference right now".
#[must_use]
pub const fn auto_topic(source: Option<ConfigSource>) -> Topic {
    if source.is_some() {
        Topic::Triage
    } else {
        Topic::Setup
    }
}

/// The first line of every guide, naming the topic and how it was chosen.
fn header(topic: Topic, selection: Selection) -> String {
    match selection {
        Selection::Explicit => format!("# gerenuk guide: {}", topic.name()),
        Selection::Auto(None) => {
            format!("# gerenuk guide: not configured here -> {}", topic.name())
        }
        Selection::Auto(Some(source)) => {
            format!("# gerenuk guide: configured via {} -> {}", source.label(), topic.name())
        }
    }
}

/// The complete guide output: header line, blank line, then the docs page
/// verbatim.
#[must_use]
pub fn render(topic: Topic, selection: Selection) -> String {
    format!("{}\n\n{}", header(topic, selection), topic.text())
}

/// Every gerenuk invocation the guides show, as argv vectors ready for clap.
///
/// Test-only, like the helpers below it: they exist to hold the pages to their
/// promises. Crate-visible because the check that matters — feeding each one
/// through the real `Cli` — lives in `cli.rs`, where the type is.
///
/// Placeholders like `<REF>` are dropped: they are holes for the reader, not
/// arguments. A `>` redirect ends the command.
#[cfg(test)]
#[must_use]
pub(crate) fn embedded_invocations(topic: Topic) -> Vec<Vec<&'static str>> {
    command_lines(topic.text()).into_iter().flat_map(gerenuk_invocations).collect()
}

/// Command strings written in a guide: inline backtick spans that invoke
/// gerenuk, plus every non-blank line of a fenced `bash` block.
#[cfg(test)]
fn command_lines(text: &str) -> Vec<&str> {
    let mut out: Vec<&str> = inline_code_spans(text).into_iter().filter(invokes).collect();
    out.extend(
        lines_with_fence(text)
            .filter(|&(line, fence)| fence == Some("bash") && !line.trim().is_empty())
            .map(|(line, _)| line),
    );
    out
}

/// Split a command line on pipes and redirects and keep the segments that
/// invoke gerenuk.
#[cfg(test)]
fn gerenuk_invocations(line: &str) -> Vec<Vec<&str>> {
    line.split(['|', '>'])
        .map(str::trim)
        .filter(invokes)
        .map(|segment| {
            segment
                .split_whitespace()
                .filter(|token| !token.contains('<') && !token.contains('>'))
                .collect()
        })
        .collect()
}

/// Whether a command string is a gerenuk invocation rather than prose or
/// another tool.
#[cfg(test)]
fn invokes(command: &&str) -> bool {
    *command == "gerenuk" || command.starts_with("gerenuk ")
}

/// Inline `code` spans outside fences, in source order.
#[cfg(test)]
pub(crate) fn inline_code_spans(text: &str) -> Vec<&str> {
    let mut spans = Vec::new();
    for (line, fence) in lines_with_fence(text) {
        if fence.is_some() {
            continue;
        }
        let mut rest = line;
        while let Some(open) = rest.find('`') {
            let after = &rest[open + 1..];
            let Some(close) = after.find('`') else { break };
            spans.push(&after[..close]);
            rest = &after[close + 1..];
        }
    }
    spans
}

/// Each line of `text` paired with the info string of the fence it sits in,
/// or `None` outside one. Fence markers themselves are not yielded.
#[cfg(test)]
pub(crate) fn lines_with_fence(text: &str) -> impl Iterator<Item = (&str, Option<&str>)> {
    let mut fence: Option<&str> = None;
    text.lines().filter_map(move |line| {
        if let Some(info) = line.strip_prefix("```") {
            fence = if fence.is_some() { None } else { Some(info.trim()) };
            return None;
        }
        Some((line, fence))
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::closure::Reason;

    /// No topic may exceed this many lines. Failing this test is the point of
    /// it: a guide that grows past a screenful stops being read.
    const LINE_CAP: usize = 60;

    fn write(dir: &Path, name: &str, contents: &str) {
        std::fs::write(dir.join(name), contents).expect("fixture write should succeed");
    }

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir should be creatable")
    }

    // --- Content rules ---

    #[test]
    fn every_topic_fits_the_line_cap() {
        for topic in Topic::all() {
            let lines = topic.text().trim_end().lines().count();
            assert!(
                lines <= LINE_CAP,
                "guide `{}` is {lines} lines, cap is {LINE_CAP}; cut it rather than raising \
                 the cap",
                topic.name(),
            );
            assert!(lines > 10, "guide `{}` is {lines} lines, which is not a guide", topic.name());
        }
    }

    #[test]
    fn every_topic_is_plain_ascii() {
        for topic in Topic::all() {
            let offender = topic.text().chars().find(|c| !c.is_ascii());
            assert!(
                offender.is_none(),
                "guide `{}` contains the non-ASCII character {offender:?}; guides are piped \
                 and captured, so they stay ASCII",
                topic.name(),
            );
        }
    }

    #[test]
    fn every_topic_ends_with_a_single_next_line() {
        for topic in Topic::all() {
            let trimmed = topic.text().trim_end();
            let last = trimmed.lines().next_back().expect("guide should not be empty");
            assert!(
                last.starts_with("next: run "),
                "guide `{}` must end with a `next: run` line, ends with {last:?}",
                topic.name(),
            );
            let count = trimmed.lines().filter(|l| l.starts_with("next: run ")).count();
            assert_eq!(count, 1, "guide `{}` should have exactly one next line", topic.name());
            assert!(
                topic.text().ends_with('\n'),
                "guide `{}` must end with a newline, or the shell prompt lands mid-line",
                topic.name(),
            );
        }
    }

    #[test]
    fn setup_installs_then_wires_the_hook_then_verifies_with_a_diff() {
        let text = Topic::Setup.text();
        let install = text.find("uv add --dev gerenuk ty-find pytest").expect("setup installs");
        let step = text.find("[tool.madoqua]").expect("setup shows the madoqua step");
        let verify = text.find("gerenuk impacted-tests").expect("setup verifies with a walk");
        assert!(install < step && step < verify, "install, wire, verify - in that order");
        assert!(
            text.contains(r#"{ name = "gerenuk", cmd = "gerenuk run -- -q", pass_files = false, timeout_s = 120 }"#),
            "the madoqua step is copy-pasted verbatim, so it is pinned verbatim",
        );
        assert!(
            text.contains("not the index"),
            "setup must say the diff is the working tree, so unstaged edits count in a hook",
        );
        assert!(text.contains("`origin/main` (then `main`, then `master`)"), "the base chain");
        assert!(text.contains("background daemon"), "setup states the daemon fact");
        assert_eq!(text.matches("daemon").count(), 1, "and states it exactly once");
    }

    /// The ladder has to cover every reason `run_all` can print, by the exact
    /// label the CLI uses, or the reader will meet a line the guide never
    /// mentions.
    #[test]
    fn triage_names_every_run_all_reason_by_its_label() {
        let text = Topic::Triage.text().split_whitespace().collect::<Vec<_>>().join(" ");
        for reason in Reason::ALL {
            let label = reason.label();
            assert!(
                text.contains(&format!("`{label}`")),
                "triage guide should name the reason {label:?} as the CLI prints it",
            );
        }
    }

    #[test]
    fn triage_states_the_three_outcomes_and_the_prohibitions() {
        let text = Topic::Triage.text();
        for phrase in ["`selected`:", "`run_all`:", "nothing:", "**Do not:**", "--impact"] {
            assert!(text.contains(phrase), "triage guide should contain {phrase:?}");
        }
    }

    #[test]
    fn tune_states_the_defaults_the_code_uses() {
        use crate::closure::{DEFAULT_BUDGET_MS, DEFAULT_MAX_DEPTH, DEFAULT_MAX_SYMBOLS};
        let text = Topic::Tune.text();
        for (key, default) in [
            ("max-depth", DEFAULT_MAX_DEPTH.to_string()),
            ("max-symbols", DEFAULT_MAX_SYMBOLS.to_string()),
            ("budget-ms", DEFAULT_BUDGET_MS.to_string()),
        ] {
            assert!(
                text.contains(&format!("| `{key}` | {default} |")),
                "tune should state the default of `{key}` as {default}",
            );
        }
        for var in ["GERENUK_TYF", "GERENUK_GIT", "GERENUK_PYTEST", "GERENUK_FALLBACK"] {
            assert!(text.contains(var), "tune should name {var}");
        }
    }

    // --- Every command in the guides must be a real invocation ---
    //
    // The commands are fed through the real clap `Command` in `cli.rs`. What
    // is checked here is that the extraction those tests rely on finds them,
    // so an empty result can never pass as "all valid".

    #[test]
    fn extraction_finds_every_command_the_guides_show() {
        let setup = embedded_invocations(Topic::Setup);
        assert!(
            setup.contains(&vec!["gerenuk", "impacted-tests"]),
            "setup shows `gerenuk impacted-tests` in a bash fence, extraction returned {setup:?}",
        );
        assert!(
            setup.contains(&vec!["gerenuk", "guide", "triage"]),
            "setup points at the triage guide inline, extraction returned {setup:?}",
        );
        let triage = embedded_invocations(Topic::Triage);
        assert!(
            triage.contains(&vec!["gerenuk", "impacted-tests", "--format", "json"]),
            "the `> impact.json` redirect should end the command; got {triage:?}",
        );
        assert!(
            triage.contains(&vec!["gerenuk", "run", "--impact", "impact.json", "--", "-q"]),
            "the replay line should survive whole; got {triage:?}",
        );
        let tune = embedded_invocations(Topic::Tune);
        assert!(
            tune.contains(&vec!["gerenuk", "audit", "src/app.py"]),
            "tune shows an audit with a real file; got {tune:?}",
        );
        for topic in Topic::all() {
            let found = embedded_invocations(topic);
            assert!(!found.is_empty(), "guide `{}` shows no commands at all", topic.name());
            for argv in &found {
                assert_eq!(argv.first().copied(), Some("gerenuk"), "argv is {argv:?}");
            }
        }
    }

    #[test]
    fn extraction_skips_commands_that_are_not_gerenuk() {
        for topic in Topic::all() {
            for argv in embedded_invocations(topic) {
                assert!(
                    !argv.contains(&"uv") && !argv.contains(&"tyf"),
                    "`uv add` and `tyf daemon` are not ours to parse: {argv:?}",
                );
            }
        }
    }

    #[test]
    fn extraction_drops_placeholders_and_stops_at_redirects() {
        assert_eq!(
            gerenuk_invocations("gerenuk run --base <REF> | tee log > out.txt"),
            vec![vec!["gerenuk", "run", "--base"]],
        );
        assert!(gerenuk_invocations("uv run gerenuk doctor").is_empty(), "prefix tools skip");
    }

    // --- Every config key in the guides must exist ---

    /// The `[tool.gerenuk]` fences are what a reader copy-pastes, so every
    /// key in them is checked against the field list serde actually accepts.
    /// Other tools' tables (`[tool.madoqua]`) are not ours to check.
    #[test]
    fn every_gerenuk_config_key_in_a_toml_fence_exists() {
        let accepted = crate::config::keys::accepted_keys();
        let mut checked = 0_usize;
        for topic in Topic::all() {
            let mut table: Option<String> = None;
            for (line, fence) in lines_with_fence(topic.text()) {
                if fence != Some("toml") {
                    table = None;
                    continue;
                }
                let line = line.trim();
                if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                    table = Some(header.to_owned());
                    continue;
                }
                if table.as_deref() != Some("tool.gerenuk") {
                    continue;
                }
                let Some((key, _)) = line.split_once('=') else { continue };
                checked += 1;
                assert!(
                    accepted.contains(key.trim()),
                    "guide `{}` sets `{}` under `[tool.gerenuk]`, which the deserializer does \
                     not accept; accepted: {accepted:?}",
                    topic.name(),
                    key.trim(),
                );
            }
        }
        assert!(checked >= 4, "expected the guides to set several config keys, found {checked}");
    }

    /// The other direction, which is the one that catches drift: a key added
    /// to the schema that no guide mentions is a key nobody will find.
    #[test]
    fn every_config_key_is_named_by_a_guide() {
        let mentioned: BTreeSet<&str> =
            Topic::all().flat_map(|topic| inline_code_spans(topic.text())).collect();
        for key in crate::config::keys::accepted_keys() {
            assert!(
                mentioned.contains(key),
                "`{key}` is an accepted config key that no guide names; document it in \
                 `docs/src/guide/tune.md`, which is the reference",
            );
        }
    }

    // --- Detection ---

    #[test]
    fn empty_directory_is_not_configured() {
        let dir = tempdir();
        assert_eq!(detect(dir.path()), None);
        assert_eq!(auto_topic(detect(dir.path())), Topic::Setup);
    }

    #[test]
    fn a_madoqua_step_naming_gerenuk_is_configured() {
        let dir = tempdir();
        write(
            dir.path(),
            "pyproject.toml",
            "[project]\nname = \"x\"\n\n[tool.madoqua]\nextend_check = [\n  { name = \
             \"gerenuk\", cmd = \"gerenuk run -- -q\", pass_files = false },\n]\n",
        );
        assert_eq!(detect(dir.path()), Some(ConfigSource::MadoquaStep));
        assert_eq!(auto_topic(detect(dir.path())), Topic::Triage);
    }

    #[test]
    fn a_string_form_step_counts_too() {
        let dir = tempdir();
        write(dir.path(), "pyproject.toml", "[tool.madoqua]\ncheck = [\"gerenuk run\"]\n");
        assert_eq!(detect(dir.path()), Some(ConfigSource::MadoquaStep));
    }

    #[test]
    fn a_madoqua_table_without_gerenuk_is_not_configured() {
        let dir = tempdir();
        write(dir.path(), "pyproject.toml", "[tool.madoqua]\nextend_check = [\"bandit -q\"]\n");
        assert_eq!(detect(dir.path()), None);
    }

    #[test]
    fn a_tool_gerenuk_table_alone_is_not_the_hook() {
        // Budgets in `[tool.gerenuk]` tune a walk; they do not make one run
        // on commit. The reader of a repository with only that table still
        // needs to wire the step.
        let dir = tempdir();
        write(dir.path(), "pyproject.toml", "[tool.gerenuk]\nmax-depth = 20\n");
        assert_eq!(detect(dir.path()), None);
    }

    #[test]
    fn gerenuk_mentioned_in_a_comment_or_elsewhere_is_not_configured() {
        let dir = tempdir();
        write(
            dir.path(),
            "pyproject.toml",
            "# TODO: gerenuk step\n[project]\ndependencies = [\"gerenuk\"]\n[tool.madoqua]\n",
        );
        assert_eq!(detect(dir.path()), None, "detection parses the TOML rather than grepping");
    }

    #[test]
    fn unparseable_or_unreadable_pyproject_is_not_configured() {
        let dir = tempdir();
        write(dir.path(), "pyproject.toml", "[tool.madoqua\ncheck = \n");
        assert_eq!(detect(dir.path()), None, "a broken file is a reason to print setup");
        let dir = tempdir();
        std::fs::create_dir(dir.path().join("pyproject.toml")).expect("create dir");
        assert_eq!(detect(dir.path()), None);
    }

    #[test]
    fn detection_never_looks_at_ancestors() {
        let dir = tempdir();
        write(dir.path(), "pyproject.toml", "[tool.madoqua]\ncheck = [\"gerenuk run\"]\n");
        let sub = dir.path().join("pkg");
        std::fs::create_dir(&sub).expect("mkdir");
        assert_eq!(detect(&sub), None, "the guide tells its reader to stand at the root");
    }

    #[test]
    fn tune_is_never_auto_selected() {
        for source in [None, Some(ConfigSource::MadoquaStep)] {
            assert_ne!(auto_topic(source), Topic::Tune);
        }
    }

    // --- Rendering ---

    #[test]
    fn headers_name_the_topic_and_the_reason() {
        assert_eq!(header(Topic::Tune, Selection::Explicit), "# gerenuk guide: tune");
        assert_eq!(
            header(Topic::Setup, Selection::Auto(None)),
            "# gerenuk guide: not configured here -> setup"
        );
        assert_eq!(
            header(Topic::Triage, Selection::Auto(Some(ConfigSource::MadoquaStep))),
            "# gerenuk guide: configured via pyproject.toml [tool.madoqua] -> triage"
        );
    }

    #[test]
    fn render_is_the_header_then_the_docs_page_verbatim() {
        let rendered = render(Topic::Triage, Selection::Explicit);
        assert_eq!(rendered, format!("# gerenuk guide: triage\n\n{}", Topic::Triage.text()));
    }
}
