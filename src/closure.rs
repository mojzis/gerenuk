//! Walking the reverse reference graph from changed symbols to the tests that
//! reach them.
//!
//! This module is pure. It reaches the world through two traits — [`Refs`],
//! "who references these symbols?", and [`Index`], "what owns this line?" — so
//! the whole search is unit-testable from a `HashMap`, with no `tyf`, no `ty`,
//! no `git` and no Python. See `docs/adr/0001-two-impure-seams.md`.
//!
//! ## Identifiers
//!
//! A symbol id is `module.path:QualName`. An id with **no colon is a module**:
//! that is what a reference landing at module scope reaches, and it is what a
//! module-level change seeds. `docs/adr/0008-module-ids-have-no-colon.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::changed::{Change, ChangedSymbols};
use crate::config::Config;
use crate::modpath::{split_symbol_id, symbol_id};

/// BFS levels walked before the search gives up.
pub const DEFAULT_MAX_DEPTH: u32 = 10;
/// Symbols visited before the search gives up.
pub const DEFAULT_MAX_SYMBOLS: usize = 500;
/// Wall-clock budget for a whole walk, in milliseconds.
pub const DEFAULT_BUDGET_MS: u64 = 30_000;

/// One symbol the walk asks [`Refs`] about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolQuery {
    /// `module.path:QualName`, the node's identity in the graph.
    pub id: String,
    /// The qualified name within its module, which is what `tyf` is asked for.
    pub name: String,
    /// Repository-relative path of the definition.
    pub file: PathBuf,
    /// Line of the definition's name.
    ///
    /// `tyf refs` includes the declaration itself, so this is what tells a
    /// symbol's own definition apart from a genuine usage.
    pub line: u32,
    /// 1-based column of the definition's name. With [`Self::file`] and
    /// [`Self::line`] this is the position [`Refs`] resolves.
    pub column: u32,
}

impl SymbolQuery {
    /// The last dotted component of [`Self::name`] — `run` for `Enricher.run`.
    ///
    /// This is what a textual search for a deleted symbol looks for: the
    /// qualified form never appears at a call site.
    #[must_use]
    pub fn bare_name(&self) -> &str {
        self.name.rsplit('.').next().unwrap_or(&self.name)
    }

    /// The site [`Refs`] will report for the definition itself.
    #[must_use]
    pub fn declaration(&self) -> RefSite {
        RefSite { file: self.file.clone(), line: self.line }
    }
}

/// How a seed enters the walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedKind {
    /// The definition is in the working tree; `tyf` can resolve it.
    Live,
    /// The definition is gone, so references to it cannot be resolved and the
    /// walk falls back to a word-boundary textual scan.
    Deleted,
    /// A whole module changed at its top level. The module selects the tests
    /// that import it; its definitions are seeded separately as [`Self::Live`].
    Module,
}

/// A symbol the diff changed, and the way the walk has to start from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seed {
    /// The symbol, and where its definition is (or was).
    pub query: SymbolQuery,
    /// How the walk has to start from it.
    pub kind: SeedKind,
}

/// One place a symbol is referenced from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefSite {
    /// Repository-relative path.
    pub file: PathBuf,
    /// 1-based line.
    pub line: u32,
}

/// Everything [`Refs`] found for one query, keyed so order does not matter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefAnswer {
    /// The [`SymbolQuery::id`] this answers, echoed back by the seam.
    pub id: String,
    /// Every place the symbol is referenced, its own declaration included.
    pub sites: Vec<RefSite>,
}

/// "Who references these symbols?"
///
/// The whole BFS frontier arrives in one call, and the real implementation
/// sends it to `tyf` as one invocation. Answers are keyed by id rather than
/// positional so the seam is free to reorder them.
/// See `docs/adr/0003-batch-the-frontier.md`.
pub trait Refs {
    fn refs(&self, queries: &[SymbolQuery]) -> Result<Vec<RefAnswer>>;
}

/// A definition, as the working tree currently has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedSymbol {
    /// Dotted name within its module, e.g. `Enricher.run`.
    pub qualname: String,
    /// Line of the definition's name.
    pub line: u32,
    /// 1-based column of the definition's name.
    pub column: u32,
    /// Dotted names of the decorators applied to it.
    pub decorators: Vec<String>,
}

/// What a referencing line turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Site {
    /// A plain `import` / `from … import` at module scope.
    ///
    /// Dropped: the usages inside the importing module show up as their own
    /// references, so the import line is redundant — and keeping it would pull
    /// in every module that so much as mentions the changed one.
    Import,
    /// The line belongs to a definition.
    Symbol(IndexedSymbol),
    /// Module scope, and not an import: a constant, a decorator argument, or a
    /// call that runs at import time.
    ModuleLevel,
    /// The file could not be read or parsed.
    Unknown,
}

/// One definition, as [`Index`] implementations report it.
///
/// Both the real [`crate::impact::FsIndex`] and the unit tests' fake go through
/// this, so the two cannot drift apart on what a span becomes.
#[must_use]
pub(crate) fn indexed(span: &crate::pysource::SymbolSpan) -> IndexedSymbol {
    IndexedSymbol {
        qualname: span.qualname.clone(),
        line: span.name_line,
        column: span.name_column,
        decorators: span.decorators.clone(),
    }
}

/// The [`Site`] a line falls in, given an already-parsed module.
///
/// The enclosing definition wins; only a line that belongs to no definition is
/// tested for being an import, which is what keeps an import *inside* a
/// function attributed to the function.
#[must_use]
pub(crate) fn classify_in(module: &crate::pysource::Module, line: u32) -> Site {
    module.symbol_at(line).map_or_else(
        || if module.is_import_line(line) { Site::Import } else { Site::ModuleLevel },
        |span| Site::Symbol(indexed(span)),
    )
}

/// "What does the working tree say about this line?"
pub trait Index {
    /// What owns `line` in `file`.
    fn classify(&self, file: &Path, line: u32) -> Result<Site>;
    /// Dotted module path for a repository-relative `.py` path.
    fn module_path(&self, file: &Path) -> Option<String>;
    /// Whether the path is test code.
    fn is_test(&self, file: &Path) -> bool;
    /// Module-scope definitions of one file.
    fn top_level(&self, file: &Path) -> Result<Vec<IndexedSymbol>>;
    /// Word-boundary occurrences of a bare name across the workspace's Python
    /// files, as `(file, line)` pairs.
    fn word_hits(&self, name: &str) -> Result<Vec<(PathBuf, u32)>>;
    /// Test files whose text imports `module`.
    fn test_importers(&self, module: &str) -> Result<Vec<PathBuf>>;
}

