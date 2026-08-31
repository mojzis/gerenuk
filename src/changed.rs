//! Turning a diff into the set of Python symbols it changed.
//!
//! This module is pure with respect to the outside world: it reads files and
//! blobs through the [`Sources`] trait, so the whole classification is
//! unit-testable from a `HashMap` — no git repository, no `tyf`, no Python.
//! [`GitSources`] is the one implementation that touches anything real.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::diff::{self, FileDiff, LineRange};
use crate::git::{Base, Git};
use crate::modpath::{module_path, symbol_id};
use crate::pysource::{self, Kind, SymbolSpan};
use crate::report::{list_section, Format};
use crate::workspace::is_test_path;

/// What happened to a symbol between the merge base and the working tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Change {
    Added,
    Modified,
    Deleted,
}

/// One symbol the diff touched.
///
/// `ignored_by` is only ever set on entries in
/// [`ChangedSymbols::ignored_symbols`], and is omitted from JSON otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolChange {
    /// `module.path:QualName`, e.g. `mypkg.pipelines.enrich:Enricher.run`.
    pub symbol: String,
    /// Repository-relative path.
    pub file: String,
    pub kind: Kind,
    /// Line of the definition's name, 1-based — where a reference to it lands.
    pub line: u32,
    /// Column of the definition's name, 1-based.
    ///
    /// Together with `line` this is the `file:line:col` position phase 2 sends
    /// to `tyf refs`. It is deliberately not `#[serde(default)]`: a replayed
    /// report missing it must fail loudly, because a wrong column comes back as
    /// *no references* rather than as an error.
    pub column: u32,
    pub change: Change,
    /// The `ignore-decorators` entry that matched, when the symbol was ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignored_by: Option<String>,
}

/// A module whose top level changed: imports, constants, module-level code.
///
/// The file travels with the module name because phase 2 has to outline the
/// module to seed its symbols, and a dotted name is not a path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleChange {
    /// Dotted module path, e.g. `mypkg.pipelines.enrich`.
    pub module: String,
    /// Repository-relative path.
    pub file: String,
}

/// The whole answer for one `gerenuk changed-symbols` run.
///
/// Deserialising is what `impacted-tests --changed report.json` replays, so the
/// rules are stricter than they look. `base` and `merge_base` are **required**
/// and unknown keys are **rejected**: a truncated file, a typo, or a report
/// from a different gerenuk would otherwise deserialise into an empty report,
/// and an empty report is indistinguishable from "nothing changed" — a
/// confident `selected` verdict that runs no tests at all. Failing loudly with
/// exit 2 is the only safe reading of a report we cannot trust.
///
/// The arrays default individually, so a hand-written report may still omit the
/// ones that are empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(
    clippy::struct_field_names,
    reason = "the field names are the published JSON schema; they cannot be shortened"
)]
pub struct ChangedSymbols {
    /// The ref the diff was taken against, e.g. `origin/main`.
    pub base: String,
    pub merge_base: String,
    #[serde(default)]
    pub changed_symbols: Vec<SymbolChange>,
    /// Symbols suppressed by `ignore-decorators`.
    #[serde(default)]
    pub ignored_symbols: Vec<SymbolChange>,
    /// Modules whose top level changed — imports, constants, module-level code.
    #[serde(default)]
    pub module_level_changes: Vec<ModuleChange>,
    #[serde(default)]
    pub non_python_changes: Vec<String>,
    #[serde(default)]
    pub test_files_changed: Vec<String>,
    /// Files that could not be parsed; each is also a module-level change.
    #[serde(default)]
    pub errors: Vec<String>,
}

/// Which lines of a file to attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sides {
    /// Only the lines the diff hunks named.
    Hunks { old: Vec<LineRange>, new: Vec<LineRange> },
    /// Every line of whichever sides exist.
    ///
    /// Used for renames — where git's similarity detection reports only the
    /// residual hunks, but every caller of the old module has to be revisited —
    /// and for untracked files, which no diff mentions at all.
    Whole,
}

/// One file's change, normalised so the analysis does not care where it came
/// from (a diff hunk, a rename, or `git ls-files --others`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub old_path: Option<PathBuf>,
    pub new_path: Option<PathBuf>,
    pub sides: Sides,
    pub binary: bool,
}

impl FileChange {
    /// The path the file is reported under: where it is now, or where it was.
    #[must_use]
    pub fn reported_path(&self) -> Option<&Path> {
        self.new_path.as_deref().or(self.old_path.as_deref())
    }

    /// True when the file moved, which is what forces [`Sides::Whole`].
    #[must_use]
    pub fn is_rename(&self) -> bool {
        matches!((&self.old_path, &self.new_path), (Some(old), Some(new)) if old != new)
    }
}

impl From<FileDiff> for FileChange {
    fn from(file: FileDiff) -> Self {
        let mut change = Self {
            old_path: file.old_path,
            new_path: file.new_path,
            sides: Sides::Hunks { old: file.old_ranges, new: file.new_ranges },
            binary: file.binary,
        };
        if change.is_rename() {
            change.sides = Sides::Whole;
        }
        change
    }
}

/// An untracked file: entirely new, and invisible to `git diff`.
#[must_use]
pub fn untracked_change(path: PathBuf) -> FileChange {
    FileChange { old_path: None, new_path: Some(path), sides: Sides::Whole, binary: false }
}

