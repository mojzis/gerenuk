//! Which tests a pytest fixture reaches, and which of them pytest can collect.
//!
//! pytest injects fixtures by **name**, not by reference, so a fixture is
//! invisible to `tyf` from the consuming side: the walk in [`crate::closure`]
//! dead-ends at the fixture's definition. This module is the compensating
//! read — it parses the test files and resolves the name edge that the type
//! checker cannot see.
//!
//! It is pure: it takes already-parsed [`Module`]s and returns paths and
//! qualified names. Nothing here reads a file, spawns a process, or knows what
//! a node id looks like — that is [`crate::select`]'s job.
//!
//! The collection conventions are pytest's defaults (`test*` functions,
//! `Test*` classes with no `__init__`, `test_*.py` / `*_test.py` files). A
//! project that overrides them in its pytest config makes this gate wrong in
//! both directions; see the phase-3 plan's deferred list.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::pysource::{Kind, Literal, Module, SymbolSpan};

/// The file name pytest gives directory-scoped fixtures.
pub const CONFTEST: &str = "conftest.py";

/// Decorator suffix that marks a fixture: matches `fixture` and
/// `pytest.fixture` alike, and deliberately not `pytest.mark.usefixtures`.
const FIXTURE_DECORATOR: &str = "fixture";

/// Decorator suffix that names fixtures a test needs without taking them as
/// parameters.
const USEFIXTURES_DECORATOR: &str = "mark.usefixtures";

/// pytest's default `python_classes`.
const TEST_CLASS_PREFIX: &str = "Test";

/// pytest's default `python_functions`.
const TEST_FUNCTION_PREFIX: &str = "test";

/// One `@pytest.fixture`, as far as reading the source can tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fixture {
    /// Repository-relative file that defines it.
    pub file: PathBuf,
    /// Dotted name within its module — what the why-chain names.
    pub qualname: String,
    /// The name pytest injects it under: the def's own name, unless a
    /// string-literal `name=` overrode it.
    pub name: String,
    /// `autouse=True`: every test in scope consumes it, named or not.
    pub autouse: bool,
    /// Other fixtures it requests, by name.
    pub requests: Vec<String>,
    /// A non-literal `name=` made the injected name unknowable, so [`Self::name`]
    /// is a guess and consumers cannot be resolved by name at all.
    pub unresolvable: bool,
}

/// One test pytest would collect, and the fixture names it asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestItem {
    /// Repository-relative file it lives in.
    pub file: PathBuf,
    /// Dotted name within the module: `TestShelter.test_summary`.
    pub qualname: String,
    /// Fixture names it requests, from its parameters and from `usefixtures`
    /// on it or on its class.
    pub requests: BTreeSet<String>,
}

/// How much of a qualified name pytest can actually collect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Collect {
    /// Every segment is collectible: the whole name is a node id.
    All,
    /// Only this prefix is. The rest is a helper nested inside a test, and
    /// selecting the prefix over-selects safely.
    Prefix(String),
    /// Not even the first segment — a helper, or a fixture.
    None,
}

/// Everything one file contributes to the map.
#[derive(Debug, Default)]
struct FileFacts {
    /// Fixtures by the name pytest injects them under.
    fixtures: BTreeMap<String, Fixture>,
    /// Collectible tests, in source order.
    tests: Vec<TestItem>,
    /// Every definition's qualname, and whether pytest can collect that
    /// segment on its own.
    segments: BTreeMap<String, bool>,
}

/// The fixture and collection facts of a whole test tree.
#[derive(Debug, Default)]
pub struct FixtureMap {
    files: BTreeMap<PathBuf, FileFacts>,
}

