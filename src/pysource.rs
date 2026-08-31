//! Mapping source lines to the Python symbol that encloses them.
//!
//! Parsing is done with `tree-sitter-python`: gerenuk never shells out to a
//! Python interpreter, and never needs the code it reads to be importable —
//! which matters, because half the input is a blob checked out of git history.
//!
//! The walk descends into class bodies but **never** into function bodies. That
//! one rule delivers three requirements at once: methods come out as
//! `Class.method`, nested classes as `Outer.Inner`, and any line inside a
//! closure resolves to its enclosing named function, because the closure never
//! gets a span of its own.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tree_sitter::{Node, Parser};

/// What a changed line turned out to be part of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Function,
    Method,
    Class,
}

impl Kind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Method => "method",
            Self::Class => "class",
        }
    }
}

/// A definition and the lines it owns, 1-based and inclusive at both ends.
///
/// `start_line` is the first *decorator* line when the definition has any, so
/// editing a decorator attributes to the thing it decorates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolSpan {
    /// Dotted name within the module, e.g. `Enricher.run`.
    pub qualname: String,
    pub kind: Kind,
    pub start_line: u32,
    pub end_line: u32,
    /// Line of the definition's *name*, which is where a reference to it lands.
    ///
    /// Distinct from [`Self::start_line`], which is the first decorator: a
    /// position query and an `include_declaration` self-reference both mean the
    /// identifier, not the `@`.
    pub name_line: u32,
    /// 1-based column of the definition's name, as `tyf refs file:line:col`
    /// wants it.
    ///
    /// Byte-based, which matches character positions for every prefix Python
    /// allows here — indentation, `def `, `class `, `async def ` are all ASCII.
    pub name_column: u32,
    /// The decorators applied to this definition, in source order.
    pub decorators: Vec<Decorator>,
    /// Parameter names of a `def`, splats and separators excluded.
    ///
    /// Empty for a class. This is how a pytest fixture request is recognised:
    /// pytest injects by parameter *name*, so the list is the whole edge.
    pub params: Vec<String>,
}

impl SymbolSpan {
    #[must_use]
    pub const fn contains(&self, line: u32) -> bool {
        line >= self.start_line && line <= self.end_line
    }

    /// Dotted names of the decorators applied to this definition.
    pub fn decorator_names(&self) -> impl Iterator<Item = &str> {
        self.decorators.iter().map(|decorator| decorator.name.as_str())
    }

    /// The first decorator whose dotted name suffix-matches `entry`.
    ///
    /// The same syntactic matcher `ignore-decorators` uses — bare, called or
    /// dotted, with import aliases deliberately unresolved.
    #[must_use]
    pub fn decorator(&self, entry: &str) -> Option<&Decorator> {
        self.decorators.iter().find(|decorator| suffix_matches(&decorator.name, entry))
    }
}

/// One `@decorator`, reduced to the dotted name plus whatever literals it was
/// called with.
///
/// `@pytest.fixture`, `@pytest.fixture()` and `@pytest.fixture(name="x")` all
/// carry the same [`Self::name`]; only the last has arguments. Anything that is
/// not a string or a bool arrives as [`Literal::Other`], which is what makes a
/// dynamic `name=` distinguishable from an absent one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decorator {
    /// Dotted name being referenced: `registry.transformation`.
    pub name: String,
    /// Positional arguments, in order.
    pub args: Vec<Literal>,
    /// Keyword arguments, in order.
    pub kwargs: Vec<(String, Literal)>,
}

impl Decorator {
    /// The value passed for `key`, if the decorator was called with it.
    #[must_use]
    pub fn kwarg(&self, key: &str) -> Option<&Literal> {
        self.kwargs.iter().find(|(name, _)| name == key).map(|(_, value)| value)
    }

    /// Every positional argument that was a string literal.
    ///
    /// Non-literal arguments are dropped rather than approximated: the callers
    /// that need to know something was lost check the count against
    /// [`Self::args`].
    pub fn string_args(&self) -> impl Iterator<Item = &str> {
        self.args.iter().filter_map(Literal::as_str)
    }
}

/// An argument value, as far as reading the source can tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Literal {
    Str(String),
    Bool(bool),
    /// A name, a call, an f-string — anything whose value needs running Python.
    Other,
}