/// Where the analysis gets file contents and module names.
pub trait Sources {
    /// Contents at the merge base, or `None` if the path did not exist there.
    fn old_source(&self, path: &Path) -> Result<Option<String>>;
    /// Contents in the working tree, or `None` if the file is not there.
    fn current_source(&self, path: &Path) -> Result<Option<String>>;
    /// Dotted module path for a repository-relative `.py` path.
    fn module_path(&self, path: &Path) -> Option<String>;
}

/// The real [`Sources`]: git for the old side, the filesystem for the new one.
pub struct GitSources<'a> {
    git: &'a Git,
    root: &'a Path,
    merge_base: &'a str,
}

impl<'a> GitSources<'a> {
    #[must_use]
    pub const fn new(git: &'a Git, root: &'a Path, merge_base: &'a str) -> Self {
        Self { git, root, merge_base }
    }
}

impl Sources for GitSources<'_> {
    fn old_source(&self, path: &Path) -> Result<Option<String>> {
        self.git
            .show(self.merge_base, path)
            .with_context(|| format!("could not read {} at the merge base", path.display()))
    }

    fn current_source(&self, path: &Path) -> Result<Option<String>> {
        match std::fs::read_to_string(self.root.join(path)) {
            Ok(text) => Ok(Some(text)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err).with_context(|| format!("could not read {}", path.display())),
        }
    }

    fn module_path(&self, path: &Path) -> Option<String> {
        module_path(self.root, path)
    }
}

/// Classify every file in `changes`.
///
/// `root` is the repository root, used only to decide which paths are tests.
pub fn analyze(
    base: &Base,
    changes: &[FileChange],
    root: &Path,
    sources: &impl Sources,
    config: &Config,
) -> Result<ChangedSymbols> {
    let mut acc = Accumulator::default();

    for change in changes {
        let Some(reported) = change.reported_path() else { continue };

        if change.binary || !is_python(reported) {
            acc.non_python.insert(display(reported));
            continue;
        }
        if is_test_path(reported, root) {
            acc.test_files.insert(display(reported));
            continue;
        }

        analyze_file(change, reported, sources, config, &mut acc)?;
    }

    Ok(acc.finish(base))
}

/// Symbol analysis for one Python file that is not a test.
fn analyze_file(
    change: &FileChange,
    reported: &Path,
    sources: &impl Sources,
    config: &Config,
    acc: &mut Accumulator,
) -> Result<()> {
    let old = load_side(change.old_path.as_deref(), sources, Side::Old)?;
    let new = load_side(change.new_path.as_deref(), sources, Side::New)?;

    // A tree we could not parse tells us nothing trustworthy about which symbol
    // owns which line, so the whole file degrades to a module-level change.
    if old.as_ref().is_some_and(|s| s.parsed.has_error)
        || new.as_ref().is_some_and(|s| s.parsed.has_error)
    {
        acc.errors.insert(display(reported));
        for side in [&old, &new].into_iter().flatten() {
            acc.module_level.insert(side.module_level_change());
        }
        return Ok(());
    }

    let (old_lines, new_lines) = touched(change, old.as_ref(), new.as_ref());

    // Keyed by the full `module:QualName` symbol id, not by qualname alone: a
    // rename moves the module, so `pkg.old:f` and `pkg.new:f` are two symbols
    // even though the definition is byte-identical.
    let mut touched_symbols: BTreeMap<String, (&SymbolSpan, &LoadedSide)> = BTreeMap::new();
    for (side, lines) in [(old.as_ref(), &old_lines), (new.as_ref(), &new_lines)] {
        let Some(side) = side else { continue };
        for &line in lines {
            match side.parsed.symbol_at(line) {
                Some(span) => {
                    // New-side entries overwrite old-side ones, so the report
                    // describes the definition as it stands now.
                    touched_symbols.insert(symbol_id(&side.module, &span.qualname), (span, side));
                }
                None if side.is_blank(line) => {}
                None => {
                    acc.module_level.insert(side.module_level_change());
                }
            }
        }
    }

    for (symbol, (span, side)) in touched_symbols {
        let module = side.module.as_str();
        let in_old = has_symbol(old.as_ref(), module, &span.qualname);
        let in_new = has_symbol(new.as_ref(), module, &span.qualname);

        // Classification is by *existence* on each side, not by which side the
        // hunk touched: deleting lines from a function that still exists is a
        // modification, not a deletion.
        let change_kind = match (in_old, in_new) {
            (true, true) => Change::Modified,
            (true, false) => Change::Deleted,
            (false, _) => Change::Added,
        };

        // Describe the surviving definition where there is one, so a decorator
        // added in this very change is taken into account.
        let (span, side) = surviving(new.as_ref(), module, &span.qualname).unwrap_or((span, side));

        let ignored_by = span
            .decorator_names()
            .find_map(|decorator| config.matching_decorator(decorator))
            .map(ToString::to_string);

        let entry = SymbolChange {
            symbol,
            file: display(&side.path),
            kind: span.kind,
            line: span.name_line,
            column: span.name_column,
            change: change_kind,
            ignored_by,
        };

        if entry.ignored_by.is_some() {
            acc.ignored.push(entry);
        } else {
            acc.changed.push(entry);
        }
    }

    Ok(())
}

/// Which side of the diff a loaded source belongs to.
#[derive(Debug, Clone, Copy)]
enum Side {
    Old,
    New,
}

/// A parsed side of one file, with the names it is reported under.
struct LoadedSide {
    path: PathBuf,
    module: String,
    parsed: pysource::Module,
    /// Whether each line (1-based) is blank, for [`Self::is_blank`].
    blank: Vec<bool>,
}

