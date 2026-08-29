//! Wiring the pure closure in [`crate::closure`] to `tyf` and the working tree.
//!
//! [`TyfRefs`] answers "who references this?" out of `tyf refs`; [`FsIndex`]
//! answers "what owns this line?" out of the checked-out files. Neither holds
//! any logic worth testing on its own — the rules live in [`crate::closure`],
//! which is why they can be this thin.

use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::changed::ChangedSymbols;
use crate::closure::{
    classify_in, indexed, seeds_from, walk, IgnoredSymbol, ImpactedTest, Index, IndexedSymbol,
    Limits, Reason, RefAnswer, RefSite, Refs, Site, Stats, SymbolQuery, Verdict, DEFAULT_BUDGET_MS,
    DEFAULT_MAX_DEPTH, DEFAULT_MAX_SYMBOLS,
};
use crate::config::Config;
use crate::modpath::module_path;
use crate::pysource::{self, imports_module, word_lines};
use crate::report::{list_section, Format};
use crate::tyf::Runner;
use crate::workspace::is_test_path;

/// The whole answer for one `gerenuk impacted-tests` run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactReport {
    pub verdict: Verdict,
    /// Why the verdict is `run_all`; `null` when it is `selected`.
    pub reason: Option<Reason>,
    /// The ref the diff was taken against, e.g. `origin/main`.
    pub base: String,
    pub merge_base: String,
    pub impacted_tests: Vec<ImpactedTest>,
    /// Changed test files, passed through from phase 1: a changed test selects
    /// itself, with no walking needed.
    pub test_files_changed: Vec<String>,
    pub ignored_symbols: Vec<IgnoredSymbol>,
    pub stats: Stats,
    pub errors: Vec<String>,
}

/// Budget overrides taken from the command line, each `None` when not given.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Budgets {
    pub max_depth: Option<u32>,
    pub max_symbols: Option<usize>,
    pub budget_ms: Option<u64>,
}

/// Resolve limits: a command-line flag beats `pyproject.toml`, which beats the
/// built-in default. A budget of `0` disables the wall clock entirely.
#[must_use]
pub(crate) fn resolve_limits(flags: Budgets, config: &Config, started: Instant) -> Limits {
    let budget_ms = flags.budget_ms.or(config.budget_ms).unwrap_or(DEFAULT_BUDGET_MS);
    Limits {
        max_depth: flags.max_depth.or(config.max_depth).unwrap_or(DEFAULT_MAX_DEPTH),
        max_symbols: flags.max_symbols.or(config.max_symbols).unwrap_or(DEFAULT_MAX_SYMBOLS),
        deadline: (budget_ms > 0).then(|| started + Duration::from_millis(budget_ms)),
    }
}

/// The verdict that can be reached without asking `tyf` anything.
///
/// Both cases mean the same thing: part of the diff is beyond what symbol
/// analysis can see, so no selection derived from the rest is trustworthy.
/// Checking them first is also what lets a diff that touches only a
/// `pyproject.toml` answer with no `tyf` installed at all.
#[must_use]
pub(crate) fn upfront_reason(changed: &ChangedSymbols) -> Option<Reason> {
    if !changed.non_python_changes.is_empty() {
        return Some(Reason::NonPythonChanges);
    }
    if !changed.errors.is_empty() {
        return Some(Reason::ParseErrors);
    }
    None
}

/// Walk from a phase-1 report to the tests it impacts.
pub fn analyze(
    changed: &ChangedSymbols,
    refs: &impl Refs,
    index: &impl Index,
    config: &Config,
    limits: &Limits,
) -> ImpactReport {
    let seeds = match seeds_from(changed, index) {
        Ok(seeds) => seeds,
        Err(err) => {
            return run_all(changed, Reason::IndexFailed, vec![format!("{err:#}")]);
        }
    };

    let closure = walk(&seeds, refs, index, config, limits);
    ImpactReport {
        verdict: closure.verdict,
        reason: closure.reason,
        base: changed.base.clone(),
        merge_base: changed.merge_base.clone(),
        impacted_tests: closure.impacted,
        test_files_changed: changed.test_files_changed.clone(),
        ignored_symbols: closure.ignored,
        stats: closure.stats,
        errors: closure.errors,
    }
}