/// What stops a walk short of a complete answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// The diff touched files gerenuk cannot reason about at all.
    NonPythonChanges,
    /// Phase 1 could not parse a changed file.
    ParseErrors,
    /// `tyf` is not installed, so no reference can be resolved.
    TyfUnavailable,
    /// `tyf` failed part-way through the walk.
    RefsFailed,
    /// The working tree could not be read part-way through the walk.
    IndexFailed,
    /// The frontier was still growing when `max-depth` levels were done.
    MaxDepth,
    /// More than `max-symbols` nodes were visited.
    MaxSymbols,
    /// The wall-clock budget ran out between levels.
    Budget,
}

impl Reason {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::NonPythonChanges => "non-Python files changed",
            Self::ParseErrors => "a changed file did not parse",
            Self::TyfUnavailable => "tyf is not available",
            Self::RefsFailed => "tyf failed during the walk",
            Self::IndexFailed => "the working tree could not be read",
            Self::MaxDepth => "the depth limit was reached",
            Self::MaxSymbols => "the symbol limit was reached",
            Self::Budget => "the time budget ran out",
        }
    }
}

/// Whether the caller may trust `impacted_tests` as the whole answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The closure completed. Run the tests listed.
    Selected,
    /// Run the whole suite; `reason` says why the selection is not trustworthy.
    RunAll,
}

/// One test the change can reach.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactedTest {
    /// Repository-relative path of the test file.
    pub file: String,
    /// `module:test_name`, or `null` when the whole file is selected.
    pub symbol: Option<String>,
    /// The symbols between the test and [`Self::origin`], nearest the test
    /// first, excluding both ends. Empty means the test references the changed
    /// symbol directly. See `docs/adr/0005-why-chain-excludes-endpoints.md`.
    pub via: Vec<String>,
    /// The changed symbol this test was reached from.
    pub origin: String,
}

/// A symbol the walk refused to expand because of `ignore-decorators`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IgnoredSymbol {
    /// `module.path:QualName` of the symbol that was not expanded.
    pub symbol: String,
    /// Repository-relative path of its definition.
    pub file: String,
    /// The `ignore-decorators` entry that matched.
    pub ignored_by: String,
}

/// Cost of one walk.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stats {
    /// Symbols the walk started from.
    pub seeds: usize,
    /// Distinct graph nodes reached, seeds included.
    pub visited: usize,
    /// Deepest BFS level expanded, counting the seed frontier as `0`.
    pub max_depth_reached: u32,
    /// `tyf refs` invocations — one per BFS level, not per symbol.
    pub tyf_calls: usize,
    pub duration_ms: u128,
}

/// Where a walk gives up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// BFS levels expanded past the seed frontier, which always runs.
    pub max_depth: u32,
    /// Nodes visited before the walk gives up.
    pub max_symbols: usize,
    /// When the walk must stop. `None` disables the wall-clock budget, which is
    /// what the unit tests use.
    pub deadline: Option<Instant>,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_depth: DEFAULT_MAX_DEPTH, max_symbols: DEFAULT_MAX_SYMBOLS, deadline: None }
    }
}

/// The whole answer for one walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Closure {
    /// Whether `impacted` can be trusted as the whole answer.
    pub verdict: Verdict,
    /// Why it cannot; `None` when the walk finished.
    pub reason: Option<Reason>,
    /// The tests reached, sorted and deduplicated.
    pub impacted: Vec<ImpactedTest>,
    /// Symbols an `ignore-decorators` entry stopped the walk at.
    pub ignored: Vec<IgnoredSymbol>,
    /// Everything that went wrong without ending the walk.
    pub errors: Vec<String>,
    /// What the walk cost.
    pub stats: Stats,
}

/// Turn a phase-1 report into the symbols a walk starts from.
///
/// `changed_symbols` seed directly; `ignored_symbols` never do — that was the
/// point of phase 1's filter. Each `module_level_changes` entry seeds the
/// module itself plus every module-scope definition in it.
pub fn seeds_from(report: &ChangedSymbols, index: &impl Index) -> Result<Vec<Seed>> {
    let mut seeds = Vec::new();

    for entry in &report.changed_symbols {
        let Some((_, qualname)) = split_symbol_id(&entry.symbol) else { continue };
        seeds.push(Seed {
            query: SymbolQuery {
                id: entry.symbol.clone(),
                name: qualname.to_string(),
                file: PathBuf::from(&entry.file),
                line: entry.line,
                column: entry.column,
            },
            kind: if entry.change == Change::Deleted { SeedKind::Deleted } else { SeedKind::Live },
        });
    }

    for entry in &report.module_level_changes {
        let file = PathBuf::from(&entry.file);
        seeds.push(Seed {
            query: SymbolQuery {
                id: entry.module.clone(),
                name: entry.module.clone(),
                file: file.clone(),
                line: 0,
                column: 0,
            },
            kind: SeedKind::Module,
        });
        for symbol in index.top_level(&file)? {
            seeds.push(Seed {
                query: SymbolQuery {
                    id: symbol_id(&entry.module, &symbol.qualname),
                    name: symbol.qualname,
                    file: file.clone(),
                    line: symbol.line,
                    column: symbol.column,
                },
                kind: SeedKind::Live,
            });
        }
    }

    seeds.sort_by(|a, b| a.query.id.cmp(&b.query.id));
    seeds.dedup_by(|a, b| a.query.id == b.query.id);
    Ok(seeds)
}

/// Walk from `seeds` until the frontier empties or a limit trips.
///
/// Never fails: a seam that errors mid-walk degrades to
/// [`Verdict::RunAll`], because "run everything" is a usable answer for a
/// pre-commit hook and a crash is not.
/// See `docs/adr/0009-run-all-is-a-success.md`.
pub fn walk(
    seeds: &[Seed],
    refs: &impl Refs,
    index: &impl Index,
    config: &Config,
    limits: &Limits,
) -> Closure {
    Walk {
        refs,
        index,
        config,
        limits,
        visited: BTreeMap::new(),
        impacted: BTreeMap::new(),
        ignored: BTreeMap::new(),
        errors: Vec::new(),
        tyf_calls: 0,
        max_depth_reached: 0,
    }
    .run(seeds)
}

/// Drop per-function selections for a file that is selected wholesale.
///
/// A module-level edge selects a whole test file; a reference inside one of its
/// tests selects that test. When both happen, the file entry already covers
/// every test in it, and emitting both would hand phase 3 the same file twice —
/// once as `file.py` and once as `file.py::test_x`.
///
/// The surviving entry keeps the whole-file chain rather than the shorter
/// per-test one, because that chain is what explains the coarser selection.
fn collapse_whole_files(tests: Vec<ImpactedTest>) -> Vec<ImpactedTest> {
    let whole: BTreeSet<String> =
        tests.iter().filter(|test| test.symbol.is_none()).map(|test| test.file.clone()).collect();

    if whole.is_empty() {
        return tests;
    }
    tests.into_iter().filter(|test| test.symbol.is_none() || !whole.contains(&test.file)).collect()
}