impl Literal {
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(value) => Some(value),
            _ => None,
        }
    }

    /// Whether this is the literal `True`.
    #[must_use]
    pub const fn is_true(&self) -> bool {
        matches!(self, Self::Bool(true))
    }
}

/// True when the dotted name ends with `entry` on a component boundary.
///
/// Shared by [`SymbolSpan::decorator`] and [`crate::config::Config`] so the
/// fixture rules and `ignore-decorators` cannot drift apart on what `@fixture`
/// matches.
#[must_use]
pub fn suffix_matches(dotted: &str, entry: &str) -> bool {
    dotted == entry || dotted.strip_suffix(entry).is_some_and(|prefix| prefix.ends_with('.'))
}

/// Every definition in one module, plus whether the parse was clean.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Module {
    pub spans: Vec<SymbolSpan>,
    /// Lines covered by an `import` or `from ... import` statement, wherever it
    /// sits. Parenthesised imports span several lines, hence ranges.
    pub imports: Vec<(u32, u32)>,
    /// The source did not parse cleanly. Callers treat the whole file as
    /// module-level rather than trusting a partial tree.
    pub has_error: bool,
}

impl Module {
    /// The innermost definition containing `line`, if any.
    ///
    /// Spans nest only class-inside-class and member-inside-class, so "deepest"
    /// is the same as "latest starting" among the spans that contain the line.
    #[must_use]
    pub fn symbol_at(&self, line: u32) -> Option<&SymbolSpan> {
        self.spans.iter().filter(|span| span.contains(line)).max_by_key(|span| span.start_line)
    }

    /// Whether `line` is part of an import statement.
    ///
    /// Callers consult this only for lines that belong to no symbol, so an
    /// import *inside* a function is never reached through here — it resolves
    /// to its enclosing definition first.
    #[must_use]
    pub fn is_import_line(&self, line: u32) -> bool {
        self.imports.iter().any(|&(start, end)| line >= start && line <= end)
    }

    /// Definitions at module level — those whose qualname has no dot.
    pub fn top_level(&self) -> impl Iterator<Item = &SymbolSpan> {
        self.spans.iter().filter(|span| !span.qualname.contains('.'))
    }
}

/// Parse Python source into its definition spans.
pub fn parse(source: &str) -> Result<Module> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .context("could not load the tree-sitter Python grammar")?;

    let tree = parser.parse(source, None).context("tree-sitter failed to parse the source")?;
    let root = tree.root_node();

    let mut spans = Vec::new();
    collect(root, source.as_bytes(), &[], false, &mut spans);
    spans.sort_by_key(|span| (span.start_line, span.end_line));

    let mut imports = Vec::new();
    collect_imports(root, &mut imports);
    imports.sort_unstable();

    Ok(Module { spans, imports, has_error: root.has_error() })
}

/// Line ranges of every import statement in the tree, at any nesting depth.
///
/// The whole tree is walked, unlike [`collect`]: an import inside `try:` or
/// `if TYPE_CHECKING:` is still an import line as far as the caller is
/// concerned, and one inside a function is simply never asked about.
fn collect_imports(node: Node, out: &mut Vec<(u32, u32)>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if matches!(
            child.kind(),
            "import_statement" | "import_from_statement" | "future_import_statement"
        ) {
            out.push((one_based(child.start_position().row), one_based(child.end_position().row)));
        } else {
            collect_imports(child, out);
        }
    }
}

/// Walk `node`'s children, emitting definitions and recursing everywhere except
/// into function bodies.
fn collect(node: Node, src: &[u8], prefix: &[String], in_class: bool, out: &mut Vec<SymbolSpan>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "decorated_definition" => {
                if let Some(def) = child.child_by_field_name("definition") {
                    emit(def, child, decorators(child, src), src, prefix, in_class, out);
                }
            }
            "function_definition" | "class_definition" => {
                emit(child, child, Vec::new(), src, prefix, in_class, out);
            }
            // `if TYPE_CHECKING:` and `try:` blocks can hold definitions that
            // are still top-level as far as the module is concerned.
            _ => collect(child, src, prefix, in_class, out),
        }
    }
}