/// A verdict reached without walking: the changed test files are still worth
/// reporting, and the caller still exits `0`.
#[must_use]
pub(crate) fn run_all(
    changed: &ChangedSymbols,
    reason: Reason,
    errors: Vec<String>,
) -> ImpactReport {
    ImpactReport {
        verdict: Verdict::RunAll,
        reason: Some(reason),
        base: changed.base.clone(),
        merge_base: changed.merge_base.clone(),
        impacted_tests: Vec::new(),
        test_files_changed: changed.test_files_changed.clone(),
        ignored_symbols: Vec::new(),
        stats: Stats::default(),
        errors,
    }
}

impl ImpactReport {
    /// The report as JSON, or as the short human summary.
    pub fn render(&self, format: Format) -> Result<String> {
        match format {
            Format::Human => Ok(self.render_human()),
            Format::Json => Ok(serde_json::to_string_pretty(self)?),
        }
    }

    /// A short summary. JSON is the contract; this is for eyeballs.
    fn render_human(&self) -> String {
        use std::fmt::Write as _;

        let mut out = String::new();
        let short = self.merge_base.get(..7).unwrap_or(&self.merge_base);
        let _ = writeln!(out, "base {} (merge-base {short})", self.base);

        match self.reason {
            Some(reason) => {
                let _ = writeln!(out, "verdict run_all — {}", reason.label());
            }
            None => {
                let _ = writeln!(out, "verdict selected");
            }
        }

        self.write_tests(&mut out);
        list_section(&mut out, "changed test files", &self.test_files_changed);

        if !self.ignored_symbols.is_empty() {
            let _ = writeln!(out, "\nignored symbols ({})", self.ignored_symbols.len());
            for entry in &self.ignored_symbols {
                let _ = writeln!(out, "  {}  (@{})", entry.symbol, entry.ignored_by);
            }
        }

        list_section(&mut out, "errors", &self.errors);

        let _ = writeln!(
            out,
            "\n{} symbol(s) visited, {} tyf call(s), {} ms",
            self.stats.visited, self.stats.tyf_calls, self.stats.duration_ms
        );
        out
    }

    /// Each impacted test, with the chain that reached it underneath.
    fn write_tests(&self, out: &mut String) {
        use std::fmt::Write as _;

        if self.impacted_tests.is_empty() {
            let _ = writeln!(out, "\nNo impacted tests.");
            return;
        }

        let _ = writeln!(out, "\nimpacted tests ({})", self.impacted_tests.len());
        for test in &self.impacted_tests {
            match &test.symbol {
                Some(symbol) => {
                    let name = symbol.rsplit(':').next().unwrap_or(symbol);
                    let _ = writeln!(out, "  {}::{name}", test.file);
                }
                None => {
                    let _ = writeln!(out, "  {}  (whole file)", test.file);
                }
            }
            // The chain the JSON splits across `via` and `origin`, joined back
            // up for reading: nearest the test first.
            let chain: Vec<&str> =
                test.via.iter().map(String::as_str).chain([test.origin.as_str()]).collect();
            let _ = writeln!(out, "    ← {}", chain.join(" ← "));
        }
    }
}

/// [`Refs`] over the `tyf` binary: one `tyf refs` call per BFS frontier.
///
/// Symbols are addressed by `file:line:col` position rather than by name.
/// `tyf refs` rejects a name with more than one dot, so `Outer.Inner.method`
/// has no name form at all — and a position also tells two same-named symbols
/// in different modules apart.
/// See `docs/adr/0002-refs-queries-by-position.md`.
pub struct TyfRefs<'a> {
    runner: &'a Runner,
    root: &'a Path,
}