impl LoadedSide {
    fn line_count(&self) -> u32 {
        u32::try_from(self.blank.len()).unwrap_or(u32::MAX)
    }

    /// The `(module, file)` pair this side is reported under.
    fn module_level_change(&self) -> (String, String) {
        (self.module.clone(), display(&self.path))
    }

    /// Whether the line is whitespace-only.
    ///
    /// Blank lines carry no meaning, but adding a function inserts two of them
    /// before the `def`. Attributing those to the module would report a
    /// module-level change — and make phase 2 select the module's whole test
    /// surface — every time anyone appends a function.
    fn is_blank(&self, line: u32) -> bool {
        usize::try_from(line)
            .ok()
            .and_then(|line| self.blank.get(line.saturating_sub(1)))
            .copied()
            .unwrap_or(false)
    }
}

/// Read and parse one side, if it exists.
fn load_side(
    path: Option<&Path>,
    sources: &impl Sources,
    side: Side,
) -> Result<Option<LoadedSide>> {
    let Some(path) = path else { return Ok(None) };
    let source = match side {
        Side::Old => sources.old_source(path)?,
        Side::New => sources.current_source(path)?,
    };
    let Some(source) = source else { return Ok(None) };

    let parsed =
        pysource::parse(&source).with_context(|| format!("could not parse {}", path.display()))?;
    let blank: Vec<bool> = source.lines().map(|line| line.trim().is_empty()).collect();
    let module = sources.module_path(path).unwrap_or_else(|| fallback_module(path));

    Ok(Some(LoadedSide { path: path.to_path_buf(), module, parsed, blank }))
}

/// Module name for a path [`crate::modpath`] could not resolve — a non-UTF-8
/// stem, or a repository-root `__init__.py`.
fn fallback_module(path: &Path) -> String {
    path.with_extension("")
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(".")
}

/// The lines to attribute on each side.
fn touched(
    change: &FileChange,
    old: Option<&LoadedSide>,
    new: Option<&LoadedSide>,
) -> (Vec<u32>, Vec<u32>) {
    match &change.sides {
        Sides::Hunks { old: old_ranges, new: new_ranges } => {
            (diff::touched_lines(old_ranges), diff::touched_lines(new_ranges))
        }
        Sides::Whole => (all_lines(old), all_lines(new)),
    }
}

fn all_lines(side: Option<&LoadedSide>) -> Vec<u32> {
    side.map(|s| (1..=s.line_count()).collect()).unwrap_or_default()
}

fn find_span<'a>(spans: &'a [SymbolSpan], qualname: &str) -> Option<&'a SymbolSpan> {
    spans.iter().find(|span| span.qualname == qualname)
}

/// Whether `side` defines this exact symbol. The module must match too, which
/// is what stops a rename from looking like a modification.
fn has_symbol(side: Option<&LoadedSide>, module: &str, qualname: &str) -> bool {
    side.is_some_and(|s| s.module == module && find_span(&s.parsed.spans, qualname).is_some())
}

/// The new side's definition of this symbol, if it still has one.
fn surviving<'a>(
    new: Option<&'a LoadedSide>,
    module: &str,
    qualname: &str,
) -> Option<(&'a SymbolSpan, &'a LoadedSide)> {
    let side = new.filter(|s| s.module == module)?;
    find_span(&side.parsed.spans, qualname).map(|span| (span, side))
}

fn is_python(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "py")
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

/// Collects results, deduplicating as it goes; [`Accumulator::finish`] sorts.
#[derive(Default)]
struct Accumulator {
    changed: Vec<SymbolChange>,
    ignored: Vec<SymbolChange>,
    /// `(module, file)` pairs; the set deduplicates and the order is the
    /// module's, which is what the report sorts by.
    module_level: BTreeSet<(String, String)>,
    non_python: BTreeSet<String>,
    test_files: BTreeSet<String>,
    errors: BTreeSet<String>,
}

impl Accumulator {
    fn finish(self, base: &Base) -> ChangedSymbols {
        ChangedSymbols {
            base: base.name.clone(),
            merge_base: base.merge_base.clone(),
            changed_symbols: sorted(self.changed),
            ignored_symbols: sorted(self.ignored),
            module_level_changes: self
                .module_level
                .into_iter()
                .map(|(module, file)| ModuleChange { module, file })
                .collect(),
            non_python_changes: self.non_python.into_iter().collect(),
            test_files_changed: self.test_files.into_iter().collect(),
            errors: self.errors.into_iter().collect(),
        }
    }
}

/// Deterministic ordering, and one entry per symbol.
///
/// A symbol can be reached twice — an `@overload` stub and its implementation
/// share a qualname — and the report should mention it once.
fn sorted(mut entries: Vec<SymbolChange>) -> Vec<SymbolChange> {
    entries.sort_by(|a, b| (&a.symbol, &a.file).cmp(&(&b.symbol, &b.file)));
    // A stable sort plus `dedup_by` keeps the first entry of each run, which is
    // what the map this replaced did.
    entries.dedup_by(|a, b| a.symbol == b.symbol && a.file == b.file);
    entries
}