/// How a node was first reached, which is its shortest path by construction.
#[derive(Debug, Clone)]
struct Visit {
    predecessor: Option<String>,
    origin: String,
}

/// A limit or seam failure that ends the walk early.
struct Stop {
    reason: Reason,
    message: Option<String>,
}

impl Stop {
    const fn limit(reason: Reason) -> Self {
        Self { reason, message: None }
    }

    fn failed(reason: Reason, err: &anyhow::Error) -> Self {
        Self { reason, message: Some(format!("{err:#}")) }
    }
}

struct Walk<'a, R, I> {
    refs: &'a R,
    index: &'a I,
    config: &'a Config,
    limits: &'a Limits,
    visited: BTreeMap<String, Visit>,
    /// Keyed by `(file, symbol)` so the first — shortest — path wins.
    impacted: BTreeMap<(String, Option<String>), ImpactedTest>,
    ignored: BTreeMap<String, IgnoredSymbol>,
    errors: Vec<String>,
    tyf_calls: usize,
    max_depth_reached: u32,
}

impl<R: Refs, I: Index> Walk<'_, R, I> {
    fn run(mut self, seeds: &[Seed]) -> Closure {
        let started = Instant::now();
        let stop = self.bfs(seeds);
        let (verdict, reason) = match stop {
            Some(stop) => {
                self.errors.extend(stop.message);
                (Verdict::RunAll, Some(stop.reason))
            }
            None => (Verdict::Selected, None),
        };

        Closure {
            verdict,
            reason,
            impacted: collapse_whole_files(self.impacted.into_values().collect()),
            ignored: self.ignored.into_values().collect(),
            errors: self.errors,
            stats: Stats {
                seeds: seeds.len(),
                visited: self.visited.len(),
                max_depth_reached: self.max_depth_reached,
                tyf_calls: self.tyf_calls,
                duration_ms: started.elapsed().as_millis(),
            },
        }
    }

    /// Level-by-level expansion. `Some(stop)` means the answer is incomplete.
    fn bfs(&mut self, seeds: &[Seed]) -> Option<Stop> {
        let mut frontier = match self.seed(seeds) {
            Ok(frontier) => frontier,
            Err(stop) => return Some(stop),
        };

        let mut depth = 0;
        loop {
            // Checked before *and* after every level, so a last frontier that
            // only adds module nodes — visited, never expanded — cannot blow
            // the symbol limit and still report `selected`.
            if let Err(stop) = self.check_limits() {
                return Some(stop);
            }
            if frontier.is_empty() {
                return None;
            }
            self.max_depth_reached = depth;
            frontier = match self.expand(&frontier) {
                Ok(next) => next,
                Err(stop) => return Some(stop),
            };
            if frontier.is_empty() {
                return self.check_limits().err();
            }
            depth += 1;
            if depth > self.limits.max_depth {
                return Some(Stop::limit(Reason::MaxDepth));
            }
        }
    }

    /// Checked between levels, not between sites: a single frontier runs to
    /// completion once started, so a very wide level can overshoot the budget
    /// by one round of `tyf` calls.
    fn check_limits(&self) -> Result<(), Stop> {
        if self.visited.len() > self.limits.max_symbols {
            return Err(Stop::limit(Reason::MaxSymbols));
        }
        if self.limits.deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err(Stop::limit(Reason::Budget));
        }
        Ok(())
    }

    /// Record every seed as visited, and turn the ones `tyf` can answer for
    /// into the first frontier.
    ///
    /// A [`SeedKind::Module`] seed never reaches `tyf`: a module is not a
    /// symbol. It selects the test files that import it and stops there.
    /// See `docs/adr/0007-module-level-selects-importers.md`.
    fn seed(&mut self, seeds: &[Seed]) -> Result<Vec<SymbolQuery>, Stop> {
        let mut frontier = Vec::new();

        for seed in seeds {
            let id = seed.query.id.clone();
            self.visited.insert(id.clone(), Visit { predecessor: None, origin: id.clone() });

            match seed.kind {
                SeedKind::Live => frontier.push(seed.query.clone()),
                SeedKind::Deleted => {
                    let sites = self.word_sites(&seed.query)?;
                    self.absorb(&seed.query, None, &sites, &mut frontier)?;
                }
                SeedKind::Module => self.select_importers(&id)?,
            }
        }

        self.check_limits()?;
        Ok(frontier)
    }

    /// Ask [`Refs`] about a whole frontier and absorb every answer.
    fn expand(&mut self, frontier: &[SymbolQuery]) -> Result<Vec<SymbolQuery>, Stop> {
        // Counted before the call, so a batch that fails still shows in `stats`.
        self.tyf_calls += 1;
        let answers =
            self.refs.refs(frontier).map_err(|err| Stop::failed(Reason::RefsFailed, &err))?;

        let by_id: BTreeMap<&str, &SymbolQuery> =
            frontier.iter().map(|q| (q.id.as_str(), q)).collect();

        let mut next = Vec::new();
        for answer in &answers {
            // An id nobody asked about means the seam and the walk disagree;
            // say so rather than losing the edge silently.
            let Some(query) = by_id.get(answer.id.as_str()) else {
                self.errors.push(format!("unexpected `refs` answer for `{}`", answer.id));
                continue;
            };
            self.absorb(query, Some(&query.declaration()), &answer.sites, &mut next)?;
        }
        next.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(next)
    }

    /// A deleted symbol has no definition left for `tyf` to resolve, so its
    /// references are approximated by a word-boundary scan for the bare name.
    ///
    /// This over-matches — comments, docstrings, same-named locals — and that
    /// is the intended trade: deletions are rare per commit and over-selection
    /// is the safe direction.
    fn word_sites(&self, query: &SymbolQuery) -> Result<Vec<RefSite>, Stop> {
        let hits = self
            .index
            .word_hits(query.bare_name())
            .map_err(|err| Stop::failed(Reason::IndexFailed, &err))?;
        Ok(hits.into_iter().map(|(file, line)| RefSite { file, line }).collect())
    }

    /// Classify every site one query produced.
    ///
    /// `declaration` is the definition's own position, which `tyf` reports
    /// alongside the genuine usages — a symbol is not its own caller. A deleted
    /// symbol passes `None`: there is no declaration left, and its sites come
    /// from a scan of the *current* tree, where that line is somebody else's
    /// code and dropping it would lose a real reference.
    fn absorb(
        &mut self,
        query: &SymbolQuery,
        declaration: Option<&RefSite>,
        sites: &[RefSite],
        next: &mut Vec<SymbolQuery>,
    ) -> Result<(), Stop> {
        for site in sites {
            if declaration == Some(site) {
                continue;
            }
            let class = self
                .index
                .classify(&site.file, site.line)
                .map_err(|err| Stop::failed(Reason::IndexFailed, &err))?;

            if self.index.is_test(&site.file) {
                self.test_site(&query.id, site, &class);
            } else {
                self.code_site(&query.id, site, &class, next)?;
            }
        }
        Ok(())
    }

    /// A reference inside test code: record it and stop. Tests are the answer,
    /// not a step towards it.
    fn test_site(&mut self, from: &str, site: &RefSite, class: &Site) {
        if matches!(class, Site::Import) {
            // The test's actual use of the symbol is its own reference; the
            // import line would otherwise select every test that imports the
            // module for any reason.
            return;
        }
        let symbol = match class {
            Site::Symbol(found) => {
                self.index.module_path(&site.file).map(|module| symbol_id(&module, &found.qualname))
            }
            // Module scope in a test file, or a file we could not parse: the
            // whole file is selected rather than one function.
            _ => None,
        };
        self.record(&site.file, symbol, from);
    }

    /// A reference in non-test code: expand it, unless it is noise.
    fn code_site(
        &mut self,
        from: &str,
        site: &RefSite,
        class: &Site,
        next: &mut Vec<SymbolQuery>,
    ) -> Result<(), Stop> {
        match class {
            Site::Import => Ok(()),
            Site::Unknown => {
                // Not Python we can attribute. Say so rather than pretending.
                self.errors.push(format!(
                    "could not read {}:{} while walking from `{from}`",
                    site.file.display(),
                    site.line
                ));
                Ok(())
            }
            Site::Symbol(found) => {
                self.symbol_site(from, site, found, next);
                Ok(())
            }
            Site::ModuleLevel => self.module_site(from, site),
        }
    }

    /// A reference inside a definition: the next node of the graph.
    fn symbol_site(
        &mut self,
        from: &str,
        site: &RefSite,
        found: &IndexedSymbol,
        next: &mut Vec<SymbolQuery>,
    ) {
        let Some(module) = self.index.module_path(&site.file) else { return };
        let id = symbol_id(&module, &found.qualname);

        if let Some(entry) = found.decorators.iter().find_map(|d| self.config.matching_decorator(d))
        {
            // A registry function is invoked by a runner, not by a caller, so
            // there is nothing to chase past it. Phase 1 made the same call.
            self.ignored.entry(id.clone()).or_insert_with(|| IgnoredSymbol {
                symbol: id,
                file: site.file.display().to_string(),
                ignored_by: entry.to_string(),
            });
            return;
        }

        if self.record_visit(&id, from) {
            next.push(SymbolQuery {
                id,
                name: found.qualname.clone(),
                file: site.file.clone(),
                line: found.line,
                column: found.column,
            });
        }
    }

    /// A reference at module scope that is not an import: code that runs when
    /// the module is imported.
    ///
    /// The module itself becomes the node, and only the tests that import it
    /// directly are selected. Transitive importers are a documented gap.
    fn module_site(&mut self, from: &str, site: &RefSite) -> Result<(), Stop> {
        let Some(module) = self.index.module_path(&site.file) else { return Ok(()) };
        if !self.record_visit(&module, from) {
            return Ok(());
        }
        self.select_importers(&module)
    }

    /// Select every test file that imports `module`, reached through the module
    /// node itself — which is why the chain shows the module and not a symbol.
    fn select_importers(&mut self, module: &str) -> Result<(), Stop> {
        let importers = self
            .index
            .test_importers(module)
            .map_err(|err| Stop::failed(Reason::IndexFailed, &err))?;
        for file in importers {
            self.record(&file, None, module);
        }
        Ok(())
    }

    /// First visit wins, which under BFS is the shortest path. Returns whether
    /// the node is new.
    fn record_visit(&mut self, id: &str, from: &str) -> bool {
        if self.visited.contains_key(id) {
            return false;
        }
        let origin = self.origin_of(from);
        self.visited.insert(id.to_string(), Visit { predecessor: Some(from.to_string()), origin });
        true
    }

    fn origin_of(&self, id: &str) -> String {
        self.visited.get(id).map_or_else(|| id.to_string(), |visit| visit.origin.clone())
    }

    /// Record an impacted test, keeping the first (shortest) route to it.
    fn record(&mut self, file: &Path, symbol: Option<String>, from: &str) {
        let (via, origin) = self.chain(from);
        let file = file.display().to_string();
        self.impacted.entry((file.clone(), symbol.clone())).or_insert(ImpactedTest {
            file,
            symbol,
            via,
            origin,
        });
    }

    /// The why-chain from `from` back to its origin, excluding both endpoints.
    ///
    /// Predecessors form a tree — every node is assigned one exactly once, on
    /// first visit — so this cannot loop.
    fn chain(&self, from: &str) -> (Vec<String>, String) {
        let origin = self.origin_of(from);
        let mut via = Vec::new();
        let mut current = from.to_string();

        while current != origin {
            via.push(current.clone());
            match self.visited.get(&current).and_then(|visit| visit.predecessor.clone()) {
                Some(predecessor) => current = predecessor,
                None => break,
            }
        }
        (via, origin)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::changed::{ModuleChange, SymbolChange};
    use crate::pysource::{self, Kind};
    use crate::workspace::is_test_path;

    /// An [`Index`] over a map of repository-relative path to source text.
    ///
    /// The classification is the real `pysource` one — only the filesystem is
    /// faked — so these tests exercise the same import and line attribution
    /// rules the binary uses.
    #[derive(Default)]
    struct MapIndex {
        files: BTreeMap<PathBuf, String>,
        fails: bool,
    }

    impl MapIndex {
        fn with(mut self, path: &str, source: &str) -> Self {
            self.files.insert(PathBuf::from(path), source.to_string());
            self
        }

        /// Every fallible method errors, standing in for a working tree that
        /// cannot be read part-way through a walk.
        const fn failing(mut self) -> Self {
            self.fails = true;
            self
        }

        fn check(&self) -> Result<()> {
            anyhow::ensure!(!self.fails, "the disk went away");
            Ok(())
        }

        fn parse(&self, file: &Path) -> Option<pysource::Module> {
            let source = self.files.get(file)?;
            pysource::parse(source).ok().filter(|module| !module.has_error)
        }
    }

    impl Index for MapIndex {
        fn classify(&self, file: &Path, line: u32) -> Result<Site> {
            self.check()?;
            let Some(module) = self.parse(file) else { return Ok(Site::Unknown) };
            Ok(classify_in(&module, line))
        }

        fn module_path(&self, file: &Path) -> Option<String> {
            let stem = file.with_extension("");
            let dotted = stem.to_str()?.trim_start_matches("src/").replace('/', ".");
            Some(dotted.trim_end_matches(".__init__").to_string())
        }

        fn is_test(&self, file: &Path) -> bool {
            is_test_path(file, Path::new("/repo"))
        }

        fn top_level(&self, file: &Path) -> Result<Vec<IndexedSymbol>> {
            self.check()?;
            let Some(module) = self.parse(file) else { return Ok(Vec::new()) };
            Ok(module.top_level().map(indexed).collect())
        }

        fn word_hits(&self, name: &str) -> Result<Vec<(PathBuf, u32)>> {
            self.check()?;
            let mut hits = Vec::new();
            for (path, source) in &self.files {
                hits.extend(
                    pysource::word_lines(source, name).into_iter().map(|line| (path.clone(), line)),
                );
            }
            Ok(hits)
        }

        fn test_importers(&self, module: &str) -> Result<Vec<PathBuf>> {
            self.check()?;
            Ok(self
                .files
                .iter()
                .filter(|(path, _)| self.is_test(path))
                .filter(|(_, source)| pysource::imports_module(source, module))
                .map(|(path, _)| path.clone())
                .collect())
        }
    }

    /// A [`Refs`] backed by a map from symbol id to reference sites.
    #[derive(Default)]
    struct MapRefs {
        answers: BTreeMap<String, Vec<(String, u32)>>,
        fails: bool,
    }

    impl MapRefs {
        fn with(mut self, id: &str, sites: &[(&str, u32)]) -> Self {
            self.answers.insert(
                id.to_string(),
                sites.iter().map(|(f, l)| ((*f).to_string(), *l)).collect(),
            );
            self
        }

        const fn failing() -> Self {
            Self { answers: BTreeMap::new(), fails: true }
        }
    }

    impl Refs for MapRefs {
        fn refs(&self, queries: &[SymbolQuery]) -> Result<Vec<RefAnswer>> {
            anyhow::ensure!(!self.fails, "ty server went away");
            Ok(queries
                .iter()
                .map(|query| RefAnswer {
                    id: query.id.clone(),
                    sites: self
                        .answers
                        .get(&query.id)
                        .map(|sites| {
                            sites
                                .iter()
                                .map(|(file, line)| RefSite {
                                    file: PathBuf::from(file),
                                    line: *line,
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect())
        }
    }

    fn seed(id: &str, file: &str, line: u32) -> Seed {
        let name = split_symbol_id(id).map_or(id, |(_, qual)| qual).to_string();
        Seed {
            query: SymbolQuery {
                id: id.to_string(),
                name,
                file: PathBuf::from(file),
                line,
                column: 5,
            },
            kind: SeedKind::Live,
        }
    }

    fn run(seeds: &[Seed], refs: &MapRefs, index: &MapIndex) -> Closure {
        walk(seeds, refs, index, &Config::default(), &Limits::default())
    }

    fn run_with(seeds: &[Seed], refs: &MapRefs, index: &MapIndex, config: &Config) -> Closure {
        walk(seeds, refs, index, config, &Limits::default())
    }

    /// `(file, symbol, via, origin)` for compact assertions.
    fn selected(closure: &Closure) -> Vec<(String, Option<String>, Vec<String>, String)> {
        closure
            .impacted
            .iter()
            .map(|t| (t.file.clone(), t.symbol.clone(), t.via.clone(), t.origin.clone()))
            .collect()
    }

    const SERVICE: &str = "\
from pkg.core import target


def middle():
    return target()
";

    const TEST_MIDDLE: &str = "\
from pkg.service import middle


def test_middle():
    assert middle() == 1
";

    #[test]
    fn a_test_referencing_the_changed_symbol_directly_has_an_empty_chain() {
        let index =
            MapIndex::default().with("src/pkg/core.py", "def target():\n    return 1\n").with(
                "tests/test_core.py",
                "from pkg.core import target\n\n\ndef test_target():\n    assert target()\n",
            );
        let refs = MapRefs::default().with("pkg.core:target", &[("tests/test_core.py", 5)]);

        let closure = run(&[seed("pkg.core:target", "src/pkg/core.py", 1)], &refs, &index);

        assert_eq!(closure.verdict, Verdict::Selected, "the frontier emptied cleanly");
        assert_eq!(
            selected(&closure),
            vec![(
                "tests/test_core.py".to_string(),
                Some("tests.test_core:test_target".to_string()),
                vec![],
                "pkg.core:target".to_string()
            )],
            "an empty `via` is what a direct reference looks like"
        );
    }

    #[test]
    fn a_two_hop_chain_records_the_intermediate_symbol() {
        let index = MapIndex::default()
            .with("src/pkg/core.py", "def target():\n    return 1\n")
            .with("src/pkg/service.py", SERVICE)
            .with("tests/test_service.py", TEST_MIDDLE);
        let refs = MapRefs::default()
            .with("pkg.core:target", &[("src/pkg/service.py", 5)])
            .with("pkg.service:middle", &[("tests/test_service.py", 5)]);

        let closure = run(&[seed("pkg.core:target", "src/pkg/core.py", 1)], &refs, &index);

        assert_eq!(
            selected(&closure),
            vec![(
                "tests/test_service.py".to_string(),
                Some("tests.test_service:test_middle".to_string()),
                vec!["pkg.service:middle".to_string()],
                "pkg.core:target".to_string()
            )],
            "`via` names the hop between the test and the changed symbol"
        );
        assert_eq!(closure.stats.max_depth_reached, 1, "two levels were expanded");
        assert_eq!(closure.stats.visited, 2, "the seed and the intermediate symbol");
    }

    #[test]
    fn a_diamond_keeps_the_shorter_route() {
        // target is reached by the test directly *and* through `middle`.
        let index = MapIndex::default()
            .with("src/pkg/core.py", "def target():\n    return 1\n")
            .with("src/pkg/service.py", SERVICE)
            .with(
                "tests/test_both.py",
                "from pkg.core import target\nfrom pkg.service import middle\n\n\ndef test_both():\n    assert target() == middle()\n",
            );
        let refs = MapRefs::default()
            .with("pkg.core:target", &[("src/pkg/service.py", 5), ("tests/test_both.py", 6)])
            .with("pkg.service:middle", &[("tests/test_both.py", 6)]);

        let closure = run(&[seed("pkg.core:target", "src/pkg/core.py", 1)], &refs, &index);

        assert_eq!(closure.impacted.len(), 1, "one test, however many routes reach it");
        assert!(
            closure.impacted[0].via.is_empty(),
            "BFS finds the direct edge first, got {:?}",
            closure.impacted[0].via
        );
    }

    #[test]
    fn a_cycle_terminates() {
        let index = MapIndex::default()
            .with("src/pkg/a.py", "def a():\n    return b()\n")
            .with("src/pkg/b.py", "def b():\n    return a()\n");
        // a references b, b references a — a genuine cycle in the call graph.
        let refs = MapRefs::default()
            .with("pkg.a:a", &[("src/pkg/b.py", 2)])
            .with("pkg.b:b", &[("src/pkg/a.py", 2)]);

        let closure = run(&[seed("pkg.a:a", "src/pkg/a.py", 1)], &refs, &index);

        assert_eq!(closure.verdict, Verdict::Selected, "a cycle must not trip a limit");
        assert_eq!(closure.stats.visited, 2, "each node is visited once");
        assert!(closure.impacted.is_empty(), "no test is reachable here");
    }

    #[test]
    fn a_self_reference_does_not_re_enter_the_frontier() {
        let index = MapIndex::default().with("src/pkg/a.py", "def a(n):\n    return a(n - 1)\n");
        // Line 1 is the definition itself; line 2 is the recursive call.
        let refs = MapRefs::default().with("pkg.a:a", &[("src/pkg/a.py", 1), ("src/pkg/a.py", 2)]);

        let closure = run(&[seed("pkg.a:a", "src/pkg/a.py", 1)], &refs, &index);

        assert_eq!(closure.stats.visited, 1, "recursion adds no node");
        assert_eq!(closure.verdict, Verdict::Selected);
    }

    #[test]
    fn an_import_line_is_dropped_rather_than_expanded() {
        let index = MapIndex::default()
            .with("src/pkg/core.py", "def target():\n    return 1\n")
            .with("src/pkg/service.py", SERVICE);
        // Line 1 of service.py is the import of `target`.
        let refs = MapRefs::default().with("pkg.core:target", &[("src/pkg/service.py", 1)]);

        let closure = run(&[seed("pkg.core:target", "src/pkg/core.py", 1)], &refs, &index);

        assert_eq!(closure.stats.visited, 1, "only the seed; the import line adds nothing");
        assert!(closure.impacted.is_empty(), "and selects no test on its own");
    }

    #[test]
    fn an_import_in_an_init_file_is_dropped_too() {
        // A re-export. Dropping it is deliberate: consumers of the re-exported
        // name resolve back to the real definition and appear as ordinary refs.
        let index = MapIndex::default()
            .with("src/pkg/core.py", "def target():\n    return 1\n")
            .with("src/pkg/__init__.py", "from pkg.core import target\n");
        let refs = MapRefs::default().with("pkg.core:target", &[("src/pkg/__init__.py", 1)]);

        let closure = run(&[seed("pkg.core:target", "src/pkg/core.py", 1)], &refs, &index);

        assert_eq!(closure.stats.visited, 1, "the package __init__ is not a caller");
    }

    #[test]
    fn an_import_line_inside_a_test_file_is_dropped_as_well() {
        // Otherwise every test importing the module would be selected, which is
        // exactly the precision the import rule buys.
        let index =
            MapIndex::default().with("src/pkg/core.py", "def target():\n    return 1\n").with(
                "tests/test_core.py",
                "from pkg.core import target\n\n\ndef test_other():\n    assert True\n",
            );
        let refs = MapRefs::default().with("pkg.core:target", &[("tests/test_core.py", 1)]);

        let closure = run(&[seed("pkg.core:target", "src/pkg/core.py", 1)], &refs, &index);

        assert!(closure.impacted.is_empty(), "the import alone selects nothing");
    }

    #[test]
    fn a_symbol_with_an_ignored_decorator_is_a_dead_end() {
        let index =
            MapIndex::default().with("src/pkg/core.py", "def target():\n    return 1\n").with(
                "src/pkg/jobs.py",
                "@registry.transformation\ndef nightly(df):\n    return target(df)\n",
            );
        let refs = MapRefs::default().with("pkg.core:target", &[("src/pkg/jobs.py", 3)]);
        let config =
            Config { ignore_decorators: vec!["transformation".to_string()], ..Config::default() };

        let closure =
            run_with(&[seed("pkg.core:target", "src/pkg/core.py", 1)], &refs, &index, &config);

        assert_eq!(closure.stats.visited, 1, "a registry function is not expanded");
        assert_eq!(
            closure.ignored,
            vec![IgnoredSymbol {
                symbol: "pkg.jobs:nightly".to_string(),
                file: "src/pkg/jobs.py".to_string(),
                ignored_by: "transformation".to_string(),
            }],
            "but it is reported, with the entry that matched"
        );
    }

    #[test]
    fn a_module_level_reference_selects_the_tests_that_import_the_module() {
        let index = MapIndex::default()
            .with("src/pkg/core.py", "def target():\n    return 1\n")
            .with("src/pkg/settings.py", "from pkg.core import target\n\nVALUE = target()\n")
            .with(
                "tests/test_settings.py",
                "from pkg import settings\n\n\ndef test_value():\n    assert settings.VALUE\n",
            )
            .with("tests/test_other.py", "def test_other():\n    assert True\n");
        let refs = MapRefs::default().with("pkg.core:target", &[("src/pkg/settings.py", 3)]);

        let closure = run(&[seed("pkg.core:target", "src/pkg/core.py", 1)], &refs, &index);

        assert_eq!(
            selected(&closure),
            vec![(
                "tests/test_settings.py".to_string(),
                None,
                vec!["pkg.settings".to_string()],
                "pkg.core:target".to_string()
            )],
            "a module-level call selects the whole importing test file, via the module"
        );
    }

    #[test]
    fn a_deleted_symbol_falls_back_to_a_textual_scan() {
        let index = MapIndex::default()
            .with("src/pkg/service.py", "def middle():\n    return target()\n")
            .with(
                "tests/test_service.py",
                "from pkg.service import middle\n\n\ndef test_middle():\n    assert middle()\n",
            );
        // `target` is gone, so nothing answers a refs query for it.
        let refs = MapRefs::default().with("pkg.service:middle", &[("tests/test_service.py", 5)]);
        let seeds = vec![Seed {
            query: SymbolQuery {
                id: "pkg.core:target".to_string(),
                name: "target".to_string(),
                file: PathBuf::from("src/pkg/core.py"),
                line: 1,
                column: 5,
            },
            kind: SeedKind::Deleted,
        }];

        let closure = run(&seeds, &refs, &index);

        assert_eq!(
            selected(&closure),
            vec![(
                "tests/test_service.py".to_string(),
                Some("tests.test_service:test_middle".to_string()),
                vec!["pkg.service:middle".to_string()],
                "pkg.core:target".to_string()
            )],
            "the word scan finds the orphaned call site, and the walk continues from it"
        );
    }

    #[test]
    fn a_deleted_symbol_keeps_a_hit_on_its_own_old_line() {
        // The self-reference skip exists because `tyf` reports a symbol's own
        // declaration. A deleted symbol has no declaration left, and its sites
        // come from a scan of the *current* tree — so the line the definition
        // used to be on is now somebody else's code.
        let index = MapIndex::default()
            .with("src/pkg/core.py", "def middle():\n    return target()\n")
            .with(
                "tests/test_core.py",
                "from pkg.core import middle\n\n\ndef test_middle():\n    assert middle()\n",
            );
        let refs = MapRefs::default().with("pkg.core:middle", &[("tests/test_core.py", 5)]);
        let seeds = vec![Seed {
            query: SymbolQuery {
                id: "pkg.core:target".to_string(),
                name: "target".to_string(),
                file: PathBuf::from("src/pkg/core.py"),
                // Where `def target():` used to be — now the call site itself.
                line: 2,
                column: 5,
            },
            kind: SeedKind::Deleted,
        }];

        let closure = run(&seeds, &refs, &index);

        assert_eq!(
            selected(&closure),
            vec![(
                "tests/test_core.py".to_string(),
                Some("tests.test_core:test_middle".to_string()),
                vec!["pkg.core:middle".to_string()],
                "pkg.core:target".to_string()
            )],
            "a collision with the old definition's line must not drop the reference"
        );
    }

    #[test]
    fn a_working_tree_that_cannot_be_read_degrades_to_run_all() {
        let index = MapIndex::default()
            .with("src/pkg/service.py", "def caller():\n    return target()\n")
            .failing();
        let refs = MapRefs::default().with("pkg.core:target", &[("src/pkg/service.py", 2)]);

        let closure = run(&[seed("pkg.core:target", "src/pkg/core.py", 1)], &refs, &index);

        assert_eq!(closure.verdict, Verdict::RunAll, "an unreadable tree is still an answer");
        assert_eq!(closure.reason, Some(Reason::IndexFailed));
        assert!(
            closure.errors.iter().any(|e| e.contains("disk went away")),
            "and the seam's own message survives: {:?}",
            closure.errors
        );
    }

    #[test]
    fn the_depth_limit_produces_a_run_all_verdict() {
        let index = MapIndex::default()
            .with("src/pkg/a.py", "def a():\n    return 1\n")
            .with("src/pkg/b.py", "def b():\n    return a()\n")
            .with("src/pkg/c.py", "def c():\n    return b()\n");
        let refs = MapRefs::default()
            .with("pkg.a:a", &[("src/pkg/b.py", 2)])
            .with("pkg.b:b", &[("src/pkg/c.py", 2)]);
        let limits = Limits { max_depth: 1, ..Limits::default() };

        let closure =
            walk(&[seed("pkg.a:a", "src/pkg/a.py", 1)], &refs, &index, &Config::default(), &limits);

        assert_eq!(closure.verdict, Verdict::RunAll, "an incomplete walk cannot be trusted");
        assert_eq!(closure.reason, Some(Reason::MaxDepth), "and it says which limit tripped");
    }

    #[test]
    fn the_symbol_limit_produces_a_run_all_verdict() {
        let index = MapIndex::default()
            .with("src/pkg/a.py", "def a():\n    return 1\n")
            .with("src/pkg/b.py", "def b():\n    return a()\n");
        let refs = MapRefs::default().with("pkg.a:a", &[("src/pkg/b.py", 2)]);
        let limits = Limits { max_symbols: 1, ..Limits::default() };

        let closure =
            walk(&[seed("pkg.a:a", "src/pkg/a.py", 1)], &refs, &index, &Config::default(), &limits);

        assert_eq!(closure.reason, Some(Reason::MaxSymbols), "two nodes exceeded a budget of one");
    }

    #[test]
    fn an_expired_budget_produces_a_run_all_verdict() {
        let index = MapIndex::default()
            .with("src/pkg/a.py", "def a():\n    return 1\n")
            .with("src/pkg/b.py", "def b():\n    return a()\n");
        let refs = MapRefs::default().with("pkg.a:a", &[("src/pkg/b.py", 2)]);
        let limits = Limits { deadline: Some(Instant::now()), ..Limits::default() };

        let closure =
            walk(&[seed("pkg.a:a", "src/pkg/a.py", 1)], &refs, &index, &Config::default(), &limits);

        assert_eq!(closure.reason, Some(Reason::Budget), "a deadline in the past trips at once");
    }

    #[test]
    fn a_failing_refs_seam_degrades_instead_of_crashing() {
        let index = MapIndex::default().with("src/pkg/a.py", "def a():\n    return 1\n");
        let closure = run(&[seed("pkg.a:a", "src/pkg/a.py", 1)], &MapRefs::failing(), &index);

        assert_eq!(closure.verdict, Verdict::RunAll, "run everything beats failing the hook");
        assert_eq!(closure.reason, Some(Reason::RefsFailed));
        assert!(
            closure.errors.iter().any(|e| e.contains("ty server went away")),
            "the underlying error must survive, got {:?}",
            closure.errors
        );
    }

    #[test]
    fn a_reference_in_an_unreadable_file_is_recorded_but_not_fatal() {
        let index = MapIndex::default().with("src/pkg/a.py", "def a():\n    return 1\n");
        // No entry for vendor/thing.py, so the index cannot classify it.
        let refs = MapRefs::default().with("pkg.a:a", &[("vendor/thing.py", 12)]);

        let closure = run(&[seed("pkg.a:a", "src/pkg/a.py", 1)], &refs, &index);

        assert_eq!(closure.verdict, Verdict::Selected, "one opaque file does not void the walk");
        assert_eq!(closure.errors.len(), 1, "but it is reported, got {:?}", closure.errors);
        assert!(closure.errors[0].contains("vendor/thing.py:12"), "naming the site");
    }

    #[test]
    fn an_unparseable_test_file_selects_the_whole_file() {
        // Being unable to find the enclosing test is not a reason to skip it.
        let index = MapIndex::default()
            .with("src/pkg/a.py", "def a():\n    return 1\n")
            .with("tests/test_broken.py", "def test_x(:\n    pass\n");
        let refs = MapRefs::default().with("pkg.a:a", &[("tests/test_broken.py", 2)]);

        let closure = run(&[seed("pkg.a:a", "src/pkg/a.py", 1)], &refs, &index);

        assert_eq!(
            selected(&closure),
            vec![("tests/test_broken.py".to_string(), None, vec![], "pkg.a:a".to_string())],
            "a null symbol means the whole file"
        );
    }

    #[test]
    fn a_whole_file_selection_supersedes_its_own_test_functions() {
        // Real repositories hit this constantly: an `if __name__` block makes a
        // module a node, and a test file both imports that module and calls the
        // changed symbol from one of its tests.
        let index = MapIndex::default()
            .with("src/pkg/a.py", "def a():\n    return 1\n")
            .with("src/pkg/entry.py", "from pkg.a import a\n\na()\n")
            .with(
                "tests/test_entry.py",
                "from pkg import entry\nfrom pkg.a import a\n\n\ndef test_a():\n    assert a()\n",
            );
        let refs = MapRefs::default()
            .with("pkg.a:a", &[("src/pkg/entry.py", 3), ("tests/test_entry.py", 6)]);

        let closure = run(&[seed("pkg.a:a", "src/pkg/a.py", 1)], &refs, &index);

        assert_eq!(
            selected(&closure),
            vec![(
                "tests/test_entry.py".to_string(),
                None,
                vec!["pkg.entry".to_string()],
                "pkg.a:a".to_string()
            )],
            "the whole file already covers the test function, so it is listed once"
        );
    }

    #[test]
    fn a_file_with_no_whole_file_selection_keeps_every_test() {
        let index = MapIndex::default()
            .with("src/pkg/a.py", "def a():\n    return 1\n")
            .with(
                "tests/test_a.py",
                "from pkg.a import a\n\n\ndef test_one():\n    assert a()\n\n\ndef test_two():\n    assert a()\n",
            );
        let refs =
            MapRefs::default().with("pkg.a:a", &[("tests/test_a.py", 5), ("tests/test_a.py", 9)]);

        let closure = run(&[seed("pkg.a:a", "src/pkg/a.py", 1)], &refs, &index);
        assert_eq!(closure.impacted.len(), 2, "both tests stand on their own");
    }

    #[test]
    fn output_is_sorted_and_deterministic() {
        let index = MapIndex::default()
            .with("src/pkg/a.py", "def a():\n    return 1\n")
            .with("tests/test_b.py", "from pkg.a import a\n\n\ndef test_b():\n    assert a()\n")
            .with("tests/test_a.py", "from pkg.a import a\n\n\ndef test_a():\n    assert a()\n");
        let refs =
            MapRefs::default().with("pkg.a:a", &[("tests/test_b.py", 5), ("tests/test_a.py", 5)]);

        let closure = run(&[seed("pkg.a:a", "src/pkg/a.py", 1)], &refs, &index);
        let files: Vec<&str> = closure.impacted.iter().map(|t| t.file.as_str()).collect();

        assert_eq!(
            files,
            vec!["tests/test_a.py", "tests/test_b.py"],
            "callers diff this output; the refs order must not leak into it"
        );
    }

    #[test]
    fn an_empty_seed_set_selects_nothing_and_still_succeeds() {
        let closure = run(&[], &MapRefs::default(), &MapIndex::default());
        assert_eq!(closure.verdict, Verdict::Selected, "no change means no tests, not a failure");
        assert!(closure.impacted.is_empty());
        assert_eq!(closure.stats.seeds, 0);
    }

    fn changed(symbol: &str, file: &str, line: u32, change: Change) -> SymbolChange {
        SymbolChange {
            symbol: symbol.to_string(),
            file: file.to_string(),
            kind: Kind::Function,
            line,
            column: 5,
            change,
            ignored_by: None,
        }
    }

    #[test]
    fn seeds_come_from_changed_symbols_and_never_from_ignored_ones() {
        let report = ChangedSymbols {
            changed_symbols: vec![changed("pkg.a:f", "src/pkg/a.py", 3, Change::Modified)],
            ignored_symbols: vec![changed("pkg.a:registered", "src/pkg/a.py", 9, Change::Modified)],
            ..ChangedSymbols::default()
        };

        let seeds = seeds_from(&report, &MapIndex::default()).expect("no index access needed");
        let ids: Vec<&str> = seeds.iter().map(|s| s.query.id.as_str()).collect();

        assert_eq!(
            ids,
            vec!["pkg.a:f"],
            "phase 1 already decided the ignored one is not worth chasing"
        );
        assert_eq!(seeds[0].kind, SeedKind::Live);
        assert_eq!(seeds[0].query.name, "f", "the query drops the module prefix");
        assert_eq!(seeds[0].query.line, 3, "and carries the definition line");
    }

    #[test]
    fn a_deleted_symbol_seeds_as_deleted() {
        let report = ChangedSymbols {
            changed_symbols: vec![changed("pkg.a:gone", "src/pkg/a.py", 3, Change::Deleted)],
            ..ChangedSymbols::default()
        };
        let seeds = seeds_from(&report, &MapIndex::default()).expect("seeding cannot fail");
        assert_eq!(seeds[0].kind, SeedKind::Deleted, "there is no definition left to resolve");
    }

    #[test]
    fn a_module_level_change_seeds_the_module_and_its_definitions() {
        let index = MapIndex::default().with(
            "src/pkg/a.py",
            "import os\n\n\ndef f():\n    return 1\n\n\nclass C:\n    def m(self):\n        return 2\n",
        );
        let report = ChangedSymbols {
            module_level_changes: vec![ModuleChange {
                module: "pkg.a".to_string(),
                file: "src/pkg/a.py".to_string(),
            }],
            ..ChangedSymbols::default()
        };

        let seeds = seeds_from(&report, &index).expect("the index has the file");
        let ids: Vec<&str> = seeds.iter().map(|s| s.query.id.as_str()).collect();

        assert_eq!(
            ids,
            vec!["pkg.a", "pkg.a:C", "pkg.a:f"],
            "the module itself plus its module-scope definitions; methods are reached through C"
        );
        assert_eq!(seeds[0].kind, SeedKind::Module, "the bare module id is the module seed");
    }

    #[test]
    fn a_module_level_seed_selects_the_tests_that_import_it() {
        let index = MapIndex::default().with("src/pkg/a.py", "import os\n\nVALUE = 1\n").with(
            "tests/test_a.py",
            "from pkg.a import VALUE\n\n\ndef test_value():\n    assert VALUE\n",
        );
        let report = ChangedSymbols {
            module_level_changes: vec![ModuleChange {
                module: "pkg.a".to_string(),
                file: "src/pkg/a.py".to_string(),
            }],
            ..ChangedSymbols::default()
        };
        let seeds = seeds_from(&report, &index).expect("seeding works");

        let closure = run(&seeds, &MapRefs::default(), &index);

        assert_eq!(
            selected(&closure),
            vec![("tests/test_a.py".to_string(), None, vec![], "pkg.a".to_string())],
            "a changed constant has no definition to walk from, so the importers are the answer"
        );
    }

    #[test]
    fn a_seed_is_never_listed_twice() {
        let index = MapIndex::default().with("src/pkg/a.py", "def f():\n    return 1\n");
        let report = ChangedSymbols {
            changed_symbols: vec![changed("pkg.a:f", "src/pkg/a.py", 1, Change::Modified)],
            module_level_changes: vec![ModuleChange {
                module: "pkg.a".to_string(),
                file: "src/pkg/a.py".to_string(),
            }],
            ..ChangedSymbols::default()
        };

        let seeds = seeds_from(&report, &index).expect("seeding works");
        let ids: Vec<&str> = seeds.iter().map(|s| s.query.id.as_str()).collect();
        assert_eq!(ids, vec!["pkg.a", "pkg.a:f"], "the symbol is both changed and module-scope");
    }
}