/// Record one definition, then descend if it is a class.
///
/// `outer` is the node whose first line the span starts at: the
/// `decorated_definition` when there are decorators, otherwise `def` itself.
fn emit(
    def: Node,
    outer: Node,
    decorators: Vec<Decorator>,
    src: &[u8],
    prefix: &[String],
    in_class: bool,
    out: &mut Vec<SymbolSpan>,
) {
    let Some(name) = def.child_by_field_name("name").and_then(|n| n.utf8_text(src).ok()) else {
        return;
    };

    let defines_class = def.kind() == "class_definition";
    let kind = match (defines_class, in_class) {
        (true, _) => Kind::Class,
        (false, true) => Kind::Method,
        (false, false) => Kind::Function,
    };

    let mut qual = prefix.to_vec();
    qual.push(name.to_string());

    let name_node = def.child_by_field_name("name");
    let name_line = name_node
        .map_or_else(|| one_based(def.start_position().row), |n| one_based(n.start_position().row));
    let name_column = name_node.map_or(1, |n| one_based(n.start_position().column));

    out.push(SymbolSpan {
        qualname: qual.join("."),
        kind,
        start_line: one_based(outer.start_position().row),
        end_line: one_based(def.end_position().row),
        name_line,
        name_column,
        decorators,
        params: parameters(def, src),
    });

    if defines_class {
        if let Some(body) = def.child_by_field_name("body") {
            collect(body, src, &qual, true, out);
        }
    }
}

/// Every `@decorator` attached to a `decorated_definition`.
fn decorators(node: Node, src: &[u8]) -> Vec<Decorator> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| child.kind() == "decorator")
        .filter_map(|decorator| decorator.named_child(0))
        .filter_map(|expr| decorator(expr, src))
        .collect()
}

/// Reduce one decorator expression to its name and its literal arguments.
///
/// Arguments only exist for the call form; `@pytest.fixture` and
/// `@pytest.fixture()` differ in nothing else.
fn decorator(expr: Node, src: &[u8]) -> Option<Decorator> {
    let name = dotted_name(expr, src)?;
    let mut args = Vec::new();
    let mut kwargs = Vec::new();

    if expr.kind() == "call" {
        if let Some(list) = expr.child_by_field_name("arguments") {
            let mut cursor = list.walk();
            for child in list.named_children(&mut cursor) {
                if child.kind() == "keyword_argument" {
                    let key = child.child_by_field_name("name").and_then(|n| n.utf8_text(src).ok());
                    let value = child.child_by_field_name("value").map(|v| literal(v, src));
                    if let (Some(key), Some(value)) = (key, value) {
                        kwargs.push((key.to_string(), value));
                    }
                } else {
                    args.push(literal(child, src));
                }
            }
        }
    }

    Some(Decorator { name, args, kwargs })
}

/// The value of an expression, when reading it is enough to know it.
fn literal(node: Node, src: &[u8]) -> Literal {
    match node.kind() {
        "true" => Literal::Bool(true),
        "false" => Literal::Bool(false),
        "string" => string_value(node, src).map_or(Literal::Other, Literal::Str),
        _ => Literal::Other,
    }
}

/// The text inside a string literal, prefixes and quotes stripped.
///
/// `None` for anything interpolated: an f-string's `string_content` is only
/// part of its value, and half a name is worse than no name.
fn string_value(node: Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.named_children(&mut cursor).collect();
    if children.iter().any(|child| child.kind() == "interpolation") {
        return None;
    }
    let content: Vec<&str> = children
        .iter()
        .filter(|child| child.kind() == "string_content")
        .filter_map(|child| child.utf8_text(src).ok())
        .collect();
    match content.as_slice() {
        // An empty literal has no `string_content` child at all.
        [] => Some(String::new()),
        [single] => Some((*single).to_string()),
        _ => None,
    }
}

/// Parameter names of a `def`, in order.
///
/// `*args`, `**kwargs` and the `/` and `*` separators are skipped: none of them
/// can carry a pytest fixture request. A class has no parameter list, and so no
/// parameters.
fn parameters(def: Node, src: &[u8]) -> Vec<String> {
    let Some(list) = def.child_by_field_name("parameters") else { return Vec::new() };
    let mut cursor = list.walk();
    list.named_children(&mut cursor)
        .filter_map(|param| match param.kind() {
            "identifier" => param.utf8_text(src).ok().map(ToString::to_string),
            "default_parameter" | "typed_default_parameter" => param
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(src).ok())
                .map(ToString::to_string),
            // `x: int` — the name is the first child, there is no `name` field.
            "typed_parameter" => {
                param.named_child(0).and_then(|n| n.utf8_text(src).ok()).map(ToString::to_string)
            }
            _ => None,
        })
        .collect()
}