impl<'a> TyfRefs<'a> {
    #[must_use]
    pub const fn new(runner: &'a Runner, root: &'a Path) -> Self {
        Self { runner, root }
    }
}

impl Refs for TyfRefs<'_> {
    fn refs(&self, queries: &[SymbolQuery]) -> Result<Vec<RefAnswer>> {
        let positions: Vec<String> = queries.iter().map(position).collect();
        let found = self.runner.refs_batch(&positions).with_context(|| {
            format!("could not resolve references for {}", positions.join(", "))
        })?;

        Ok(queries
            .iter()
            .zip(found)
            .map(|(query, result)| {
                // The production/test split is re-derived downstream from paths
                // relative to the root, so both of tyf's buckets go in together.
                let sites = result
                    .references
                    .iter()
                    .chain(&result.test_references)
                    .map(|reference| RefSite {
                        file: relative_to(&reference.file, self.root),
                        line: reference.line,
                    })
                    .collect();
                RefAnswer { id: query.id.clone(), sites }
            })
            .collect())
    }
}

/// The `file:line:col` form `tyf refs` auto-detects as a position.
fn position(query: &SymbolQuery) -> String {
    format!("{}:{}:{}", query.file.display(), query.line, query.column)
}

/// `tyf` reports repository-relative paths, but not reliably so; an absolute
/// one is made relative rather than being treated as a different file.
fn relative_to(path: &Path, root: &Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

/// One working-tree file, read once and parsed at most once.
///
/// The tree is deferred because the two scans that touch *every* file —
/// [`Index::word_hits`] and [`Index::test_importers`] — are purely textual. A
/// single deleted symbol would otherwise parse the whole repository.
///
/// [`Self::module`] stays `None` when the file did not parse — but `source` is
/// still there, and that matters: a file with syntax the vendored grammar does
/// not know must not vanish from the textual scans. It would take its tests
/// with it, and the verdict would still say `selected`.
struct Cached {
    source: String,
    module: OnceCell<Option<pysource::Module>>,
}

impl Cached {
    /// The file's symbol table, parsed on first use.
    fn module(&self) -> Option<&pysource::Module> {
        self.module
            .get_or_init(|| pysource::parse(&self.source).ok().filter(|module| !module.has_error))
            .as_ref()
    }
}

/// [`Index`] over the checked-out working tree.
///
/// Files are read once and cached: references cluster heavily in the same
/// handful of modules, so re-reading per site would dominate the walk.
pub struct FsIndex<'a> {
    root: &'a Path,
    /// Repository-relative `.py` paths, from `git`.
    /// See `docs/adr/0006-file-list-from-git.md`.
    files: Vec<PathBuf>,
    cache: RefCell<HashMap<PathBuf, Option<Rc<Cached>>>>,
}

impl<'a> FsIndex<'a> {
    /// `files` is every path in the repository; non-Python ones are dropped.
    #[must_use]
    pub fn new(root: &'a Path, files: &[PathBuf]) -> Self {
        let files = files
            .iter()
            .filter(|path| path.extension().is_some_and(|ext| ext == "py"))
            .cloned()
            .collect();
        Self { root, files, cache: RefCell::new(HashMap::new()) }
    }

    /// The file's text and, if it parsed, its symbol table.
    ///
    /// `None` means it could not be read at all. A miss is cached too: a walk
    /// that meets the same vanished file forty times should stat it once.
    fn load(&self, file: &Path) -> Result<Option<Rc<Cached>>> {
        if let Some(cached) = self.cache.borrow().get(file) {
            return Ok(cached.as_ref().map(Rc::clone));
        }
        let cached = self.read(file)?.map(Rc::new);
        self.cache.borrow_mut().insert(file.to_path_buf(), cached.clone());
        Ok(cached)
    }

