//! Turning an [`ImpactReport`] into the pytest node ids that express it.
//!
//! Three things happen here, in this order:
//!
//! 1. **Collectibility.** `impacted_tests[].symbol` is the *enclosing* symbol of
//!    a reference, which is frequently a helper or a fixture rather than a test.
//!    Handing pytest a non-collectible node id is a usage error that fails the
//!    whole run, so each qualified name is checked segment by segment against
//!    pytest's default conventions and trimmed — or widened to the whole file —
//!    until it is one pytest will accept.
//! 2. **Fixture expansion.** A reference from inside a fixture body dead-ends at
//!    the fixture, and a fixture's own file (`conftest.py`) collects no tests at
//!    all. [`crate::fixtures`] resolves the name edge pytest would have made.
//! 3. **Collapse.** A whole-file selection supersedes every per-test node id in
//!    that file — re-run after expansion, because expansion can introduce new
//!    whole-file entries.
//!
//! Every degrade is towards *more* tests, never fewer: the one failure mode
//! this command must not have is a confident selection that misses a test.
//!
//! The module is pure. It reaches the working tree through [`Tree`], so the
//! whole mapping is unit-testable from a `HashMap` — no `tyf`, no `git`, no
//! pytest. See `docs/adr/0001-two-impure-seams.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use serde::Serialize;

use crate::closure::{Reason, Verdict};
use crate::fixtures::{collects_tests, is_conftest, Collect, Fixture, FixtureMap};
use crate::impact::ImpactReport;
use crate::modpath::split_symbol_id;
use crate::pysource::Module;

/// "What does the working tree hold?"
///
/// A subset of [`crate::closure::Index`]: the selection needs whole parsed test
/// modules, which the walk never does.
pub trait Tree {
    /// Repository-relative paths of every Python file under a test path.
    fn test_files(&self) -> Vec<PathBuf>;
    /// The parsed module for a file: `None` when it is missing or unparseable.
    fn module(&self, file: &Path) -> Option<Rc<Module>>;
    /// Whether the working tree currently holds this path.
    fn exists(&self, file: &Path) -> bool;
    /// Dotted module path for a repository-relative `.py` path.
    fn module_path(&self, file: &Path) -> Option<String>;
}

/// What gerenuk is going to do about the report.
///
/// The three-way outcome an argument list cannot express: an empty pytest argv
/// *is* "run everything", which is why gerenuk owns the invocation rather than
/// printing node ids for a shell to interpolate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Run the listed node ids.
    Selected,
    /// Run the whole suite — the report says the selection cannot be trusted.
    RunAll,
    /// Run nothing at all. Nothing was impacted, and that is a green result.
    Nothing,
}

/// One pytest node id, and the chain that reached it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Selected {
    /// `tests/test_x.py::TestFoo::test_bar`, or just the file.
    pub node_id: String,
    /// Symbols between the test and [`Self::origin`], nearest the test first.
    pub via: Vec<String>,
    /// The changed symbol it was reached from; `null` for a changed test file,
    /// which selects itself.
    pub origin: Option<String>,
}

/// Why an entry became something other than its own node id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpansionKind {
    /// A fixture's consumers, resolved by name.
    Fixture,
    /// A fixture whose injected name could not be read: its whole scope.
    UnresolvableFixture,
    /// A `conftest.py` executes at collection time for its whole subtree.
    ConftestSubtree,
}

impl ExpansionKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Fixture => "fixture",
            Self::UnresolvableFixture => "fixture with an unreadable name",
            Self::ConftestSubtree => "conftest subtree",
        }
    }
}

/// One entry that turned into a set of others.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Expansion {
    /// The symbol or module id that was expanded.
    pub from: String,
    /// What kind of edge the expansion followed.
    pub kind: ExpansionKind,
    /// The node ids it produced.
    pub into: Vec<String>,
}

/// Why an entry produced no node id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DropReason {
    /// The working tree no longer has the file.
    Deleted,
    /// The file is selected wholesale, so a node id inside it is redundant.
    Superseded,
    /// pytest collects nothing from this path — a package marker, a helper
    /// module, a `conftest.py` handed over as if it were a test.
    NotCollectible,
    /// A fixture nothing in its scope consumes.
    NoConsumers,
}

impl DropReason {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Deleted => "no longer in the working tree",
            Self::Superseded => "covered by the whole file",
            Self::NotCollectible => "pytest collects no tests from it",
            Self::NoConsumers => "no test in scope consumes it",
        }
    }
}

