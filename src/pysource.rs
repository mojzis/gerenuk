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
    /// Dotted names of the decorators applied to this definition.
    pub decorators: Vec<String>,
}

impl SymbolSpan {
    #[must_use]
    pub const fn contains(&self, line: u32) -> bool {
        line >= self.start_line && line <= self.end_line
    }
}

/// Every definition in one module, plus whether the parse was clean.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Module {
    pub spans: Vec<SymbolSpan>,
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

    Ok(Module { spans, has_error: root.has_error() })
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
    decorators: Vec<String>,
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

    out.push(SymbolSpan {
        qualname: qual.join("."),
        kind,
        start_line: line_of(outer.start_position().row),
        end_line: line_of(def.end_position().row),
        decorators,
    });

    if defines_class {
        if let Some(body) = def.child_by_field_name("body") {
            collect(body, src, &qual, true, out);
        }
    }
}

/// Dotted names of every `@decorator` attached to a `decorated_definition`.
fn decorators(node: Node, src: &[u8]) -> Vec<String> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| child.kind() == "decorator")
        .filter_map(|decorator| decorator.named_child(0))
        .filter_map(|expr| dotted_name(expr, src))
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

/// tree-sitter rows are 0-based; every line number gerenuk reports is 1-based.
fn line_of(row: usize) -> u32 {
    u32::try_from(row).unwrap_or(u32::MAX).saturating_add(1)
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
        let decorators = |name: &str| {
            spans
                .iter()
                .find(|s| s.qualname == name)
                .map(|s| s.decorators.clone())
                .unwrap_or_default()
        };
        assert_eq!(decorators("a"), vec!["transformation".to_string()], "bare decorator");
        assert_eq!(
            decorators("b"),
            vec!["registry.transformation".to_string()],
            "call parentheses and arguments are dropped"
        );
        assert_eq!(
            decorators("C"),
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
            spans[0].decorators,
            vec![
                "staticmethod".to_string(),
                "transformation".to_string(),
                "functools.cache".to_string()
            ],
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