impl FixtureMap {
    /// Read the map out of already-parsed test files.
    ///
    /// A `None` module is a file that is present but could not be parsed: it
    /// stays in the map — so subtree expansion still selects it — and
    /// contributes no fixtures and no tests.
    pub fn build<'a>(modules: impl IntoIterator<Item = (PathBuf, Option<&'a Module>)>) -> Self {
        let mut files = BTreeMap::new();
        for (path, module) in modules {
            let facts = module.map(|module| facts_of(&path, module)).unwrap_or_default();
            files.insert(path, facts);
        }
        Self { files }
    }

    /// Whether the map has heard of this file at all.
    #[must_use]
    pub fn has(&self, file: &Path) -> bool {
        self.files.contains_key(file)
    }

    /// The tests pytest would collect from one file.
    #[must_use]
    pub fn tests_in(&self, file: &Path) -> &[TestItem] {
        self.files.get(file).map_or(&[], |facts| facts.tests.as_slice())
    }

    /// The fixture defined at `file`'s `qualname`, if that definition is one.
    #[must_use]
    pub fn fixture_at(&self, file: &Path, qualname: &str) -> Option<&Fixture> {
        self.files.get(file)?.fixtures.values().find(|fixture| fixture.qualname == qualname)
    }

    /// How much of `qualname` pytest can collect, segment by segment.
    #[must_use]
    pub fn collect(&self, file: &Path, qualname: &str) -> Collect {
        self.files
            .get(file)
            .map_or(Collect::None, |facts| longest_collectible(&facts.segments, qualname))
    }

    /// Every file in the map under `dir` that pytest would collect tests from.
    ///
    /// `dir` is a repository-relative directory; the empty path means the whole
    /// tree, which is what a repository-root `conftest.py` scopes over.
    #[must_use]
    pub fn subtree(&self, dir: &Path) -> Vec<PathBuf> {
        self.files
            .keys()
            .filter(|file| file.starts_with(dir) && collects_tests(file))
            .cloned()
            .collect()
    }

    /// The fixture named `name` that a test in `file` would actually get.
    ///
    /// pytest resolves a fixture from the module outwards: a fixture defined in
    /// the test module shadows one of the same name in any `conftest.py`, and a
    /// nearer `conftest.py` shadows a further one.
    #[must_use]
    pub fn visible(&self, file: &Path, name: &str) -> Option<&Fixture> {
        if let Some(found) = self.files.get(file).and_then(|facts| facts.fixtures.get(name)) {
            return Some(found);
        }
        let mut dir = file.parent();
        while let Some(current) = dir {
            let conftest = current.join(CONFTEST);
            if conftest != file {
                if let Some(found) =
                    self.files.get(&conftest).and_then(|facts| facts.fixtures.get(name))
                {
                    return Some(found);
                }
            }
            dir = current.parent();
        }
        None
    }

    /// The tests and whole files one fixture reaches.
    ///
    /// The scope is the fixture's own module, or — for a `conftest.py` — every
    /// collectible file in its subtree. Within that scope a test consumes the
    /// fixture when it names it, when `usefixtures` names it, when another
    /// fixture it requests names it, or when the fixture is `autouse`.
    #[must_use]
    pub fn consumers(&self, fixture: &Fixture) -> Consumers {
        let mut consumers = Consumers::default();

        for file in self.scope_of(fixture) {
            if fixture.unresolvable {
                // The injected name is unknowable, so no test can be matched to
                // it by name — and the shadowing check below would be matching
                // on a guess. The whole file is the honest answer.
                consumers.files.push(file);
                continue;
            }
            if !self.reaches(&file, fixture) {
                continue;
            }

            let visible = self.visible_fixtures(&file);
            let names = requesting_names(&visible, &fixture.name);
            for test in self.tests_in(&file) {
                if fixture.autouse || test.requests.iter().any(|name| names.contains(name)) {
                    consumers.tests.push(test.clone());
                }
            }
        }

        consumers
    }

    /// Whether a test in `file` asking for `fixture`'s name ends up running
    /// this fixture.
    ///
    /// Usually that is just "nothing nearer shadows the name". The exception is
    /// pytest's override idiom — `@pytest.fixture def shelter(shelter)` in a
    /// nearer `conftest.py` — where the override shadows the name *and*
    /// requests it, so the outer fixture still executes. Following that chain
    /// is what stops a change to the outer fixture vanishing under an override.
    fn reaches(&self, file: &Path, fixture: &Fixture) -> bool {
        let name = fixture.name.as_str();
        let mut current = file.to_path_buf();
        let mut seen = BTreeSet::new();

        loop {
            let Some(found) = self.visible(&current, name) else { return false };
            if found == fixture {
                return true;
            }
            // An override that does not ask for the name it shadows replaces
            // the outer fixture outright.
            if !found.requests.iter().any(|request| request == name) {
                return false;
            }
            if !seen.insert(found.file.clone()) {
                return false;
            }
            // Resume the search immediately above whatever defined the
            // override: its own directory, or its parent when it is a conftest.
            let above = if is_conftest(&found.file) {
                found.file.parent().and_then(Path::parent)
            } else {
                found.file.parent()
            };
            let Some(above) = above else { return false };
            current = above.join(CONFTEST);
        }
    }

    /// The files a fixture could possibly be injected into.
    fn scope_of(&self, fixture: &Fixture) -> Vec<PathBuf> {
        if is_conftest(&fixture.file) {
            self.subtree(fixture.file.parent().unwrap_or_else(|| Path::new("")))
        } else if collects_tests(&fixture.file) {
            vec![fixture.file.clone()]
        } else {
            Vec::new()
        }
    }

    /// Every fixture name a test in `file` could request, each resolved to the
    /// definition pytest would actually use.
    ///
    /// Built once per file rather than per lookup: the conftest chain is walked
    /// for every name otherwise, and a repository-root `conftest.py` puts every
    /// test file in the repository in scope.
    fn visible_fixtures(&self, file: &Path) -> BTreeMap<&str, &Fixture> {
        let mut visible: BTreeMap<&str, &Fixture> = BTreeMap::new();
        if let Some(facts) = self.files.get(file) {
            visible.extend(facts.fixtures.iter().map(|(name, f)| (name.as_str(), f)));
        }

        let mut dir = file.parent();
        while let Some(current) = dir {
            let conftest = current.join(CONFTEST);
            if conftest != file {
                if let Some(facts) = self.files.get(&conftest) {
                    // Nearer wins: anything already recorded shadows this.
                    for (name, fixture) in &facts.fixtures {
                        visible.entry(name.as_str()).or_insert(fixture);
                    }
                }
            }
            dir = current.parent();
        }
        visible
    }
}