/// One entry that produced no node id, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Dropped {
    /// The node id, file or symbol id that did not survive.
    pub entry: String,
    pub why: DropReason,
}

/// The whole answer for one `gerenuk run` selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Selection {
    /// The impact report's verdict, carried through unchanged.
    pub verdict: Verdict,
    /// Why the verdict is `run_all`; `null` when it is `selected`.
    pub reason: Option<Reason>,
    pub decision: Decision,
    pub node_ids: Vec<Selected>,
    pub expanded: Vec<Expansion>,
    pub dropped: Vec<Dropped>,
}

impl Selection {
    /// The node ids, in the order pytest will be given them.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.node_ids.iter().map(|selected| selected.node_id.as_str())
    }

    /// How many distinct changed symbols the selection traces back to.
    #[must_use]
    pub fn origins(&self) -> usize {
        self.node_ids
            .iter()
            .filter_map(|selected| selected.origin.as_deref())
            .collect::<BTreeSet<_>>()
            .len()
    }
}

/// Map a report onto node ids, expanding fixtures and collapsing whole files.
#[must_use]
pub fn select(report: &ImpactReport, tree: &impl Tree) -> Selection {
    if report.verdict == Verdict::RunAll {
        // Nothing is worth mapping: the whole suite runs either way, and a
        // half-trustworthy list of node ids would only mislead.
        return Selection {
            verdict: report.verdict,
            reason: report.reason,
            decision: Decision::RunAll,
            node_ids: Vec::new(),
            expanded: Vec::new(),
            dropped: Vec::new(),
        };
    }

    // Paths now, facts on demand. A report about one test must not cost a
    // parse of every test file in the repository — the same reason
    // `impacted-tests` never lists the tree it does not need.
    let map = FixtureMap::paths(tree.test_files());

    Mapper { tree, map, selected: BTreeMap::new(), expanded: Vec::new(), dropped: Vec::new() }
        .run(report)
}

/// The chain that reached an entry, as the JSON splits it.
#[derive(Debug, Clone, Default)]
struct Chain {
    via: Vec<String>,
    origin: Option<String>,
}

impl Chain {
    /// The same chain with one more symbol nearest the test.
    ///
    /// This is what keeps a fixture expansion auditable: the fixture's own
    /// symbol id is a real id a human can feed straight to `tyf refs`.
    fn under(&self, symbol: &str) -> Self {
        let mut via = vec![symbol.to_string()];
        via.extend(self.via.iter().cloned());
        Self { via, origin: self.origin.clone() }
    }
}

struct Mapper<'a, T> {
    tree: &'a T,
    map: FixtureMap,
    /// Keyed by node id so the first — shortest — chain to it wins.
    selected: BTreeMap<String, Selected>,
    expanded: Vec<Expansion>,
    dropped: Vec<Dropped>,
}