impl ChangedSymbols {
    /// True when the diff touched no Python at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changed_symbols.is_empty()
            && self.ignored_symbols.is_empty()
            && self.module_level_changes.is_empty()
            && self.non_python_changes.is_empty()
            && self.test_files_changed.is_empty()
    }

    pub fn render(&self, format: Format) -> Result<String> {
        match format {
            Format::Human => Ok(self.render_human()),
            Format::Json => Ok(serde_json::to_string_pretty(self)?),
        }
    }

    /// A short, aligned summary. JSON is the contract; this is for eyeballs.
    fn render_human(&self) -> String {
        use std::fmt::Write as _;

        let mut out = String::new();
        let short = self.merge_base.get(..7).unwrap_or(&self.merge_base);
        // Writing into a String is infallible, so the Results carry nothing.
        let _ = writeln!(out, "base {} (merge-base {short})", self.base);

        if self.is_empty() {
            let _ = writeln!(out, "\nNo changes against {}.", self.base);
            return out;
        }

        let width = self
            .changed_symbols
            .iter()
            .chain(&self.ignored_symbols)
            .map(|entry| entry.symbol.chars().count())
            .max()
            .unwrap_or(0);

        symbol_section(&mut out, "changed symbols", &self.changed_symbols, width);
        symbol_section(&mut out, "ignored symbols", &self.ignored_symbols, width);
        module_section(&mut out, "module-level changes", &self.module_level_changes);
        list_section(&mut out, "changed test files", &self.test_files_changed);
        list_section(&mut out, "non-Python changes", &self.non_python_changes);
        list_section(&mut out, "parse errors", &self.errors);

        out
    }
}

impl Change {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Modified => "modified",
            Self::Deleted => "deleted",
        }
    }
}

fn symbol_section(out: &mut String, title: &str, entries: &[SymbolChange], width: usize) {
    use std::fmt::Write as _;

    if entries.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n{title} ({})", entries.len());
    for entry in entries {
        let _ = write!(
            out,
            "  {:<8}  {:<8}  {:<width$}  {}",
            entry.change.label(),
            entry.kind.label(),
            entry.symbol,
            entry.file,
        );
        match &entry.ignored_by {
            Some(decorator) => {
                let _ = writeln!(out, "  (@{decorator})");
            }
            None => {
                let _ = writeln!(out);
            }
        }
    }
}