/// Every visible fixture name that resolves, transitively, to `name` — a test
/// asking for any of them ends up running that fixture.
///
/// Terminates on the reached set, so a fixture cycle is a fixed point rather
/// than a hang.
fn requesting_names(visible: &BTreeMap<&str, &Fixture>, name: &str) -> BTreeSet<String> {
    let mut reached: BTreeSet<String> = [name.to_string()].into();
    loop {
        let mut grew = false;
        for (candidate, fixture) in visible {
            if reached.contains(*candidate) {
                continue;
            }
            if fixture.requests.iter().any(|request| reached.contains(request)) {
                reached.insert((*candidate).to_string());
                grew = true;
            }
        }
        if !grew {
            return reached;
        }
    }
}

/// The longest prefix of `qualname` pytest can collect, given each definition's
/// own collectibility.
///
/// The one place the segment walk lives: [`FixtureMap::collect`] answers with
/// it, and [`facts_of`] uses it during construction, before there is a
/// [`FixtureMap`] to ask.
fn longest_collectible(segments: &BTreeMap<String, bool>, qualname: &str) -> Collect {
    let mut prefix = String::new();
    let mut collectible = String::new();

    for segment in qualname.split('.') {
        if !prefix.is_empty() {
            prefix.push('.');
        }
        prefix.push_str(segment);
        // An unknown segment is a definition the parse never saw. Treating it
        // as non-collectible is the over-selecting direction.
        if !segments.get(&prefix).copied().unwrap_or(false) {
            break;
        }
        collectible.clone_from(&prefix);
    }

    if collectible.is_empty() {
        Collect::None
    } else if collectible == qualname {
        Collect::All
    } else {
        Collect::Prefix(collectible)
    }
}