/// Reduce a decorator expression to the dotted name being referenced.
///
/// `@registry.transformation(name="x")` and `@registry.transformation` both
/// reduce to `registry.transformation`. Anything more exotic (a subscript, a
/// lambda) yields `None` and simply never matches the ignore list.
fn dotted_name(node: Node, src: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" => node.utf8_text(src).ok().map(ToString::to_string),
        "call" => dotted_name(node.child_by_field_name("function")?, src),
        "attribute" => {
            let object = dotted_name(node.child_by_field_name("object")?, src)?;
            let attribute = node.child_by_field_name("attribute")?.utf8_text(src).ok()?;
            Some(format!("{object}.{attribute}"))
        }
        _ => None,
    }
}

/// Lines on which `name` appears as a whole word, 1-based, in order.
///
/// Deliberately textual: it is the fallback for a symbol whose definition no
/// longer exists, so there is nothing left for a type checker to resolve. It
/// over-matches — comments, docstrings, same-named locals — and that is the
/// safe direction.
#[must_use]
pub fn word_lines(source: &str, name: &str) -> Vec<u32> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| word_appears(line, name))
        .filter_map(|(index, _)| u32::try_from(index).ok().map(|row| row.saturating_add(1)))
        .collect()
}

/// Whether `haystack` contains `needle` bounded by non-identifier characters.
fn word_appears(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let bytes = haystack.as_bytes();
    haystack.match_indices(needle).any(|(at, _)| {
        let before = at.checked_sub(1).map(|i| bytes[i]);
        let after = bytes.get(at + needle.len()).copied();
        !before.is_some_and(is_ident_byte) && !after.is_some_and(is_ident_byte)
    })
}

const fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Whether the source imports `module`, in any of the forms Python allows.
///
/// Also textual, and for the same reason: this answers "which test files would
/// re-run if this module's top level changed?", where the module may have no
/// symbol worth resolving at all. `from pkg import sub` is recognised as an
/// import of `pkg.sub`, because that is what it is.
#[must_use]
pub fn imports_module(source: &str, module: &str) -> bool {
    let (parent, leaf) = module.rsplit_once('.').unwrap_or(("", module));
    let submodule_prefix = format!("{module}.");

    source.lines().any(|line| {
        let line = line.trim();

        if let Some(rest) = line.strip_prefix("import ") {
            return rest.split(',').any(|part| {
                let name = part.split_whitespace().next().unwrap_or_default();
                name == module || name.starts_with(&submodule_prefix)
            });
        }

        let Some(rest) = line.strip_prefix("from ") else { return false };
        let Some((from, names)) = rest.split_once(" import ") else { return false };
        let from = from.trim();

        if from == module || from.starts_with(&submodule_prefix) {
            return true;
        }
        // `from pkg import sub` imports the module `pkg.sub`. A parenthesised
        // list continues on later lines, so an open bracket counts as a match
        // rather than a miss.
        !parent.is_empty()
            && from == parent
            && (word_appears(names, leaf) || names.trim_end().ends_with('('))
    })
}