    /// A file that is simply gone is not an error — `git ls-files` lists paths
    /// the working tree may have deleted. Anything else is, and surfacing it is
    /// what turns a silently short answer into a `run_all`.
    fn read(&self, file: &Path) -> Result<Option<Cached>> {
        let path = self.root.join(file);
        let source = match std::fs::read_to_string(&path) {
            Ok(source) => source,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(err).with_context(|| format!("could not read {}", path.display()))
            }
        };
        Ok(Some(Cached { source, module: OnceCell::new() }))
    }
}

impl Index for FsIndex<'_> {
    fn classify(&self, file: &Path, line: u32) -> Result<Site> {
        let Some(cached) = self.load(file)? else { return Ok(Site::Unknown) };
        let Some(module) = cached.module() else { return Ok(Site::Unknown) };
        Ok(classify_in(module, line))
    }

    fn module_path(&self, file: &Path) -> Option<String> {
        module_path(self.root, file)
    }

    fn is_test(&self, file: &Path) -> bool {
        is_test_path(file, self.root)
    }

    fn top_level(&self, file: &Path) -> Result<Vec<IndexedSymbol>> {
        let Some(cached) = self.load(file)? else { return Ok(Vec::new()) };
        let Some(module) = cached.module() else { return Ok(Vec::new()) };
        Ok(module.top_level().map(indexed).collect())
    }

    fn word_hits(&self, name: &str) -> Result<Vec<(PathBuf, u32)>> {
        let mut hits = Vec::new();
        for file in &self.files {
            // Textual, and deliberately so: no tree is built for these files.
            let Some(cached) = self.load(file)? else { continue };
            hits.extend(
                word_lines(&cached.source, name).into_iter().map(|line| (file.clone(), line)),
            );
        }
        Ok(hits)
    }

    fn test_importers(&self, module: &str) -> Result<Vec<PathBuf>> {
        let mut importers = Vec::new();
        for file in self.files.iter().filter(|file| self.is_test(file)) {
            let Some(cached) = self.load(file)? else { continue };
            if imports_module(&cached.source, module) {
                importers.push(file.clone());
            }
        }
        Ok(importers)
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::closure::IgnoredSymbol;

    fn changed() -> ChangedSymbols {
        ChangedSymbols {
            base: "origin/main".to_string(),
            merge_base: "abc1234def".to_string(),
            ..ChangedSymbols::default()
        }
    }

    #[test]
    fn a_non_python_change_is_decided_before_tyf_is_needed() {
        let mut report = changed();
        report.non_python_changes = vec!["pyproject.toml".to_string()];
        assert_eq!(
            upfront_reason(&report),
            Some(Reason::NonPythonChanges),
            "a dependency bump can change any test's behaviour"
        );
    }

    #[test]
    fn a_phase_one_parse_error_is_decided_up_front_too() {
        let mut report = changed();
        report.errors = vec!["src/pkg/broken.py".to_string()];
        assert_eq!(upfront_reason(&report), Some(Reason::ParseErrors));
    }

    #[test]
    fn a_clean_python_only_diff_has_no_up_front_verdict() {
        assert_eq!(upfront_reason(&changed()), None, "nothing to stop the walk");
    }

    #[test]
    fn a_flag_beats_the_config_which_beats_the_default() {
        let config = Config { max_depth: Some(4), max_symbols: Some(50), ..Config::default() };
        let flags = Budgets { max_depth: Some(2), ..Budgets::default() };

        let limits = resolve_limits(flags, &config, Instant::now());
        assert_eq!(limits.max_depth, 2, "the flag wins");
        assert_eq!(limits.max_symbols, 50, "the config wins where there is no flag");
        assert!(limits.deadline.is_some(), "and the default budget still applies");
    }

    #[test]
    fn unset_everywhere_means_the_built_in_defaults() {
        let limits = resolve_limits(Budgets::default(), &Config::default(), Instant::now());
        assert_eq!(limits.max_depth, DEFAULT_MAX_DEPTH);
        assert_eq!(limits.max_symbols, DEFAULT_MAX_SYMBOLS);
    }

    #[test]
    fn a_zero_budget_disables_the_wall_clock() {
        let flags = Budgets { budget_ms: Some(0), ..Budgets::default() };
        let limits = resolve_limits(flags, &Config::default(), Instant::now());
        assert_eq!(limits.deadline, None, "0 means `do not time me out`, not `stop now`");
    }

    #[test]
    fn a_run_all_report_still_carries_the_changed_test_files() {
        let mut report = changed();
        report.test_files_changed = vec!["tests/test_x.py".to_string()];
        let out = run_all(&report, Reason::NonPythonChanges, vec!["boom".to_string()]);

        assert_eq!(out.verdict, Verdict::RunAll);
        assert_eq!(out.test_files_changed, vec!["tests/test_x.py".to_string()], "still useful");
        assert_eq!(out.errors, vec!["boom".to_string()]);
        assert_eq!(out.stats.visited, 0, "nothing was walked");
    }

    fn report_with(tests: Vec<ImpactedTest>) -> ImpactReport {
        ImpactReport {
            verdict: Verdict::Selected,
            reason: None,
            base: "main".to_string(),
            merge_base: "5ddda1ffff".to_string(),
            impacted_tests: tests,
            test_files_changed: vec![],
            ignored_symbols: vec![],
            stats: Stats { visited: 3, tyf_calls: 2, ..Stats::default() },
            errors: vec![],
        }
    }

    #[test]
    fn human_output_joins_the_why_chain_back_together() {
        let report = report_with(vec![ImpactedTest {
            file: "tests/test_enrich.py".to_string(),
            symbol: Some("tests.test_enrich:test_run".to_string()),
            via: vec!["mypkg.api:endpoint".to_string()],
            origin: "mypkg.enrich:Enricher.run".to_string(),
        }]);

        let text = report.render(Format::Human).expect("human rendering cannot fail");
        assert!(
            text.contains("tests/test_enrich.py::test_run"),
            "the node id shape should be readable, got:\n{text}"
        );
        assert!(
            text.contains("← mypkg.api:endpoint ← mypkg.enrich:Enricher.run"),
            "the chain runs from the test to the change, got:\n{text}"
        );
        assert!(text.contains("verdict selected"), "the verdict is the headline, got:\n{text}");
    }

    #[test]
    fn a_whole_file_selection_says_so() {
        let report = report_with(vec![ImpactedTest {
            file: "tests/test_settings.py".to_string(),
            symbol: None,
            via: vec![],
            origin: "mypkg.settings".to_string(),
        }]);
        let text = report.render(Format::Human).expect("human rendering cannot fail");
        assert!(
            text.contains("tests/test_settings.py  (whole file)"),
            "a null symbol must not render as an empty node id, got:\n{text}"
        );
    }

    #[test]
    fn a_run_all_verdict_names_its_reason_in_human_output() {
        let mut report = report_with(vec![]);
        report.verdict = Verdict::RunAll;
        report.reason = Some(Reason::MaxDepth);

        let text = report.render(Format::Human).expect("human rendering cannot fail");
        assert!(
            text.contains("run_all"),
            "the human spelling must match the token the docs and JSON use, got:\n{text}"
        );
        assert!(text.contains("depth limit"), "the reason must be legible, got:\n{text}");
        assert!(text.contains("No impacted tests."), "and an empty list says so, got:\n{text}");
    }

    #[test]
    fn json_output_uses_null_for_a_missing_reason_and_symbol() {
        let report = report_with(vec![ImpactedTest {
            file: "tests/test_a.py".to_string(),
            symbol: None,
            via: vec![],
            origin: "mypkg.a".to_string(),
        }]);
        let text = report.render(Format::Json).expect("JSON rendering cannot fail");
        let value: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");

        assert_eq!(value["verdict"], "selected", "the verdict serialises snake_case");
        assert!(value["reason"].is_null(), "a clean run has no reason");
        assert!(
            value["impacted_tests"][0]["symbol"].is_null(),
            "a whole-file selection is a null symbol, not an empty string"
        );
    }

    #[test]
    fn an_ignored_symbol_is_rendered_with_the_decorator_that_matched() {
        let mut report = report_with(vec![]);
        report.ignored_symbols = vec![IgnoredSymbol {
            symbol: "mypkg.jobs:nightly".to_string(),
            file: "src/mypkg/jobs.py".to_string(),
            ignored_by: "transformation".to_string(),
        }];
        let text = report.render(Format::Human).expect("human rendering cannot fail");
        assert!(text.contains("mypkg.jobs:nightly  (@transformation)"), "got:\n{text}");
    }

    /// A working tree on disk, for the [`FsIndex`] tests.
    fn tree(files: &[(&str, &str)]) -> (TempDir, Vec<PathBuf>) {
        let tmp = TempDir::new().expect("temp dir");
        let mut paths = Vec::new();
        for (rel, body) in files {
            let path = tmp.path().join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create parent");
            }
            std::fs::write(&path, body).expect("write file");
            paths.push(PathBuf::from(rel));
        }
        (tmp, paths)
    }

    const SETTINGS: &str = "\
import os