impl<T: Tree> Mapper<'_, T> {
    fn run(mut self, report: &ImpactReport) -> Selection {
        for test in &report.impacted_tests {
            let chain = Chain { via: test.via.clone(), origin: Some(test.origin.clone()) };
            let file = PathBuf::from(&test.file);
            match &test.symbol {
                Some(symbol) => self.symbol_entry(&file, symbol, &chain),
                None => self.whole_file(&file, &chain),
            }
        }

        // A changed test selects itself and needs no chain: there is no symbol
        // between the change and the test, because the test *is* the change.
        for file in &report.test_files_changed {
            self.whole_file(&PathBuf::from(file), &Chain::default());
        }

        let node_ids = self.collapse();
        let decision = if node_ids.is_empty() { Decision::Nothing } else { Decision::Selected };

        Selection {
            verdict: report.verdict,
            reason: report.reason,
            decision,
            node_ids,
            expanded: self.expanded,
            dropped: self.dropped,
        }
    }

    /// One `impacted_tests` entry that named a symbol.
    fn symbol_entry(&mut self, file: &Path, symbol: &str, chain: &Chain) {
        if !self.tree.exists(file) {
            self.drop_entry(symbol, DropReason::Deleted);
            return;
        }
        // A module id has no colon; that shape only reaches here for a test
        // file, where the whole file is the selection (ADR 0008).
        let Some((_, qualname)) = split_symbol_id(symbol) else {
            self.whole_file(file, chain);
            return;
        };
        self.load(file);

        // The name edge first: pytest injects a fixture rather than collecting
        // it, so a fixture is never a node id however collectible its name
        // looks.
        if let Some(fixture) = self.map.fixture_at(file, qualname).cloned() {
            self.expand_fixture(&fixture, symbol, chain);
            return;
        }
        // Only a file pytest collects tests from can yield a `file::name` node
        // id at all; in a `conftest.py` or a helper module the file — or what
        // it configures — is the honest answer.
        if !collects_tests(file) {
            self.whole_file(file, chain);
            return;
        }

        match self.map.collect(file, qualname) {
            Collect::All => self.push(node_id(file, qualname), chain),
            Collect::Prefix(prefix) => self.push(node_id(file, &prefix), chain),
            // A helper nested somewhere pytest cannot name: the whole file.
            Collect::None => self.whole_file(file, chain),
        }
    }

    /// Read one file's facts into the map, if they are not there already.
    ///
    /// [`FixtureMap::paths`] hands out a map that knows every test file's path
    /// and none of their contents, so every query that reads facts goes through
    /// here or [`Self::load_scope`] first.
    fn load(&mut self, file: &Path) {
        if self.map.is_loaded(file) {
            return;
        }
        let module = self.tree.module(file);
        self.map.load(file, module.as_deref());
    }

    /// Load everything a fixture's consumer resolution reads: every file it
    /// could be injected into, and the `conftest.py` chain above each of them
    /// that decides which definition of the name wins.
    fn load_scope(&mut self, fixture: &Fixture) {
        for file in self.map.scope_of(fixture) {
            for conftest in self.map.conftests_above(&file) {
                self.load(&conftest);
            }
            self.load(&file);
        }
    }

    /// Select a whole file — or, for a `conftest.py`, everything it configures.
    fn whole_file(&mut self, file: &Path, chain: &Chain) {
        if !self.tree.exists(file) {
            self.drop_entry(&file.display().to_string(), DropReason::Deleted);
            return;
        }
        if is_conftest(file) {
            self.expand_conftest(file, chain);
            return;
        }
        if collects_tests(file) {
            self.push(file.display().to_string(), chain);
        } else {
            self.drop_entry(&file.display().to_string(), DropReason::NotCollectible);
        }
    }

    /// A `conftest.py` executes at collection time for its whole subtree, so a
    /// change anywhere in it can alter any test under that directory.
    fn expand_conftest(&mut self, file: &Path, chain: &Chain) {
        let module = self.tree.module_path(file).unwrap_or_else(|| file.display().to_string());
        let under = chain.under(&module);
        let files = self.map.subtree(file.parent().unwrap_or_else(|| Path::new("")));

        if files.is_empty() {
            self.drop_entry(&module, DropReason::NotCollectible);
            return;
        }
        let into: Vec<String> = files.iter().map(|file| file.display().to_string()).collect();
        for node_id in &into {
            self.push(node_id.clone(), &under);
        }
        self.expanded.push(Expansion { from: module, kind: ExpansionKind::ConftestSubtree, into });
    }

    /// The tests a fixture reaches, resolved by name because `tyf` cannot.
    fn expand_fixture(&mut self, fixture: &Fixture, symbol: &str, chain: &Chain) {
        self.load_scope(fixture);
        let consumers = self.map.consumers(fixture);
        if consumers.is_empty() {
            self.drop_entry(symbol, DropReason::NoConsumers);
            return;
        }

        let under = chain.under(symbol);
        let mut into: Vec<String> =
            consumers.tests.iter().map(|test| node_id(&test.file, &test.qualname)).collect();
        into.extend(consumers.files.iter().map(|file| file.display().to_string()));

        for node_id in &into {
            self.push(node_id.clone(), &under);
        }
        let kind = if fixture.unresolvable {
            ExpansionKind::UnresolvableFixture
        } else {
            ExpansionKind::Fixture
        };
        self.expanded.push(Expansion { from: symbol.to_string(), kind, into });
    }

    /// Record a node id, keeping the first chain that reached it.
    fn push(&mut self, node_id: String, chain: &Chain) {
        self.selected.entry(node_id.clone()).or_insert_with(|| Selected {
            node_id,
            via: chain.via.clone(),
            origin: chain.origin.clone(),
        });
    }

    fn drop_entry(&mut self, entry: &str, why: DropReason) {
        self.dropped.push(Dropped { entry: entry.to_string(), why });
    }

    /// Drop the per-test node ids of any file selected wholesale.
    ///
    /// The closure already does this once; it has to happen again here because
    /// fixture and conftest expansion can introduce whole-file entries after
    /// the fact.
    fn collapse(&mut self) -> Vec<Selected> {
        let whole: BTreeSet<String> =
            self.selected.keys().filter(|id| !id.contains("::")).cloned().collect();

        let mut kept = Vec::new();
        for (id, selected) in std::mem::take(&mut self.selected) {
            match id.split_once("::") {
                Some((file, _)) if whole.contains(file) => {
                    self.dropped.push(Dropped { entry: id, why: DropReason::Superseded });
                }
                _ => kept.push(selected),
            }
        }
        kept
    }
}