fn module_section(out: &mut String, title: &str, entries: &[ModuleChange]) {
    use std::fmt::Write as _;

    if entries.is_empty() {
        return;
    }
    let width = entries.iter().map(|e| e.module.chars().count()).max().unwrap_or(0);
    let _ = writeln!(out, "\n{title} ({})", entries.len());
    for entry in entries {
        let _ = writeln!(out, "  {:<width$}  {}", entry.module, entry.file);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::diff::LineRange;

    /// A [`Sources`] backed by two maps, so classification is testable without
    /// git, a filesystem, or a Python environment.
    #[derive(Default)]
    struct MapSources {
        old: HashMap<PathBuf, String>,
        new: HashMap<PathBuf, String>,
    }

    impl MapSources {
        fn with_old(mut self, path: &str, body: &str) -> Self {
            self.old.insert(PathBuf::from(path), body.to_string());
            self
        }

        fn with_new(mut self, path: &str, body: &str) -> Self {
            self.new.insert(PathBuf::from(path), body.to_string());
            self
        }

        /// Same body on both sides, for "only some lines changed" cases.
        fn with_both(self, path: &str, body: &str) -> Self {
            self.with_old(path, body).with_new(path, body)
        }
    }

    impl Sources for MapSources {
        fn old_source(&self, path: &Path) -> Result<Option<String>> {
            Ok(self.old.get(path).cloned())
        }

        fn current_source(&self, path: &Path) -> Result<Option<String>> {
            Ok(self.new.get(path).cloned())
        }

        fn module_path(&self, path: &Path) -> Option<String> {
            let stem = path.with_extension("");
            let dotted = stem.to_str()?.trim_start_matches("src/").replace('/', ".");
            Some(dotted)
        }
    }

    fn base() -> Base {
        Base { name: "origin/main".to_string(), merge_base: "abc123".to_string() }
    }

    fn hunks(old: &[(u32, u32)], new: &[(u32, u32)]) -> Sides {
        let range = |&(start, count): &(u32, u32)| LineRange { start, count };
        Sides::Hunks { old: old.iter().map(range).collect(), new: new.iter().map(range).collect() }
    }

    fn modified(path: &str, sides: Sides) -> FileChange {
        FileChange {
            old_path: Some(PathBuf::from(path)),
            new_path: Some(PathBuf::from(path)),
            sides,
            binary: false,
        }
    }

    fn run(changes: &[FileChange], sources: &MapSources) -> ChangedSymbols {
        run_with(changes, sources, &Config::default())
    }

    fn run_with(changes: &[FileChange], sources: &MapSources, config: &Config) -> ChangedSymbols {
        analyze(&base(), changes, Path::new("/repo"), sources, config)
            .expect("the map-backed sources never fail")
    }

    /// `(symbol, change)` pairs, for compact assertions.
    fn pairs(entries: &[SymbolChange]) -> Vec<(String, Change)> {
        entries.iter().map(|e| (e.symbol.clone(), e.change)).collect()
    }

    /// The module names of a report's module-level changes.
    fn modules(report: &ChangedSymbols) -> Vec<&str> {
        report.module_level_changes.iter().map(|m| m.module.as_str()).collect()
    }

    const BEFORE: &str = "\
import os

LIMIT = 10


def keep(x):
    return x + 1


def goes_away():
    return 0


class Service:
    def run(self):
        return 1
";

    #[test]
    fn a_changed_body_line_reports_the_function_as_modified() {
        let sources = MapSources::default().with_both("src/pkg/mod.py", BEFORE);
        let report = run(&[modified("src/pkg/mod.py", hunks(&[(7, 1)], &[(7, 1)]))], &sources);

        assert_eq!(
            pairs(&report.changed_symbols),
            vec![("pkg.mod:keep".to_string(), Change::Modified)],
            "the line inside `keep` should name only `keep`"
        );
        assert_eq!(report.changed_symbols[0].kind, Kind::Function, "kind comes from the parse");
        assert_eq!(report.changed_symbols[0].file, "src/pkg/mod.py", "reported under its path");
    }

    #[test]
    fn the_base_and_merge_base_are_carried_into_the_report() {
        let report = run(&[], &MapSources::default());
        assert_eq!(report.base, "origin/main", "the ref that was resolved");
        assert_eq!(report.merge_base, "abc123", "and the commit it resolved to");
        assert!(report.is_empty(), "an empty change list produces an empty report");
    }

    #[test]
    fn a_new_function_is_added() {
        let after = format!("{BEFORE}\n\ndef fresh():\n    return 2\n");
        let sources = MapSources::default()
            .with_old("src/pkg/mod.py", BEFORE)
            .with_new("src/pkg/mod.py", &after);
        // The new definition sits on lines 19-20 of the new file.
        let report = run(&[modified("src/pkg/mod.py", hunks(&[], &[(19, 2)]))], &sources);

        assert_eq!(
            pairs(&report.changed_symbols),
            vec![("pkg.mod:fresh".to_string(), Change::Added)],
            "a symbol only the new side knows about is `added`"
        );
    }

    #[test]
    fn a_removed_function_is_deleted() {
        let after = BEFORE.replace("def goes_away():\n    return 0\n", "");
        let sources = MapSources::default()
            .with_old("src/pkg/mod.py", BEFORE)
            .with_new("src/pkg/mod.py", &after);
        let report = run(&[modified("src/pkg/mod.py", hunks(&[(11, 2)], &[]))], &sources);

        assert_eq!(
            pairs(&report.changed_symbols),
            vec![("pkg.mod:goes_away".to_string(), Change::Deleted)],
            "a symbol only the old side knows about is `deleted`"
        );
    }

    #[test]
    fn deleting_lines_from_a_surviving_function_is_a_modification() {
        // The load-bearing case: the hunk touches only the old side, but the
        // function is still there, so its callers were not orphaned.
        let after = "\
import os

LIMIT = 10


def keep(x):
    return x + 1


def goes_away():
    return 0


class Service:
    def run(self):
        return 1
"
        .replace("    return x + 1\n", "");
        let sources = MapSources::default()
            .with_old("src/pkg/mod.py", BEFORE)
            .with_new("src/pkg/mod.py", &after);
        let report = run(&[modified("src/pkg/mod.py", hunks(&[(7, 1)], &[]))], &sources);

        assert_eq!(
            pairs(&report.changed_symbols),
            vec![("pkg.mod:keep".to_string(), Change::Modified)],
            "existence on both sides decides the verdict, not which side the hunk touched"
        );
    }

    #[test]
    fn a_method_keeps_its_class_in_the_symbol_name() {
        let sources = MapSources::default().with_both("src/pkg/mod.py", BEFORE);
        let report = run(&[modified("src/pkg/mod.py", hunks(&[(16, 1)], &[(16, 1)]))], &sources);

        assert_eq!(
            pairs(&report.changed_symbols),
            vec![("pkg.mod:Service.run".to_string(), Change::Modified)],
            "the symbol id is module.path:QualName"
        );
        assert_eq!(report.changed_symbols[0].kind, Kind::Method, "and its kind is method");
    }

    #[test]
    fn a_module_level_line_reports_the_module_not_a_symbol() {
        let sources = MapSources::default().with_both("src/pkg/mod.py", BEFORE);
        let report = run(&[modified("src/pkg/mod.py", hunks(&[(3, 1)], &[(3, 1)]))], &sources);

        assert!(report.changed_symbols.is_empty(), "a constant belongs to no symbol");
        assert_eq!(
            modules(&report),
            vec!["pkg.mod"],
            "it is reported as a whole-module change instead"
        );
        assert_eq!(
            report.module_level_changes[0].file, "src/pkg/mod.py",
            "the file travels with the module, so phase 2 can outline it"
        );
    }

    #[test]
    fn a_file_can_report_both_symbol_and_module_level_changes() {
        let sources = MapSources::default().with_both("src/pkg/mod.py", BEFORE);
        let report = run(
            &[modified("src/pkg/mod.py", hunks(&[(1, 1), (7, 1)], &[(1, 1), (7, 1)]))],
            &sources,
        );

        assert_eq!(
            pairs(&report.changed_symbols),
            vec![("pkg.mod:keep".to_string(), Change::Modified)]
        );
        assert_eq!(modules(&report), vec!["pkg.mod"], "the import too");
    }

    #[test]
    fn an_added_file_reports_every_symbol_in_it() {
        let sources = MapSources::default().with_new("src/pkg/fresh.py", BEFORE);
        let change = FileChange {
            old_path: None,
            new_path: Some(PathBuf::from("src/pkg/fresh.py")),
            sides: Sides::Whole,
            binary: false,
        };
        let report = run(&[change], &sources);

        assert_eq!(
            pairs(&report.changed_symbols),
            vec![
                ("pkg.fresh:Service".to_string(), Change::Added),
                ("pkg.fresh:Service.run".to_string(), Change::Added),
                ("pkg.fresh:goes_away".to_string(), Change::Added),
                ("pkg.fresh:keep".to_string(), Change::Added),
            ],
            "every definition in a new file is added, sorted by symbol"
        );
    }

    #[test]
    fn a_deleted_file_reports_every_symbol_as_deleted() {
        let sources = MapSources::default().with_old("src/pkg/gone.py", BEFORE);
        let change = FileChange {
            old_path: Some(PathBuf::from("src/pkg/gone.py")),
            new_path: None,
            sides: Sides::Whole,
            binary: false,
        };
        let report = run(&[change], &sources);

        assert!(
            report.changed_symbols.iter().all(|s| s.change == Change::Deleted),
            "nothing survives a deleted file, got {:?}",
            pairs(&report.changed_symbols)
        );
        assert_eq!(report.changed_symbols.len(), 4, "all four definitions are reported");
        assert_eq!(modules(&report), vec!["pkg.gone"], "its module-level code went too");
    }

    #[test]
    fn a_rename_produces_a_deleted_and_an_added_set_without_pairing_them() {
        let sources = MapSources::default()
            .with_old("src/pkg/old.py", "def f():\n    return 1\n")
            .with_new("src/pkg/new.py", "def f():\n    return 1\n");
        let change = FileChange {
            old_path: Some(PathBuf::from("src/pkg/old.py")),
            new_path: Some(PathBuf::from("src/pkg/new.py")),
            sides: Sides::Whole,
            binary: false,
        };
        let report = run(&[change], &sources);

        assert_eq!(
            pairs(&report.changed_symbols),
            vec![
                ("pkg.new:f".to_string(), Change::Added),
                ("pkg.old:f".to_string(), Change::Deleted),
            ],
            "phase 1 does not pair renames: callers of the old name still need chasing"
        );
    }

    #[test]
    fn a_rename_ignores_the_residual_hunks_and_takes_the_whole_file() {
        // git -M reports only the lines that differ; that would under-report a
        // move, so FileChange::from promotes renames to Sides::Whole.
        let file = FileDiff {
            old_path: Some(PathBuf::from("a.py")),
            new_path: Some(PathBuf::from("b.py")),
            old_ranges: vec![LineRange { start: 1, count: 1 }],
            new_ranges: vec![LineRange { start: 1, count: 1 }],
            binary: false,
        };
        assert_eq!(FileChange::from(file).sides, Sides::Whole, "renames take everything");
    }

    #[test]
    fn an_untracked_file_is_treated_as_wholly_added() {
        let sources =
            MapSources::default().with_new("src/pkg/fresh.py", "def f():\n    return 1\n");
        let report = run(&[untracked_change(PathBuf::from("src/pkg/fresh.py"))], &sources);

        assert_eq!(
            pairs(&report.changed_symbols),
            vec![("pkg.fresh:f".to_string(), Change::Added)],
            "git diff cannot see untracked files, so they are fed in separately"
        );
    }

    #[test]
    fn an_ignored_decorator_moves_the_symbol_out_of_changed_symbols() {
        let body = "\
@transformation
def normalize_prices(df):
    return df
";
        let sources = MapSources::default().with_both("src/pkg/daily.py", body);
        let config =
            Config { ignore_decorators: vec!["transformation".to_string()], ..Config::default() };
        let report = run_with(
            &[modified("src/pkg/daily.py", hunks(&[(3, 1)], &[(3, 1)]))],
            &sources,
            &config,
        );

        assert!(report.changed_symbols.is_empty(), "a registry function is not worth chasing");
        assert_eq!(
            pairs(&report.ignored_symbols),
            vec![("pkg.daily:normalize_prices".to_string(), Change::Modified)],
            "it is reported as ignored rather than dropped"
        );
        assert_eq!(
            report.ignored_symbols[0].ignored_by.as_deref(),
            Some("transformation"),
            "the report says which config entry matched"
        );
    }

    #[test]
    fn a_registry_decorator_wins_over_a_non_matching_one() {
        let body = "\
@staticmethod
@registry.transformation(name=\"x\")
def normalize(df):
    return df
";
        let sources = MapSources::default().with_both("src/pkg/daily.py", body);
        let config =
            Config { ignore_decorators: vec!["transformation".to_string()], ..Config::default() };
        let report = run_with(
            &[modified("src/pkg/daily.py", hunks(&[(4, 1)], &[(4, 1)]))],
            &sources,
            &config,
        );

        assert!(report.changed_symbols.is_empty(), "one matching decorator is enough");
        assert_eq!(
            report.ignored_symbols[0].ignored_by.as_deref(),
            Some("transformation"),
            "suffix matching reaches into the dotted form"
        );
    }

    #[test]
    fn an_aliased_decorator_is_not_matched() {
        // Documented limitation: matching is syntactic, so an alias escapes it.
        let body = "\
from registry import transformation as t


@t
def normalize(df):
    return df
";
        let sources = MapSources::default().with_both("src/pkg/daily.py", body);
        let config =
            Config { ignore_decorators: vec!["transformation".to_string()], ..Config::default() };
        let report = run_with(
            &[modified("src/pkg/daily.py", hunks(&[(6, 1)], &[(6, 1)]))],
            &sources,
            &config,
        );

        assert_eq!(
            pairs(&report.changed_symbols),
            vec![("pkg.daily:normalize".to_string(), Change::Modified)],
            "import aliases are deliberately not resolved in phase 1"
        );
        assert!(report.ignored_symbols.is_empty(), "so nothing is ignored");
    }

    #[test]
    fn ignoring_applies_only_to_the_decorated_symbol() {
        let body = "\
@transformation
def registered(df):
    return df


def ordinary(x):
    return x
";
        let sources = MapSources::default().with_both("src/pkg/daily.py", body);
        let config =
            Config { ignore_decorators: vec!["transformation".to_string()], ..Config::default() };
        let report = run_with(
            &[modified("src/pkg/daily.py", hunks(&[(3, 1), (7, 1)], &[(3, 1), (7, 1)]))],
            &sources,
            &config,
        );

        assert_eq!(
            pairs(&report.changed_symbols),
            vec![("pkg.daily:ordinary".to_string(), Change::Modified)],
            "the neighbouring function is unaffected"
        );
        assert_eq!(report.ignored_symbols.len(), 1, "only the decorated one is ignored");
    }

    #[test]
    fn a_decorated_class_can_be_ignored_too() {
        let body = "\
@registry.model
class Row:
    field: int = 0
";
        let sources = MapSources::default().with_both("src/pkg/daily.py", body);
        let config =
            Config { ignore_decorators: vec!["registry.model".to_string()], ..Config::default() };
        let report = run_with(
            &[modified("src/pkg/daily.py", hunks(&[(3, 1)], &[(3, 1)]))],
            &sources,
            &config,
        );

        assert_eq!(report.ignored_symbols[0].kind, Kind::Class, "classes take decorators as well");
    }

    #[test]
    fn a_syntax_error_degrades_the_file_to_a_module_level_change() {
        let sources = MapSources::default()
            .with_old("src/pkg/mod.py", BEFORE)
            .with_new("src/pkg/mod.py", "def broken(:\n    pass\n");
        let report = run(&[modified("src/pkg/mod.py", hunks(&[(7, 1)], &[(1, 1)]))], &sources);

        assert_eq!(report.errors, vec!["src/pkg/mod.py".to_string()], "the file is named");
        assert_eq!(
            modules(&report),
            vec!["pkg.mod"],
            "an untrustworthy tree means the whole module is suspect"
        );
        assert!(report.changed_symbols.is_empty(), "no symbol claim is made from a broken parse");
    }

    #[test]
    fn a_rename_whose_old_blob_is_broken_reports_both_modules() {
        // Only the old side fails to parse, and the whole file still degrades:
        // a partial tree cannot be trusted to say which symbol owns which line.
        let sources = MapSources::default()
            .with_old("src/pkg/old.py", "def broken(:\n    pass\n")
            .with_new("src/pkg/new.py", BEFORE);
        let change = FileChange {
            old_path: Some(PathBuf::from("src/pkg/old.py")),
            new_path: Some(PathBuf::from("src/pkg/new.py")),
            sides: Sides::Whole,
            binary: false,
        };
        let report = run(&[change], &sources);

        assert_eq!(
            report.errors,
            vec!["src/pkg/new.py".to_string()],
            "the file is reported under its new home, as everything else is"
        );
        assert_eq!(
            modules(&report),
            vec!["pkg.new", "pkg.old"],
            "both modules are suspect: callers of either have to be revisited"
        );
        assert!(report.changed_symbols.is_empty(), "no symbol claim from a broken parse");
    }

    #[test]
    fn a_test_file_is_listed_but_not_analysed() {
        let sources = MapSources::default().with_both("tests/test_mod.py", BEFORE);
        let report = run(&[modified("tests/test_mod.py", hunks(&[(7, 1)], &[(7, 1)]))], &sources);

        assert_eq!(report.test_files_changed, vec!["tests/test_mod.py".to_string()], "listed");
        assert!(report.changed_symbols.is_empty(), "a changed test simply selects itself later");
        assert!(report.module_level_changes.is_empty(), "and produces no module-level noise");
    }

    #[test]
    fn non_python_files_are_partitioned_off() {
        let sources = MapSources::default();
        let report = run(
            &[
                modified("pyproject.toml", hunks(&[(1, 1)], &[(1, 1)])),
                modified("data/schema.sql", hunks(&[(1, 1)], &[(1, 1)])),
                modified("src/pkg/stub.pyi", hunks(&[(1, 1)], &[(1, 1)])),
            ],
            &sources,
        );

        assert_eq!(
            report.non_python_changes,
            vec![
                "data/schema.sql".to_string(),
                "pyproject.toml".to_string(),
                "src/pkg/stub.pyi".to_string(),
            ],
            "stubs are a phase-1 non-goal, so .pyi counts as non-Python"
        );
    }

    #[test]
    fn a_binary_file_is_never_parsed() {
        let mut change = modified("src/pkg/mod.py", hunks(&[], &[]));
        change.binary = true;
        let report = run(&[change], &MapSources::default());

        assert_eq!(
            report.non_python_changes,
            vec!["src/pkg/mod.py".to_string()],
            "a .py git calls binary is not source we can trust"
        );
    }

    #[test]
    fn overloads_are_reported_once() {
        let body = "\
from typing import overload


@overload
def f(x: int) -> int: ...


def f(x):
    return x
";
        let sources = MapSources::default().with_both("src/pkg/mod.py", body);
        let report = run(
            &[modified("src/pkg/mod.py", hunks(&[(5, 1), (9, 1)], &[(5, 1), (9, 1)]))],
            &sources,
        );

        assert_eq!(
            pairs(&report.changed_symbols),
            vec![("pkg.mod:f".to_string(), Change::Modified)],
            "a stub and its implementation share a symbol, so they share an entry"
        );
    }

    #[test]
    fn blank_lines_do_not_count_as_module_level_changes() {
        // Appending a function inserts two blank separator lines; treating them
        // as module-level would make every added function look like a
        // whole-module change.
        let after = format!("{BEFORE}\n\ndef fresh():\n    return 2\n");
        let sources = MapSources::default()
            .with_old("src/pkg/mod.py", BEFORE)
            .with_new("src/pkg/mod.py", &after);
        let report = run(&[modified("src/pkg/mod.py", hunks(&[], &[(17, 4)]))], &sources);

        assert_eq!(
            pairs(&report.changed_symbols),
            vec![("pkg.mod:fresh".to_string(), Change::Added)],
            "the new function is still found"
        );
        assert!(
            report.module_level_changes.is_empty(),
            "but the blank lines before it are not a module-level change, got {:?}",
            report.module_level_changes
        );
    }

    #[test]
    fn a_non_blank_module_level_line_still_counts() {
        let sources = MapSources::default().with_both("src/pkg/mod.py", BEFORE);
        let report = run(&[modified("src/pkg/mod.py", hunks(&[(3, 1)], &[(3, 1)]))], &sources);
        assert_eq!(
            modules(&report),
            vec!["pkg.mod"],
            "skipping blanks must not skip real module-level code"
        );
    }

    #[test]
    fn output_arrays_are_sorted() {
        let sources = MapSources::default()
            .with_both("src/pkg/b.py", BEFORE)
            .with_both("src/pkg/a.py", BEFORE);
        let report = run(
            &[
                modified("src/pkg/b.py", hunks(&[(16, 1)], &[(16, 1)])),
                modified("src/pkg/a.py", hunks(&[(7, 1)], &[(7, 1)])),
            ],
            &sources,
        );

        let symbols: Vec<&str> = report.changed_symbols.iter().map(|s| s.symbol.as_str()).collect();
        let mut sorted = symbols.clone();
        sorted.sort_unstable();
        assert_eq!(symbols, sorted, "callers diff this output; ordering must be stable");
    }

    #[test]
    fn a_symbol_carries_the_line_of_its_name_not_its_decorator() {
        let body = "\
@registry.thing
def normalize(df):
    return df
";
        let sources = MapSources::default().with_both("src/pkg/daily.py", body);
        let report = run(&[modified("src/pkg/daily.py", hunks(&[(3, 1)], &[(3, 1)]))], &sources);

        assert_eq!(
            (report.changed_symbols[0].line, report.changed_symbols[0].column),
            (2, 5),
            "phase 2 queries `file:line:col`, which means the `def` line, not the decorator"
        );
    }

    #[test]
    fn the_report_round_trips_through_json() {
        // `--changed report.json` replays a saved phase-1 run, so the schema has
        // to deserialise into exactly what produced it.
        let sources = MapSources::default().with_both("src/pkg/mod.py", BEFORE);
        let report = run(
            &[modified("src/pkg/mod.py", hunks(&[(3, 1), (7, 1)], &[(3, 1), (7, 1)]))],
            &sources,
        );

        let json = serde_json::to_string(&report).expect("the report serialises");
        let back: ChangedSymbols = serde_json::from_str(&json).expect("and deserialises");
        assert_eq!(back, report, "a round trip must not lose or reshape anything");
    }

    #[test]
    fn a_report_missing_optional_arrays_still_deserialises() {
        let back: ChangedSymbols =
            serde_json::from_str(r#"{"base": "main", "merge_base": "abc123"}"#)
                .expect("a hand-written --changed file need not spell out empty arrays");
        assert_eq!(back.base, "main");
        assert!(back.changed_symbols.is_empty(), "absent arrays default to empty");
    }

    #[test]
    fn a_report_without_a_base_is_rejected_rather_than_defaulted() {
        // The load-bearing case: `{}` used to deserialise into an empty report,
        // which phase 2 then walked to a confident `selected` verdict selecting
        // nothing. A CI job piping that into pytest runs no tests and passes.
        let err = serde_json::from_str::<ChangedSymbols>("{}")
            .expect_err("a report with no base is not a report");
        assert!(err.to_string().contains("base"), "the error names what is missing: {err}");
    }

    #[test]
    fn an_unknown_key_is_rejected_rather_than_ignored() {
        // A typo, or a report written by a different gerenuk. Both mean the
        // arrays we *did* read may be incomplete, so neither can be walked.
        let err = serde_json::from_str::<ChangedSymbols>(
            r#"{"base": "main", "merge_base": "abc", "changed_symbol": []}"#,
        )
        .expect_err("a misspelled key must not silently read as an empty array");
        assert!(err.to_string().contains("changed_symbol"), "the error names the key: {err}");
    }

    #[test]
    fn changed_symbols_omit_the_ignored_by_field_in_json() {
        let sources = MapSources::default().with_both("src/pkg/mod.py", BEFORE);
        let report = run(&[modified("src/pkg/mod.py", hunks(&[(7, 1)], &[(7, 1)]))], &sources);
        let json = serde_json::to_string(&report).expect("the report serialises");

        assert!(json.contains("\"symbol\":\"pkg.mod:keep\""), "the symbol id is in the JSON");
        assert!(
            !json.contains("ignored_by"),
            "a non-ignored symbol must not carry a null field, got: {json}"
        );
    }
}