/// What a fixture reaches: precise tests where the names resolved, whole files
/// where they could not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Consumers {
    /// Individual tests, where every name involved resolved.
    pub tests: Vec<TestItem>,
    /// Whole test files, where they did not.
    pub files: Vec<PathBuf>,
}

impl Consumers {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tests.is_empty() && self.files.is_empty()
    }
}

/// Whether pytest would collect tests from this path, by its default
/// `python_files` conventions.
///
/// A `conftest.py` collects nothing: it is configuration, and handing it to
/// pytest as a node id selects no tests at all.
#[must_use]
pub fn collects_tests(file: &Path) -> bool {
    if is_conftest(file) || file.extension().is_none_or(|ext| ext != "py") {
        return false;
    }
    file.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem.starts_with("test_") || stem.ends_with("_test"))
}

/// Whether the path is a `conftest.py`.
#[must_use]
pub fn is_conftest(file: &Path) -> bool {
    file.file_name().is_some_and(|name| name == CONFTEST)
}

/// Read one parsed module's fixtures, tests and collectible segments.
fn facts_of(file: &Path, module: &Module) -> FileFacts {
    let mut facts = FileFacts::default();

    let defines_init: BTreeSet<&str> =
        module.spans.iter().filter_map(|span| span.qualname.strip_suffix(".__init__")).collect();

    for span in &module.spans {
        let last = last_segment(&span.qualname);
        let collectible = match span.kind {
            // pytest skips a `Test*` class with an `__init__`: it cannot
            // instantiate it, and collecting it is a warning, not a test.
            Kind::Class => {
                last.starts_with(TEST_CLASS_PREFIX)
                    && !defines_init.contains(span.qualname.as_str())
            }
            Kind::Function | Kind::Method => last.starts_with(TEST_FUNCTION_PREFIX),
        };
        facts.segments.insert(span.qualname.clone(), collectible);
    }

    for span in &module.spans {
        if let Some(fixture) = fixture_of(file, span) {
            // A fixture whose name is already taken keeps the first definition,
            // which is the one nearer the top of the file.
            facts.fixtures.entry(fixture.name.clone()).or_insert(fixture);
            continue;
        }
        if matches!(span.kind, Kind::Function | Kind::Method)
            && longest_collectible(&facts.segments, &span.qualname) == Collect::All
        {
            facts.tests.push(TestItem {
                file: file.to_path_buf(),
                qualname: span.qualname.clone(),
                requests: requests_of(module, span),
            });
        }
    }

    facts
}

/// The fixture a definition declares, if its decorators say it is one.
fn fixture_of(file: &Path, span: &SymbolSpan) -> Option<Fixture> {
    let decorator = span.decorator(FIXTURE_DECORATOR)?;
    let override_name = decorator.kwarg("name");

    let (name, unresolvable) = match override_name {
        Some(Literal::Str(name)) => (name.clone(), false),
        // `name=` is there and we cannot read it: the def's own name is no
        // longer what pytest injects, so nothing can be matched by name.
        Some(_) => (last_segment(&span.qualname).to_string(), true),
        None => (last_segment(&span.qualname).to_string(), false),
    };

    Some(Fixture {
        file: file.to_path_buf(),
        qualname: span.qualname.clone(),
        name,
        autouse: decorator.kwarg("autouse").is_some_and(Literal::is_true),
        // `self` is not a fixture request, and neither is `request`; both are
        // harmless here because no fixture is ever named either.
        requests: span.params.clone(),
        unresolvable,
    })
}