/// `tests/test_x.py::TestFoo::test_bar` — the file as-is, the qualname's dots
/// translated.
fn node_id(file: &Path, qualname: &str) -> String {
    format!("{}::{}", file.display(), qualname.replace('.', "::"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::closure::{ImpactedTest, Stats};
    use crate::modpath::module_path;
    use crate::pysource;
    use crate::workspace::is_test_path;

    /// A [`Tree`] over a map of repository-relative path to source text.
    ///
    /// The parsing is the real `pysource` one, so these tests exercise the same
    /// collection rules the binary uses.
    #[derive(Default)]
    struct MapTree {
        files: BTreeMap<PathBuf, String>,
        /// Paths git still lists but the working tree no longer has.
        gone: BTreeSet<PathBuf>,
    }

    impl MapTree {
        fn with(mut self, path: &str, source: &str) -> Self {
            self.files.insert(PathBuf::from(path), source.to_string());
            self
        }

        fn deleted(mut self, path: &str) -> Self {
            self.gone.insert(PathBuf::from(path));
            self
        }
    }

    impl Tree for MapTree {
        fn test_files(&self) -> Vec<PathBuf> {
            self.files
                .keys()
                .filter(|file| is_test_path(file, Path::new("/repo")))
                .cloned()
                .collect()
        }

        fn module(&self, file: &Path) -> Option<Rc<Module>> {
            let source = self.files.get(file)?;
            pysource::parse(source).ok().filter(|module| !module.has_error).map(Rc::new)
        }

        fn exists(&self, file: &Path) -> bool {
            !self.gone.contains(file) && self.files.contains_key(file)
        }

        fn module_path(&self, file: &Path) -> Option<String> {
            // No `__init__.py` chain in a map, so the fallback rule applies —
            // which is what a flat `tests/` directory gets in a real tree too.
            module_path(Path::new("/nonexistent"), file)
        }
    }

    /// A [`Tree`] that records which files were actually parsed.
    ///
    /// The laziness is a property of `select`, not of any one answer, so it
    /// needs a `Tree` that can be asked afterwards what it was made to read.
    struct Counting {
        tree: MapTree,
        parsed: std::cell::RefCell<BTreeSet<PathBuf>>,
    }

    impl Counting {
        fn new(tree: MapTree) -> Self {
            Self { tree, parsed: std::cell::RefCell::default() }
        }
    }

    impl Tree for Counting {
        fn test_files(&self) -> Vec<PathBuf> {
            self.tree.test_files()
        }

        fn module(&self, file: &Path) -> Option<Rc<Module>> {
            self.parsed.borrow_mut().insert(file.to_path_buf());
            self.tree.module(file)
        }

        fn exists(&self, file: &Path) -> bool {
            self.tree.exists(file)
        }

        fn module_path(&self, file: &Path) -> Option<String> {
            self.tree.module_path(file)
        }
    }

    fn report(tests: Vec<ImpactedTest>, changed_tests: Vec<&str>) -> ImpactReport {
        ImpactReport {
            verdict: Verdict::Selected,
            reason: None,
            base: "origin/main".to_string(),
            merge_base: "abc1234".to_string(),
            impacted_tests: tests,
            test_files_changed: changed_tests.into_iter().map(ToString::to_string).collect(),
            ignored_symbols: Vec::new(),
            stats: Stats::default(),
            errors: Vec::new(),
        }
    }

    fn impacted(file: &str, symbol: Option<&str>) -> ImpactedTest {
        ImpactedTest {
            file: file.to_string(),
            symbol: symbol.map(ToString::to_string),
            via: Vec::new(),
            origin: "pkg.core:target".to_string(),
        }
    }

    fn ids(selection: &Selection) -> Vec<&str> {
        selection.ids().collect()
    }

    const TESTS: &str = "
import pytest


def test_top(shelter):
    pass


class TestGood:
    def test_ok(self):
        pass

    def helper(self):
        pass


class TestBad:
    def __init__(self):
        pass

    def test_skipped(self):
        pass


def _make_thing():
    pass
";

    const CONFTEST: &str = "
import pytest


@pytest.fixture
def shelter():
    return 1
";

    fn tree() -> MapTree {
        MapTree::default()
            .with("tests/conftest.py", CONFTEST)
            .with("tests/test_a.py", TESTS)
            .with("tests/test_b.py", "def test_b():\n    pass\n")
            .with("tests/__init__.py", "")
            .with("src/pkg/core.py", "def target():\n    pass\n")
    }

    #[test]
    fn a_fixture_named_like_a_test_expands_instead_of_becoming_a_node_id() {
        // The whole-pipeline form of the fixtures.rs regression: pytest cannot
        // collect `tests/conftest.py::test_client`, so emitting it would fail
        // the run *and* miss `test_x` — a confident selection that misses a
        // test, which is the one failure this module must not have.
        let tree = MapTree::default()
            .with(
                "tests/conftest.py",
                r"import pytest


@pytest.fixture
def test_client():
    return 1
",
            )
            .with("tests/test_a.py", "def test_x(test_client):\n    pass\n");

        let selection = select(
            &report(
                vec![impacted("tests/conftest.py", Some("tests.conftest:test_client"))],
                vec![],
            ),
            &tree,
        );
        assert_eq!(ids(&selection), vec!["tests/test_a.py::test_x"]);
        assert_eq!(selection.expanded[0].kind, ExpansionKind::Fixture);
    }

    #[test]
    fn a_test_named_helper_in_a_conftest_never_becomes_a_node_id() {
        // Not a fixture, so no name edge — but `conftest.py` collects nothing,
        // and handing pytest `tests/conftest.py::test_helper` is a usage error.
        let tree = MapTree::default()
            .with("tests/conftest.py", "def test_helper():\n    pass\n")
            .with("tests/test_a.py", "def test_x():\n    pass\n");

        let selection = select(
            &report(
                vec![impacted("tests/conftest.py", Some("tests.conftest:test_helper"))],
                vec![],
            ),
            &tree,
        );
        assert_eq!(
            ids(&selection),
            vec!["tests/test_a.py"],
            "the conftest widens to its subtree instead"
        );
    }

    #[test]
    fn only_the_files_the_report_needs_are_parsed() {
        // `impacted-tests` refuses to list a tree it does not need; `run` must
        // not undo that by parsing every test file to answer one entry.
        let counting = Counting::new(tree());
        let selection = select(
            &report(
                vec![impacted("tests/test_a.py", Some("tests.test_a:TestGood.test_ok"))],
                vec![],
            ),
            &counting,
        );
        assert_eq!(ids(&selection), vec!["tests/test_a.py::TestGood::test_ok"]);
        assert_eq!(
            counting.parsed.into_inner(),
            [PathBuf::from("tests/test_a.py")].into(),
            "one collectible test needs one file, not the whole tree"
        );
    }

    #[test]
    fn a_fixture_expansion_parses_its_scope_and_nothing_further() {
        // A fixture in `tests/unit/conftest.py` scopes over `tests/unit` only:
        // `tests/test_a.py` is outside it and must stay unread.
        let inner = MapTree::default()
            .with("tests/conftest.py", CONFTEST)
            .with("tests/test_a.py", TESTS)
            .with("tests/unit/conftest.py", CONFTEST)
            .with("tests/unit/test_c.py", "def test_c(shelter):\n    pass\n");
        let counting = Counting::new(inner);

        let selection = select(
            &report(
                vec![impacted("tests/unit/conftest.py", Some("tests.unit.conftest:shelter"))],
                vec![],
            ),
            &counting,
        );
        assert_eq!(ids(&selection), vec!["tests/unit/test_c.py::test_c"]);

        let parsed = counting.parsed.into_inner();
        assert!(parsed.contains(Path::new("tests/unit/test_c.py")), "the scope was read");
        assert!(
            parsed.contains(Path::new("tests/conftest.py")),
            "and the conftest chain above it, which decides which `shelter` wins"
        );
        assert!(
            !parsed.contains(Path::new("tests/test_a.py")),
            "but nothing outside the scope: {parsed:?}"
        );
    }

    #[test]
    fn a_collectible_symbol_becomes_its_node_id() {
        let selection = select(
            &report(vec![impacted("tests/test_a.py", Some("tests.test_a:test_top"))], vec![]),
            &tree(),
        );
        assert_eq!(ids(&selection), vec!["tests/test_a.py::test_top"]);
        assert_eq!(selection.decision, Decision::Selected);
    }

    #[test]
    fn a_method_qualname_translates_its_dots_to_colons() {
        let selection = select(
            &report(
                vec![impacted("tests/test_a.py", Some("tests.test_a:TestGood.test_ok"))],
                vec![],
            ),
            &tree(),
        );
        assert_eq!(ids(&selection), vec!["tests/test_a.py::TestGood::test_ok"]);
    }

    #[test]
    fn a_null_symbol_selects_the_file_itself() {
        let selection = select(&report(vec![impacted("tests/test_a.py", None)], vec![]), &tree());
        assert_eq!(ids(&selection), vec!["tests/test_a.py"], "a whole-file selection is the file");
    }

    #[test]
    fn a_helper_method_trims_back_to_its_class() {
        let selection = select(
            &report(
                vec![impacted("tests/test_a.py", Some("tests.test_a:TestGood.helper"))],
                vec![],
            ),
            &tree(),
        );
        assert_eq!(
            ids(&selection),
            vec!["tests/test_a.py::TestGood"],
            "the longest collectible prefix over-selects safely"
        );
    }

    #[test]
    fn a_test_class_pytest_cannot_instantiate_degrades_to_the_whole_file() {
        let selection = select(
            &report(
                vec![impacted("tests/test_a.py", Some("tests.test_a:TestBad.test_skipped"))],
                vec![],
            ),
            &tree(),
        );
        assert_eq!(
            ids(&selection),
            vec!["tests/test_a.py"],
            "an __init__ makes even the first segment uncollectible"
        );
    }

    #[test]
    fn a_module_level_helper_degrades_to_the_whole_file() {
        let selection = select(
            &report(vec![impacted("tests/test_a.py", Some("tests.test_a:_make_thing"))], vec![]),
            &tree(),
        );
        assert_eq!(ids(&selection), vec!["tests/test_a.py"], "a helper is not a node id");
    }

    #[test]
    fn a_fixture_expands_to_the_tests_that_consume_it() {
        // The failure mode this exists for: `conftest.py` collects zero tests,
        // so the whole-file fallback would have selected nothing at all.
        let selection = select(
            &report(vec![impacted("tests/conftest.py", Some("tests.conftest:shelter"))], vec![]),
            &tree(),
        );

        assert_eq!(ids(&selection), vec!["tests/test_a.py::test_top"]);
        assert_eq!(
            selection.node_ids[0].via,
            vec!["tests.conftest:shelter".to_string()],
            "the fixture is prepended to the chain, so the human output names the edge"
        );
        assert_eq!(
            selection.node_ids[0].origin.as_deref(),
            Some("pkg.core:target"),
            "and the origin is still the changed symbol"
        );
        assert_eq!(selection.expanded[0].kind, ExpansionKind::Fixture);
        assert_eq!(selection.expanded[0].from, "tests.conftest:shelter");
    }

    #[test]
    fn a_fixture_nothing_consumes_is_dropped_rather_than_widened() {
        let tree = MapTree::default()
            .with("tests/conftest.py", CONFTEST)
            .with("tests/test_b.py", "def test_b():\n    pass\n");
        let selection = select(
            &report(vec![impacted("tests/conftest.py", Some("tests.conftest:shelter"))], vec![]),
            &tree,
        );
        assert!(ids(&selection).is_empty(), "no consumer means nothing to run");
        assert_eq!(selection.decision, Decision::Nothing);
        assert_eq!(selection.dropped[0].why, DropReason::NoConsumers);
    }

    #[test]
    fn a_changed_conftest_selects_its_whole_subtree() {
        let selection = select(&report(vec![], vec!["tests/conftest.py"]), &tree());
        assert_eq!(
            ids(&selection),
            vec!["tests/test_a.py", "tests/test_b.py"],
            "the module executes at collection time for everything under it"
        );
        assert_eq!(selection.expanded[0].kind, ExpansionKind::ConftestSubtree);
        assert_eq!(
            selection.node_ids[0].via,
            vec!["tests.conftest".to_string()],
            "a module id has no colon, which is what says `whole module`"
        );
        assert_eq!(selection.node_ids[0].origin, None, "a changed test file has no origin");
    }

    #[test]
    fn a_module_level_conftest_change_reaches_the_subtree_too() {
        // The `symbol: null` route rather than the `test_files_changed` one.
        let selection = select(&report(vec![impacted("tests/conftest.py", None)], vec![]), &tree());
        assert_eq!(ids(&selection), vec!["tests/test_a.py", "tests/test_b.py"]);
    }

    #[test]
    fn a_changed_test_file_selects_itself() {
        let selection = select(&report(vec![], vec!["tests/test_b.py"]), &tree());
        assert_eq!(ids(&selection), vec!["tests/test_b.py"]);
        assert_eq!(selection.node_ids[0].origin, None);
    }

    #[test]
    fn a_deleted_test_file_is_dropped() {
        let tree = tree().deleted("tests/test_b.py");
        let selection = select(&report(vec![], vec!["tests/test_b.py"]), &tree);
        assert!(ids(&selection).is_empty(), "a file that is gone cannot be run");
        assert_eq!(
            selection.dropped,
            vec![Dropped { entry: "tests/test_b.py".to_string(), why: DropReason::Deleted }]
        );
    }

    #[test]
    fn a_file_pytest_collects_nothing_from_is_dropped() {
        let selection = select(&report(vec![], vec!["tests/__init__.py"]), &tree());
        assert!(ids(&selection).is_empty(), "a package marker would be exit code 5");
        assert_eq!(selection.dropped[0].why, DropReason::NotCollectible);
    }

    #[test]
    fn a_whole_file_supersedes_its_own_node_ids_after_expansion() {
        let selection = select(
            &report(
                vec![impacted("tests/test_a.py", Some("tests.test_a:test_top"))],
                vec!["tests/conftest.py"],
            ),
            &tree(),
        );
        assert_eq!(
            ids(&selection),
            vec!["tests/test_a.py", "tests/test_b.py"],
            "the conftest expansion covers `test_a.py` wholesale"
        );
        assert!(
            selection.dropped.contains(&Dropped {
                entry: "tests/test_a.py::test_top".to_string(),
                why: DropReason::Superseded
            }),
            "and the finer entry says why it went, got {:?}",
            selection.dropped
        );
    }

    #[test]
    fn a_run_all_verdict_maps_nothing_at_all() {
        let mut report = report(vec![impacted("tests/test_a.py", None)], vec!["tests/test_b.py"]);
        report.verdict = Verdict::RunAll;
        report.reason = Some(Reason::TyfUnavailable);

        let selection = select(&report, &tree());
        assert_eq!(selection.decision, Decision::RunAll);
        assert!(selection.node_ids.is_empty(), "the whole suite runs either way");
        assert_eq!(selection.reason, Some(Reason::TyfUnavailable), "and the reason survives");
    }

    #[test]
    fn an_empty_selection_is_a_decision_of_its_own() {
        let selection = select(&report(vec![], vec![]), &tree());
        assert_eq!(
            selection.decision,
            Decision::Nothing,
            "an empty pytest argv would mean `run everything`, so this has to be its own case"
        );
    }

    #[test]
    fn node_ids_come_out_sorted_and_deduplicated() {
        let selection = select(
            &report(
                vec![
                    impacted("tests/test_b.py", Some("tests.test_b:test_b")),
                    impacted("tests/test_a.py", Some("tests.test_a:test_top")),
                    impacted("tests/test_a.py", Some("tests.test_a:test_top")),
                ],
                vec![],
            ),
            &tree(),
        );
        assert_eq!(
            ids(&selection),
            vec!["tests/test_a.py::test_top", "tests/test_b.py::test_b"],
            "the argv has to be the same on every run"
        );
    }

    #[test]
    fn origins_counts_the_distinct_changed_symbols() {
        let mut second = impacted("tests/test_b.py", Some("tests.test_b:test_b"));
        second.origin = "pkg.other:thing".to_string();
        let selection = select(
            &report(
                vec![impacted("tests/test_a.py", Some("tests.test_a:test_top")), second],
                vec![],
            ),
            &tree(),
        );
        assert_eq!(selection.origins(), 2, "the stderr line reports what the run traced back to");
    }
}