/// tree-sitter rows and columns are 0-based; every line and column gerenuk
/// reports is 1-based, which is also what `tyf` positions use.
fn one_based(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(source: &str) -> Module {
        parse(source).expect("the grammar loads and tree-sitter always returns a tree")
    }

    /// The qualified name owning `line`, or `"<module>"` for module level.
    fn at(source: &str, line: u32) -> String {
        module(source)
            .symbol_at(line)
            .map_or_else(|| "<module>".to_string(), |span| span.qualname.clone())
    }

    /// The kind owning `line`, or `None` for module level.
    fn kind_at(source: &str, line: u32) -> Option<Kind> {
        module(source).symbol_at(line).map(|span| span.kind)
    }

    const SAMPLE: &str = "\
import os

CONSTANT = 3


def top(a, b=os.sep):
    \"\"\"Docstring.\"\"\"
    inner = a
    return inner


class Enricher:
    field: int = 0

    def __init__(self):
        self.x = 1

    @property
    def run(self):
        def helper():
            return 2

        return helper()

    class Inner:
        def deep(self):
            return 3


async def fetch():
    return None
";

    #[test]
    fn a_body_line_maps_to_its_function() {
        assert_eq!(at(SAMPLE, 8), "top", "`inner = a` is inside `top`");
        assert_eq!(kind_at(SAMPLE, 8), Some(Kind::Function), "a top-level def is a function");
    }

    #[test]
    fn the_signature_and_its_defaults_map_to_the_function() {
        assert_eq!(at(SAMPLE, 6), "top", "the `def` line belongs to the function it opens");
    }

    #[test]
    fn a_docstring_line_maps_to_its_function() {
        assert_eq!(at(SAMPLE, 7), "top", "docstrings are attributed like any other line");
    }

    #[test]
    fn a_method_is_qualified_by_its_class() {
        assert_eq!(at(SAMPLE, 16), "Enricher.__init__", "`self.x = 1` is inside __init__");
        assert_eq!(kind_at(SAMPLE, 16), Some(Kind::Method), "a def inside a class is a method");
    }

    #[test]
    fn a_class_body_line_outside_any_method_maps_to_the_class() {
        assert_eq!(at(SAMPLE, 13), "Enricher", "`field: int = 0` is a class attribute");
        assert_eq!(kind_at(SAMPLE, 13), Some(Kind::Class), "and its kind is the class");
    }

    #[test]
    fn the_class_statement_itself_maps_to_the_class() {
        assert_eq!(at(SAMPLE, 12), "Enricher", "the `class` line opens the class span");
    }

    #[test]
    fn a_decorator_line_maps_to_the_decorated_definition() {
        assert_eq!(
            at(SAMPLE, 18),
            "Enricher.run",
            "`@property` belongs to `run`, not to the class"
        );
    }

    #[test]
    fn a_closure_maps_to_its_enclosing_named_function() {
        assert_eq!(at(SAMPLE, 20), "Enricher.run", "the nested `def helper` line");
        assert_eq!(at(SAMPLE, 21), "Enricher.run", "and the closure's body");
    }

    #[test]
    fn a_nested_class_keeps_the_outer_class_in_its_qualname() {
        assert_eq!(at(SAMPLE, 25), "Enricher.Inner", "`class Inner` inside `Enricher`");
        assert_eq!(at(SAMPLE, 27), "Enricher.Inner.deep", "and its method nests one deeper");
        assert_eq!(
            kind_at(SAMPLE, 27),
            Some(Kind::Method),
            "a method of a nested class is a method"
        );
    }

    #[test]
    fn async_definitions_are_ordinary_functions() {
        assert_eq!(at(SAMPLE, 30), "fetch", "`async def` needs no special handling");
        assert_eq!(kind_at(SAMPLE, 31), Some(Kind::Function), "and its body too");
    }

    #[test]
    fn module_level_lines_belong_to_no_symbol() {
        assert_eq!(at(SAMPLE, 1), "<module>", "an import is module level");
        assert_eq!(at(SAMPLE, 3), "<module>", "so is a module constant");
        assert_eq!(at(SAMPLE, 11), "<module>", "so is a blank line between definitions");
        assert_eq!(at(SAMPLE, 29), "<module>", "and the gap after a class body");
    }

    #[test]
    fn a_definition_guarded_by_a_conditional_is_still_found() {
        let source = "\
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    def only_for_typing():
        return 1
";
        assert_eq!(
            at(source, 5),
            "only_for_typing",
            "definitions inside `if TYPE_CHECKING:` must not be lost"
        );
    }

    #[test]
    fn overloads_share_the_qualname_of_the_implementation() {
        let source = "\
from typing import overload


@overload
def f(x: int) -> int: ...


def f(x):
    return x
";
        assert_eq!(at(source, 4), "f", "the @overload stub");
        assert_eq!(at(source, 9), "f", "and the implementation");
    }

    #[test]
    fn decorators_are_recorded_as_dotted_names() {
        let source = "\
@transformation
def a():
    pass


@registry.transformation(name=\"x\")
def b():
    pass


@registry.sub.thing
class C:
    pass
";
        let spans = module(source).spans;
        let names = |name: &str| {
            spans
                .iter()
                .find(|s| s.qualname == name)
                .map(|s| s.decorator_names().map(ToString::to_string).collect::<Vec<_>>())
                .unwrap_or_default()
        };
        assert_eq!(names("a"), vec!["transformation".to_string()], "bare decorator");
        assert_eq!(
            names("b"),
            vec!["registry.transformation".to_string()],
            "call parentheses do not change the name"
        );
        assert_eq!(
            names("C"),
            vec!["registry.sub.thing".to_string()],
            "classes carry decorators too"
        );
    }

    #[test]
    fn every_decorator_on_a_definition_is_recorded() {
        let source = "\
@staticmethod
@transformation
@functools.cache
def a():
    pass
";
        let spans = module(source).spans;
        assert_eq!(
            spans[0].decorator_names().collect::<Vec<_>>(),
            vec!["staticmethod", "transformation", "functools.cache"],
            "all three must be visible so the ignore list can match any of them"
        );
        assert_eq!(spans[0].start_line, 1, "the span starts at the first decorator");
    }

    #[test]
    fn an_unparseable_decorator_expression_is_skipped_not_fatal() {
        let source = "\
@registry[\"key\"]
def a():
    pass
";
        let spans = module(source).spans;
        assert!(
            spans[0].decorators.is_empty(),
            "a subscript decorator reduces to no dotted name, got {:?}",
            spans[0].decorators
        );
        assert_eq!(spans[0].qualname, "a", "the definition itself is still found");
    }

    /// The span for `qualname`, which every fixture-rule test starts from.
    fn span(source: &str, qualname: &str) -> SymbolSpan {
        module(source)
            .spans
            .into_iter()
            .find(|s| s.qualname == qualname)
            .unwrap_or_else(|| panic!("`{qualname}` should be in the source"))
    }

    #[test]
    fn a_string_keyword_argument_is_read_off_the_decorator() {
        let source = "\
@pytest.fixture(name=\"shelter\", autouse=True, scope=\"session\")
def _make_shelter():
    pass
";
        let decorator = &span(source, "_make_shelter").decorators[0];
        assert_eq!(decorator.name, "pytest.fixture");
        assert_eq!(
            decorator.kwarg("name").and_then(Literal::as_str),
            Some("shelter"),
            "the injected name is the one the rules match on"
        );
        assert!(decorator.kwarg("autouse").is_some_and(Literal::is_true), "`autouse=True`");
        assert_eq!(decorator.kwarg("missing"), None, "an absent kwarg is absent");
    }

    #[test]
    fn a_non_literal_keyword_argument_is_recorded_as_unreadable() {
        // The distinction that matters: `Other` means "there is a name and we
        // cannot know it", which is a coarser fallback, not an absent kwarg.
        let source = "\
@pytest.fixture(name=NAME, autouse=flag)
def f():
    pass
";
        let decorator = &span(source, "f").decorators[0];
        assert_eq!(decorator.kwarg("name"), Some(&Literal::Other), "a variable is not a literal");
        assert!(
            !decorator.kwarg("autouse").is_some_and(Literal::is_true),
            "and a non-literal autouse is not True"
        );
    }

    #[test]
    fn positional_string_arguments_are_read_in_order() {
        let source = "\
@pytest.mark.usefixtures(\"shelter\", \"clock\")
def test_x():
    pass
";
        let decorator = &span(source, "test_x").decorators[0];
        assert_eq!(decorator.name, "pytest.mark.usefixtures");
        assert_eq!(decorator.string_args().collect::<Vec<_>>(), vec!["shelter", "clock"]);
    }

    #[test]
    fn an_f_string_argument_is_not_mistaken_for_its_literal_half() {
        let source = "\
@pytest.fixture(name=f\"{prefix}_shelter\")
def f():
    pass
";
        let decorator = &span(source, "f").decorators[0];
        assert_eq!(
            decorator.kwarg("name"),
            Some(&Literal::Other),
            "half an interpolated name is worse than no name"
        );
    }

    #[test]
    fn a_bare_decorator_has_no_arguments_at_all() {
        let decorator = &span("@fixture\ndef f():\n    pass\n", "f").decorators[0];
        assert!(decorator.args.is_empty() && decorator.kwargs.is_empty(), "nothing was called");
        assert_eq!(decorator.kwarg("name"), None, "so there is no name override");
    }

    #[test]
    fn parameters_are_listed_in_order_without_splats_or_separators() {
        let source = "\
def f(shelter, clock: Clock, seed=1, count: int = 2, /, *args, key=None, **kwargs):
    pass
";
        assert_eq!(
            span(source, "f").params,
            vec!["shelter", "clock", "seed", "count", "key"],
            "a splat cannot carry a fixture request; a named parameter can"
        );
    }

    #[test]
    fn a_method_lists_self_like_any_other_parameter() {
        let source = "\
class TestThing:
    def test_x(self, shelter):
        pass
";
        assert_eq!(span(source, "TestThing.test_x").params, vec!["self", "shelter"]);
        assert!(span(source, "TestThing").params.is_empty(), "a class has no parameter list");
    }

    #[test]
    fn the_decorator_matcher_is_the_one_the_ignore_list_uses() {
        let source = "\
@pytest.fixture
def a():
    pass


@fixture
def b():
    pass


@other.thing
def c():
    pass
";
        assert!(span(source, "a").decorator("pytest.fixture").is_some(), "the dotted form");
        assert!(span(source, "a").decorator("fixture").is_some(), "and its suffix");
        assert!(span(source, "b").decorator("fixture").is_some(), "the bare form");
        assert!(span(source, "b").decorator("pytest.fixture").is_none(), "but not the other way");
        assert!(span(source, "c").decorator("fixture").is_none(), "an unrelated decorator");
        assert!(!suffix_matches("prefixture", "fixture"), "the boundary must be a dot");
    }

    #[test]
    fn the_name_line_skips_the_decorators() {
        let source = "\
@staticmethod
@registry.thing
def a():
    pass
";
        let span = &module(source).spans[0];
        assert_eq!(span.start_line, 1, "the span still opens at the first decorator");
        assert_eq!(
            span.name_line, 3,
            "but the name is on the `def` line, which is where references land"
        );
        assert_eq!(span.name_column, 5, "`a` sits at 1-based column 5 of `def a():`");
    }

    #[test]
    fn a_methods_name_column_includes_its_indentation() {
        // A position query with the wrong column silently returns no
        // references, so this has to be exact rather than approximately right.
        let source = "\
class C:
    def m(self):
        pass
";
        let span = module(source)
            .spans
            .into_iter()
            .find(|s| s.qualname == "C.m")
            .expect("the method is found");
        assert_eq!((span.name_line, span.name_column), (2, 9), "4 spaces + `def ` + 1");
    }

    #[test]
    fn an_undecorated_definition_starts_where_its_name_is() {
        let spans = module(SAMPLE).spans;
        let top = spans.iter().find(|s| s.qualname == "top").expect("`top` is in the sample");
        assert_eq!(top.name_line, top.start_line, "no decorators means the two coincide");
        assert_eq!(top.name_line, 6, "and it is the `def top(...)` line");
    }

    #[test]
    fn import_statements_are_recorded_by_line() {
        let source = "\
import os
from pathlib import Path

x = 1
";
        let parsed = module(source);
        assert!(parsed.is_import_line(1), "`import os`");
        assert!(parsed.is_import_line(2), "`from ... import ...`");
        assert!(!parsed.is_import_line(3), "a blank line is not an import");
        assert!(!parsed.is_import_line(4), "an assignment is not an import");
    }

    #[test]
    fn a_parenthesised_import_covers_every_line_it_spans() {
        let source = "\
from pkg import (
    a,
    b,
)

y = 2
";
        let parsed = module(source);
        for line in 1..=4 {
            assert!(parsed.is_import_line(line), "line {line} is inside the import");
        }
        assert!(!parsed.is_import_line(6), "the assignment below is not");
    }

    #[test]
    fn a_conditional_import_is_still_an_import_line() {
        let source = "\
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from pkg import Thing
";
        assert!(module(source).is_import_line(4), "`if TYPE_CHECKING:` guards imports too");
    }

    #[test]
    fn an_import_inside_a_function_resolves_to_the_function_first() {
        // Both are true of line 2; callers ask `symbol_at` first, so the import
        // flag never gets consulted for it.
        let source = "\
def f():
    import os

    return os
";
        let parsed = module(source);
        assert_eq!(
            parsed.symbol_at(2).map(|s| s.qualname.as_str()),
            Some("f"),
            "the enclosing definition wins"
        );
        assert!(parsed.is_import_line(2), "the line is nonetheless an import");
    }

    #[test]
    fn top_level_lists_module_scope_definitions_only() {
        let parsed = module(SAMPLE);
        let names: Vec<&str> = parsed.top_level().map(|s| s.qualname.as_str()).collect();
        assert_eq!(
            names,
            vec!["top", "Enricher", "fetch"],
            "methods and nested classes carry a dot and are excluded"
        );
    }

    #[test]
    fn a_module_with_no_definitions_has_no_top_level_symbols() {
        assert_eq!(module("import os\n\nX = 1\n").top_level().count(), 0, "constants are not defs");
    }

    #[test]
    fn word_lines_finds_whole_word_occurrences_only() {
        let source = "\
def enrich():
    pass


enriched = 1
value = enrich()
# enrich again
";
        assert_eq!(
            word_lines(source, "enrich"),
            vec![1, 6, 7],
            "`enriched` must not match, but a comment mention does"
        );
    }

    #[test]
    fn word_lines_reports_a_line_once_however_often_it_appears() {
        assert_eq!(word_lines("f(f(f()))\n", "f"), vec![1], "one hit per line");
    }

    #[test]
    fn word_boundaries_include_dots_and_brackets() {
        assert!(word_appears("obj.run()", "run"), "an attribute access is a hit");
        assert!(word_appears("[run]", "run"), "so is a list element");
        assert!(!word_appears("rerun()", "run"), "but not a longer identifier");
        assert!(!word_appears("run_all()", "run"), "nor a prefix of one");
        assert!(!word_appears("anything", ""), "an empty needle never matches");
    }

    #[test]
    fn plain_imports_are_recognised() {
        assert!(imports_module("import mypkg.enrich\n", "mypkg.enrich"));
        assert!(imports_module("import os, mypkg.enrich\n", "mypkg.enrich"), "comma lists");
        assert!(
            imports_module("import mypkg.enrich as e\n", "mypkg.enrich"),
            "an alias does not hide the module"
        );
        assert!(
            imports_module("import mypkg.enrich.deep\n", "mypkg.enrich"),
            "importing a submodule imports the package on the way"
        );
        assert!(!imports_module("import mypkg.enricher\n", "mypkg.enrich"), "prefix collision");
    }

    #[test]
    fn from_imports_are_recognised_in_both_shapes() {
        assert!(
            imports_module("from mypkg.enrich import Enricher\n", "mypkg.enrich"),
            "the module named directly"
        );
        assert!(
            imports_module("from mypkg import enrich\n", "mypkg.enrich"),
            "`from pkg import sub` is an import of pkg.sub"
        );
        assert!(
            imports_module("from mypkg import (\n    enrich,\n)\n", "mypkg.enrich"),
            "a parenthesised list continues elsewhere, so it counts"
        );
        assert!(
            !imports_module("from mypkg import other\n", "mypkg.enrich"),
            "a sibling module is not this one"
        );
    }

    #[test]
    fn unrelated_source_imports_nothing() {
        let source = "\
x = 1


def f():
    return x
";
        assert!(!imports_module(source, "mypkg.enrich"), "no import statement, no import");
        assert!(!imports_module("# import mypkg.enrich\n", "mypkg.enrich"), "a comment is not one");
    }

    #[test]
    fn a_syntax_error_is_reported_rather_than_thrown() {
        let parsed = module("def broken(:\n    pass\n");
        assert!(parsed.has_error, "tree-sitter must tell us the tree is untrustworthy");
    }

    #[test]
    fn clean_source_reports_no_error() {
        assert!(!module(SAMPLE).has_error, "valid Python parses cleanly");
    }

    #[test]
    fn an_empty_module_has_no_spans() {
        let parsed = module("");
        assert!(parsed.spans.is_empty(), "nothing to find");
        assert!(!parsed.has_error, "an empty file is valid Python");
    }

    #[test]
    fn spans_come_out_in_source_order() {
        let starts: Vec<u32> = module(SAMPLE).spans.iter().map(|s| s.start_line).collect();
        let mut sorted = starts.clone();
        sorted.sort_unstable();
        assert_eq!(starts, sorted, "callers rely on deterministic ordering");
    }

    #[test]
    fn crlf_line_endings_attribute_the_same_as_lf() {
        // A Windows checkout with `core.autocrlf=true` hands us \r\n, and the
        // blob out of git history does not.
        let crlf = SAMPLE.replace('\n', "\r\n");
        assert_eq!(at(&crlf, 8), at(SAMPLE, 8), "a function body resolves the same");
        assert_eq!(at(&crlf, 16), at(SAMPLE, 16), "and so does a method body");
    }
}