/// Every fixture name a definition asks for: its parameters, plus the
/// string-literal arguments of `usefixtures` on it and on its class.
fn requests_of(module: &Module, span: &SymbolSpan) -> BTreeSet<String> {
    let mut requests: BTreeSet<String> = span.params.iter().cloned().collect();
    requests.extend(usefixtures(span));

    // A `usefixtures` on a class applies to every method in it, and pytest
    // collects nested `Test*` classes — so every enclosing definition counts,
    // not just the immediate one. A qualname's prefixes are exactly those.
    let mut owner = span.qualname.as_str();
    while let Some((prefix, _)) = owner.rsplit_once('.') {
        if let Some(enclosing) = module.spans.iter().find(|other| other.qualname == prefix) {
            requests.extend(usefixtures(enclosing));
        }
        owner = prefix;
    }
    requests
}

/// String-literal fixture names from `@pytest.mark.usefixtures(...)`.
fn usefixtures(span: &SymbolSpan) -> Vec<String> {
    span.decorators
        .iter()
        .filter(|decorator| crate::pysource::suffix_matches(&decorator.name, USEFIXTURES_DECORATOR))
        .flat_map(|decorator| decorator.string_args().map(ToString::to_string))
        .collect()
}

/// The last dotted component — `test_bar` for `TestFoo.test_bar`.
fn last_segment(qualname: &str) -> &str {
    qualname.rsplit('.').next().unwrap_or(qualname)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pysource;

    /// Build a map from repository-relative path to source text.
    fn map(files: &[(&str, &str)]) -> FixtureMap {
        let parsed: Vec<(PathBuf, Option<Module>)> = files
            .iter()
            .map(|(path, source)| {
                let module =
                    pysource::parse(source).ok().filter(|module: &Module| !module.has_error);
                (PathBuf::from(path), module)
            })
            .collect();
        FixtureMap::build(parsed.iter().map(|(path, module)| (path.clone(), module.as_ref())))
    }

    /// The `(file, qualname)` pairs a fixture reaches, plus its whole files.
    fn reached(map: &FixtureMap, file: &str, qualname: &str) -> (Vec<String>, Vec<String>) {
        let fixture = map
            .fixture_at(Path::new(file), qualname)
            .unwrap_or_else(|| panic!("`{qualname}` in `{file}` should be a fixture"));
        let consumers = map.consumers(fixture);
        (
            consumers
                .tests
                .iter()
                .map(|test| format!("{}::{}", test.file.display(), test.qualname))
                .collect(),
            consumers.files.iter().map(|file| file.display().to_string()).collect(),
        )
    }

    const CONFTEST_SRC: &str = r#"
import pytest


@pytest.fixture
def shelter():
    return "shelter"


@pytest.fixture
def service(shelter):
    return shelter


@pytest.fixture(autouse=True)
def clock():
    return 0


@pytest.fixture(name="renamed")
def _make_renamed():
    return 1
"#;

    const TEST_SRC: &str = r#"
import pytest


def test_direct(shelter):
    assert shelter


def test_chained(service):
    assert service


def test_nothing():
    assert True


@pytest.mark.usefixtures("shelter")
def test_marked():
    assert True


def _helper():
    return 1
"#;

    fn tree() -> FixtureMap {
        map(&[("tests/conftest.py", CONFTEST_SRC), ("tests/test_a.py", TEST_SRC)])
    }

    #[test]
    fn a_parameter_naming_a_fixture_is_a_consumer() {
        let (tests, files) = reached(&tree(), "tests/conftest.py", "shelter");
        assert!(files.is_empty(), "the names resolved, so nothing needs a whole file");
        assert_eq!(
            tests,
            vec![
                "tests/test_a.py::test_direct",
                "tests/test_a.py::test_chained",
                "tests/test_a.py::test_marked",
            ],
            "the direct request, the transitive one and the usefixtures mark, in source order"
        );
    }

    #[test]
    fn a_fixture_requesting_another_carries_only_its_own_consumers() {
        let (tests, _) = reached(&tree(), "tests/conftest.py", "service");
        assert_eq!(tests, vec!["tests/test_a.py::test_chained"], "`test_direct` never asks for it");
    }

    #[test]
    fn autouse_reaches_every_test_in_scope_without_being_named() {
        let (tests, _) = reached(&tree(), "tests/conftest.py", "clock");
        assert_eq!(
            tests,
            vec![
                "tests/test_a.py::test_direct",
                "tests/test_a.py::test_chained",
                "tests/test_a.py::test_nothing",
                "tests/test_a.py::test_marked",
            ],
            "`test_nothing` requests nothing and still runs it"
        );
    }

    #[test]
    fn a_string_literal_name_is_the_one_consumers_use() {
        let map = map(&[
            ("tests/conftest.py", CONFTEST_SRC),
            ("tests/test_a.py", "def test_x(renamed):\n    assert renamed\n"),
        ]);
        let (tests, _) = reached(&map, "tests/conftest.py", "_make_renamed");
        assert_eq!(
            tests,
            vec!["tests/test_a.py::test_x"],
            "the def is `_make_renamed`; pytest injects it as `renamed`"
        );
    }

    #[test]
    fn a_dynamic_name_falls_back_to_every_file_in_scope() {
        let map = map(&[
            (
                "tests/conftest.py",
                "import pytest\n\nNAME = \"x\"\n\n\n@pytest.fixture(name=NAME)\ndef f():\n    pass\n",
            ),
            ("tests/test_a.py", "def test_x():\n    pass\n"),
            ("tests/test_b.py", "def test_y():\n    pass\n"),
        ]);
        let (tests, files) = reached(&map, "tests/conftest.py", "f");
        assert!(tests.is_empty(), "no test can be matched to a name we cannot read");
        assert_eq!(files, vec!["tests/test_a.py", "tests/test_b.py"], "so the scope is the answer");
    }

    #[test]
    fn a_conftest_fixture_scopes_over_its_subtree_only() {
        let map = map(&[
            (
                "tests/unit/conftest.py",
                "import pytest\n\n\n@pytest.fixture\ndef shelter():\n    pass\n",
            ),
            ("tests/unit/test_a.py", "def test_a(shelter):\n    pass\n"),
            ("tests/wide/test_b.py", "def test_b(shelter):\n    pass\n"),
        ]);
        let (tests, _) = reached(&map, "tests/unit/conftest.py", "shelter");
        assert_eq!(
            tests,
            vec!["tests/unit/test_a.py::test_a"],
            "a sibling directory never sees this conftest"
        );
    }

    #[test]
    fn a_module_fixture_shadows_the_conftest_one_of_the_same_name() {
        let map = map(&[
            ("tests/conftest.py", CONFTEST_SRC),
            (
                "tests/test_a.py",
                "import pytest\n\n\n@pytest.fixture\ndef shelter():\n    pass\n\n\ndef test_x(shelter):\n    pass\n",
            ),
            ("tests/test_b.py", "def test_y(shelter):\n    pass\n"),
        ]);

        let (conftest_tests, _) = reached(&map, "tests/conftest.py", "shelter");
        assert_eq!(
            conftest_tests,
            vec!["tests/test_b.py::test_y"],
            "the shadowing module's test consumes its own fixture, not this one"
        );

        let (module_tests, _) = reached(&map, "tests/test_a.py", "shelter");
        assert_eq!(
            module_tests,
            vec!["tests/test_a.py::test_x"],
            "and pytest's rule is module-wins"
        );
    }

    #[test]
    fn an_override_that_requests_what_it_shadows_still_runs_the_outer_fixture() {
        // pytest's documented override idiom. The nearer fixture takes the
        // name *and* asks for it, so the outer one still executes — dropping
        // the file here would lose a test the change really does reach.
        let map = map(&[
            ("tests/conftest.py", CONFTEST_SRC),
            (
                "tests/unit/conftest.py",
                "import pytest\n\n\n@pytest.fixture\ndef shelter(shelter):\n    return shelter\n",
            ),
            ("tests/unit/test_a.py", "def test_a(shelter):\n    pass\n"),
            (
                "tests/wide/conftest.py",
                "import pytest\n\n\n@pytest.fixture\ndef shelter():\n    return 2\n",
            ),
            ("tests/wide/test_b.py", "def test_b(shelter):\n    pass\n"),
        ]);

        let (tests, _) = reached(&map, "tests/conftest.py", "shelter");
        assert_eq!(
            tests,
            vec!["tests/unit/test_a.py::test_a"],
            "the override passes the change on; the outright replacement in `wide/` does not"
        );
    }

    #[test]
    fn usefixtures_on_an_outer_class_reaches_a_nested_test_class() {
        // pytest collects nested `Test*` classes, and the outer class's mark
        // applies to them. Reading only the immediate parent would miss it.
        let map = map(&[
            ("tests/conftest.py", CONFTEST_SRC),
            (
                "tests/test_a.py",
                "import pytest\n\n\n@pytest.mark.usefixtures(\"shelter\")\nclass TestOuter:\n    class TestInner:\n        def test_x(self):\n            pass\n",
            ),
        ]);
        let (tests, _) = reached(&map, "tests/conftest.py", "shelter");
        assert_eq!(tests, vec!["tests/test_a.py::TestOuter.TestInner.test_x"]);
    }

    #[test]
    fn a_fixture_cycle_terminates() {
        let map = map(&[
            (
                "tests/conftest.py",
                "import pytest\n\n\n@pytest.fixture\ndef a(b):\n    pass\n\n\n@pytest.fixture\ndef b(a):\n    pass\n",
            ),
            ("tests/test_a.py", "def test_x(a):\n    pass\n"),
        ]);
        let (tests, _) = reached(&map, "tests/conftest.py", "b");
        assert_eq!(tests, vec!["tests/test_a.py::test_x"], "a cycle is a fixed point, not a hang");
    }

    #[test]
    fn usefixtures_on_a_class_applies_to_its_methods() {
        let map = map(&[
            ("tests/conftest.py", CONFTEST_SRC),
            (
                "tests/test_a.py",
                "import pytest\n\n\n@pytest.mark.usefixtures(\"shelter\")\nclass TestThing:\n    def test_x(self):\n        pass\n",
            ),
        ]);
        let (tests, _) = reached(&map, "tests/conftest.py", "shelter");
        assert_eq!(tests, vec!["tests/test_a.py::TestThing.test_x"]);
    }

    #[test]
    fn the_decorator_is_recognised_bare_called_and_dotted() {
        let map = map(&[(
            "tests/conftest.py",
            "from pytest import fixture\nimport pytest\n\n\n@fixture\ndef a():\n    pass\n\n\n@fixture()\ndef b():\n    pass\n\n\n@pytest.fixture(scope=\"session\")\ndef c():\n    pass\n",
        )]);
        for name in ["a", "b", "c"] {
            assert!(
                map.fixture_at(Path::new("tests/conftest.py"), name).is_some(),
                "`{name}` should be recognised as a fixture"
            );
        }
    }

    #[test]
    fn usefixtures_is_not_mistaken_for_a_fixture_definition() {
        let map = map(&[(
            "tests/test_a.py",
            "import pytest\n\n\n@pytest.mark.usefixtures(\"shelter\")\ndef test_x():\n    pass\n",
        )]);
        assert!(
            map.fixture_at(Path::new("tests/test_a.py"), "test_x").is_none(),
            "`mark.usefixtures` does not end in a `.fixture` component"
        );
        assert_eq!(map.tests_in(Path::new("tests/test_a.py")).len(), 1, "it is a test, though");
    }

    const CLASSES: &str = "
class TestGood:
    def test_ok(self):
        def nested():
            return 1

        return nested()

    def helper(self):
        pass


class TestBad:
    def __init__(self):
        pass

    def test_skipped(self):
        pass


class Helper:
    def test_never(self):
        pass


def test_top():
    pass


def _make_shelter():
    pass
";

    #[test]
    fn collection_follows_pytests_default_conventions() {
        let map = map(&[("tests/test_c.py", CLASSES)]);
        let file = Path::new("tests/test_c.py");

        assert_eq!(map.collect(file, "test_top"), Collect::All, "a top-level test function");
        assert_eq!(map.collect(file, "TestGood.test_ok"), Collect::All, "a method of a Test class");
        assert_eq!(
            map.collect(file, "TestGood.helper"),
            Collect::Prefix("TestGood".to_string()),
            "a helper method trims back to the class, which over-selects safely"
        );
        assert_eq!(
            map.collect(file, "TestBad.test_skipped"),
            Collect::None,
            "pytest cannot instantiate a Test class with an __init__"
        );
        assert_eq!(map.collect(file, "Helper.test_never"), Collect::None, "the class is not Test*");
        assert_eq!(map.collect(file, "_make_shelter"), Collect::None, "a module-level helper");
        assert_eq!(map.collect(file, "gone"), Collect::None, "and a name the parse never saw");
    }

    #[test]
    fn only_collectible_definitions_are_listed_as_tests() {
        let map = map(&[("tests/test_c.py", CLASSES)]);
        let names: Vec<&str> = map
            .tests_in(Path::new("tests/test_c.py"))
            .iter()
            .map(|t| t.qualname.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["TestGood.test_ok", "test_top"],
            "the __init__ class, the non-Test class and both helpers are not tests"
        );
    }

    #[test]
    fn conftest_and_non_test_files_collect_nothing() {
        assert!(!collects_tests(Path::new("tests/conftest.py")), "conftest holds no tests");
        assert!(!collects_tests(Path::new("tests/__init__.py")), "nor does a package marker");
        assert!(!collects_tests(Path::new("tests/helpers.py")), "nor a helper module");
        assert!(collects_tests(Path::new("tests/test_a.py")), "`test_*.py` does");
        assert!(collects_tests(Path::new("tests/a_test.py")), "so does `*_test.py`");
        assert!(!collects_tests(Path::new("tests/test_a.txt")), "and it has to be Python");
    }

    #[test]
    fn a_subtree_lists_collectible_files_only() {
        let map = map(&[
            ("tests/conftest.py", ""),
            ("tests/__init__.py", ""),
            ("tests/test_a.py", ""),
            ("tests/unit/test_b.py", ""),
            ("src/pkg/a.py", ""),
        ]);
        assert_eq!(
            map.subtree(Path::new("tests")),
            vec![PathBuf::from("tests/test_a.py"), PathBuf::from("tests/unit/test_b.py")],
            "the conftest and the package marker cannot be handed to pytest"
        );
        assert_eq!(
            map.subtree(Path::new("")).len(),
            2,
            "a repository-root conftest scopes over everything"
        );
    }

    #[test]
    fn a_file_that_does_not_parse_still_exists_for_subtree_expansion() {
        let map = map(&[("tests/test_a.py", "def test_x(:\n    pass\n")]);
        let file = Path::new("tests/test_a.py");
        assert!(map.has(file), "the file is in the tree");
        assert!(map.tests_in(file).is_empty(), "but a partial tree names no tests");
        assert_eq!(
            map.subtree(Path::new("tests")),
            vec![PathBuf::from("tests/test_a.py")],
            "so the coarse fallback still reaches it"
        );
    }
}