VALUE = os.environ.get(\"X\")


def read():
    return VALUE
";

    #[test]
    fn the_index_classifies_imports_symbols_and_module_level_lines() {
        let (tmp, files) = tree(&[("pkg/settings.py", SETTINGS)]);
        let index = FsIndex::new(tmp.path(), &files);
        let file = Path::new("pkg/settings.py");

        assert_eq!(index.classify(file, 1).expect("readable"), Site::Import, "`import os`");
        assert_eq!(index.classify(file, 3).expect("readable"), Site::ModuleLevel, "a constant");
        match index.classify(file, 7).expect("readable") {
            Site::Symbol(found) => {
                assert_eq!(found.qualname, "read");
                assert_eq!(found.line, 6, "the definition's name line, for the next query");
            }
            other => panic!("expected the enclosing function, got {other:?}"),
        }
    }

    #[test]
    fn an_unreadable_file_classifies_as_unknown_rather_than_failing() {
        let (tmp, files) = tree(&[("pkg/settings.py", SETTINGS)]);
        let index = FsIndex::new(tmp.path(), &files);
        assert_eq!(
            index.classify(Path::new("pkg/gone.py"), 1).expect("a missing file is not an error"),
            Site::Unknown,
            "the walk records it and carries on"
        );
    }

    #[test]
    fn a_file_that_does_not_parse_is_unknown_too() {
        let (tmp, files) = tree(&[("pkg/broken.py", "def f(:\n    pass\n")]);
        let index = FsIndex::new(tmp.path(), &files);
        assert_eq!(
            index.classify(Path::new("pkg/broken.py"), 2).expect("no error"),
            Site::Unknown,
            "a partial tree cannot say which symbol owns a line"
        );
    }

    #[test]
    fn the_index_only_ever_scans_python_files() {
        let (tmp, files) =
            tree(&[("pkg/a.py", "def target():\n    pass\n"), ("README.md", "target\n")]);
        let index = FsIndex::new(tmp.path(), &files);

        let hits = index.word_hits("target").expect("scan runs");
        assert_eq!(hits, vec![(PathBuf::from("pkg/a.py"), 1)], "the Markdown file is not source");
    }

    #[test]
    fn test_importers_finds_only_test_files_that_import_the_module() {
        let (tmp, files) = tree(&[
            ("pkg/settings.py", SETTINGS),
            ("tests/test_settings.py", "from pkg import settings\n\n\ndef test_x():\n    pass\n"),
            ("tests/test_other.py", "def test_y():\n    pass\n"),
            ("pkg/consumer.py", "from pkg import settings\n"),
        ]);
        let index = FsIndex::new(tmp.path(), &files);

        assert_eq!(
            index.test_importers("pkg.settings").expect("scan runs"),
            vec![PathBuf::from("tests/test_settings.py")],
            "a non-test importer is walked through the graph, not selected"
        );
    }

    #[test]
    fn the_index_resolves_module_paths_and_test_paths() {
        let (tmp, files) = tree(&[("src/mypkg/__init__.py", ""), ("src/mypkg/a.py", "x = 1\n")]);
        let index = FsIndex::new(tmp.path(), &files);

        assert_eq!(
            index.module_path(Path::new("src/mypkg/a.py")).as_deref(),
            Some("mypkg.a"),
            "src-layout resolves through the __init__ chain"
        );
        assert!(index.is_test(Path::new("tests/test_a.py")), "and the test heuristic is shared");
    }

    #[test]
    fn top_level_skips_methods() {
        let (tmp, files) = tree(&[(
            "pkg/a.py",
            "def f():\n    pass\n\n\nclass C:\n    def m(self):\n        pass\n",
        )]);
        let index = FsIndex::new(tmp.path(), &files);

        let names: Vec<String> = index
            .top_level(Path::new("pkg/a.py"))
            .expect("readable")
            .into_iter()
            .map(|s| s.qualname)
            .collect();
        assert_eq!(names, vec!["f".to_string(), "C".to_string()], "methods come with their class");
    }

    #[test]
    fn a_file_that_does_not_parse_is_still_scanned_textually() {
        // The regression that matters: dropping unparseable files from the
        // scans omits their tests while the verdict still says `selected`.
        let (tmp, files) = tree(&[
            ("pkg/broken.py", "from pkg import settings\n\ndef f(:\n    pass\n"),
            ("tests/test_broken.py", "from pkg import settings\n\ndef test_x(:\n    pass\n"),
        ]);
        let index = FsIndex::new(tmp.path(), &files);

        assert_eq!(
            index.test_importers("pkg.settings").expect("scan runs"),
            vec![PathBuf::from("tests/test_broken.py")],
            "an importer that does not parse is still an importer"
        );
        assert_eq!(
            index.word_hits("settings").expect("scan runs").len(),
            2,
            "and its text is still searchable for a deleted name"
        );
    }

    #[test]
    fn a_file_git_lists_but_the_tree_no_longer_has_is_not_an_error() {
        let (tmp, mut files) = tree(&[("pkg/a.py", "def target():\n    pass\n")]);
        files.push(PathBuf::from("pkg/deleted.py"));
        let index = FsIndex::new(tmp.path(), &files);

        assert_eq!(
            index.word_hits("target").expect("a deleted path is skipped, not fatal"),
            vec![(PathBuf::from("pkg/a.py"), 1)],
            "`git ls-files` lists paths the working tree may have removed"
        );
    }

    #[test]
    fn a_query_renders_as_a_file_line_column_position() {
        let query = SymbolQuery {
            id: "mypkg.a:Outer.Inner.method".to_string(),
            name: "Outer.Inner.method".to_string(),
            file: PathBuf::from("src/mypkg/a.py"),
            line: 15,
            column: 9,
        };
        assert_eq!(
            position(&query),
            "src/mypkg/a.py:15:9",
            "a nested name has no `tyf refs` name form, so the position is the only query"
        );
    }

    #[test]
    fn tyf_paths_are_made_relative_to_the_root() {
        assert_eq!(
            relative_to(Path::new("/repo/pkg/a.py"), Path::new("/repo")),
            PathBuf::from("pkg/a.py"),
            "an absolute answer must not read as a different file"
        );
        assert_eq!(
            relative_to(Path::new("pkg/a.py"), Path::new("/repo")),
            PathBuf::from("pkg/a.py"),
            "an already-relative answer is left alone"
        );
    }
}
