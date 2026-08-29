//! Wire types for the JSON that `tyf --format json` emits.
//!
//! These mirror the LSP shapes `tyf` passes through, so the field names are
//! LSP's (`uri`, `range`, `character`, `selectionRange`) rather than ours.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Zero-based line/column pair, as LSP reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

/// Half-open span between two [`Position`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

impl Range {
    /// Number of source lines the range covers, minimum 1.
    #[must_use]
    pub fn line_span(&self) -> u32 {
        self.end.line.saturating_sub(self.start.line) + 1
    }
}

/// A definition site, as returned by `tyf find`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub uri: String,
    pub range: Range,
}

impl Location {
    /// Filesystem path behind [`Self::uri`].
    ///
    /// `tyf` always emits `file://` URIs; anything else is returned verbatim so
    /// callers still get something displayable instead of an error.
    #[must_use]
    pub fn path(&self) -> PathBuf {
        PathBuf::from(self.uri.strip_prefix("file://").unwrap_or(&self.uri))
    }

    /// One-based line number, for display alongside editor conventions.
    #[must_use]
    pub fn display_line(&self) -> u32 {
        self.range.start.line + 1
    }
}

/// A single usage site, as returned by `tyf refs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reference {
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
    /// Enclosing symbol name, when `tyf` could determine one.
    #[serde(default)]
    pub context: String,
}

/// Full `tyf refs` answer for one symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferencesResult {
    pub symbol: String,
    #[serde(default)]
    pub reference_count: usize,
    #[serde(default)]
    pub references: Vec<Reference>,
    #[serde(default)]
    pub test_reference_count: usize,
    #[serde(default)]
    pub test_references: Vec<Reference>,
}

impl ReferencesResult {
    /// References outside test files — the ones that constrain a refactor.
    #[must_use]
    pub fn production_refs(&self) -> &[Reference] {
        &self.references
    }

    /// True when nothing but tests uses the symbol.
    #[must_use]
    pub fn only_used_by_tests(&self) -> bool {
        self.references.is_empty() && !self.test_references.is_empty()
    }
}

/// LSP `SymbolKind`, restricted to the variants `tyf list` actually emits.
///
/// Unknown numeric kinds map to [`SymbolKind::Other`] rather than failing the
/// whole parse — new `ty` releases may add kinds we have not seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "u8")]
pub enum SymbolKind {
    Class,
    Function,
    Method,
    Property,
    Variable,
    Constant,
    Other,
}

impl From<u8> for SymbolKind {
    fn from(raw: u8) -> Self {
        match raw {
            5 => Self::Class,
            6 => Self::Method,
            7 => Self::Property,
            12 => Self::Function,
            13 => Self::Variable,
            14 => Self::Constant,
            _ => Self::Other,
        }
    }
}

impl SymbolKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Class => "class",
            Self::Function => "func",
            Self::Method => "method",
            Self::Property => "property",
            Self::Variable => "var",
            Self::Constant => "const",
            Self::Other => "symbol",
        }
    }

    /// Whether the symbol is callable, and so worth checking for usages.
    #[must_use]
    pub const fn is_callable(self) -> bool {
        matches!(self, Self::Function | Self::Method)
    }
}

/// One entry of a `tyf list` document outline. Nested via [`Self::children`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub range: Range,
    #[serde(rename = "selectionRange")]
    pub selection_range: Range,
    #[serde(default)]
    pub children: Vec<Self>,
}

impl DocumentSymbol {
    /// Depth-first walk over `self` and every descendant.
    pub fn walk(&self) -> impl Iterator<Item = &Self> {
        let mut stack = vec![self];
        std::iter::from_fn(move || {
            let node = stack.pop()?;
            stack.extend(node.children.iter().rev());
            Some(node)
        })
    }

    /// Dotted name of a child relative to this symbol, e.g. `Calculator.add`.
    #[must_use]
    pub fn qualified_child(&self, child: &Self) -> String {
        format!("{}.{}", self.name, child.name)
    }
}

/// Depth-first walk over a whole outline.
pub fn walk_outline(outline: &[DocumentSymbol]) -> impl Iterator<Item = &DocumentSymbol> {
    outline.iter().flat_map(DocumentSymbol::walk)
}

/// Render a path relative to `root` when possible, else absolute.
#[must_use]
pub fn relative_display(path: &Path, root: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_uri_becomes_a_filesystem_path() {
        let loc = Location {
            uri: "file:///home/u/proj/main.py".to_string(),
            range: Range {
                start: Position { line: 13, character: 0 },
                end: Position { line: 16, character: 29 },
            },
        };
        assert_eq!(
            loc.path(),
            PathBuf::from("/home/u/proj/main.py"),
            "file:// prefix should be stripped"
        );
        assert_eq!(loc.display_line(), 14, "display lines are one-based");
        assert_eq!(loc.range.line_span(), 4, "13..=16 inclusive is 4 lines");
    }

    #[test]
    fn location_without_a_file_scheme_is_passed_through() {
        let loc = Location {
            uri: "untitled:Untitled-1".to_string(),
            range: Range {
                start: Position { line: 0, character: 0 },
                end: Position { line: 0, character: 1 },
            },
        };
        assert_eq!(
            loc.path(),
            PathBuf::from("untitled:Untitled-1"),
            "non-file URIs are kept verbatim"
        );
    }

    #[test]
    fn single_line_range_spans_one_line() {
        let range = Range {
            start: Position { line: 4, character: 0 },
            end: Position { line: 4, character: 12 },
        };
        assert_eq!(range.line_span(), 1, "a range inside one line spans one line");
    }

    #[test]
    fn unknown_symbol_kinds_degrade_instead_of_failing() {
        assert_eq!(SymbolKind::from(12), SymbolKind::Function);
        assert_eq!(SymbolKind::from(5), SymbolKind::Class);
        assert_eq!(
            SymbolKind::from(99),
            SymbolKind::Other,
            "unseen LSP kinds must not break parsing"
        );
    }

    #[test]
    fn only_functions_and_methods_are_callable() {
        assert!(SymbolKind::Function.is_callable(), "functions are callable");
        assert!(SymbolKind::Method.is_callable(), "methods are callable");
        assert!(!SymbolKind::Class.is_callable(), "classes are not counted as callable here");
        assert!(!SymbolKind::Variable.is_callable(), "variables are not callable");
    }

    #[test]
    fn references_result_distinguishes_test_only_symbols() {
        let test_ref = Reference {
            file: PathBuf::from("tests/test_x.py"),
            line: 3,
            column: 5,
            context: "test_x".to_string(),
        };
        let test_only = ReferencesResult {
            symbol: "helper".to_string(),
            reference_count: 0,
            references: vec![],
            test_reference_count: 1,
            test_references: vec![test_ref],
        };
        assert!(test_only.only_used_by_tests(), "no production refs but a test ref is test-only");
        assert!(test_only.production_refs().is_empty(), "production refs must exclude test refs");

        let unused = ReferencesResult {
            symbol: "dead".to_string(),
            reference_count: 0,
            references: vec![],
            test_reference_count: 0,
            test_references: vec![],
        };
        assert!(!unused.only_used_by_tests(), "a symbol with no refs at all is not test-only");
    }

    #[test]
    fn walk_visits_parents_before_children_in_source_order() {
        let outline: Vec<DocumentSymbol> = serde_json::from_str(
            r#"[{
                "name": "Calculator", "kind": 5,
                "range": {"start": {"line": 0, "character": 0}, "end": {"line": 10, "character": 0}},
                "selectionRange": {"start": {"line": 0, "character": 6}, "end": {"line": 0, "character": 16}},
                "children": [
                  {"name": "add", "kind": 6,
                   "range": {"start": {"line": 2, "character": 4}, "end": {"line": 3, "character": 20}},
                   "selectionRange": {"start": {"line": 2, "character": 8}, "end": {"line": 2, "character": 11}},
                   "children": []},
                  {"name": "sub", "kind": 6,
                   "range": {"start": {"line": 5, "character": 4}, "end": {"line": 6, "character": 20}},
                   "selectionRange": {"start": {"line": 5, "character": 8}, "end": {"line": 5, "character": 11}},
                   "children": []}
                ]
            }]"#,
        )
        .expect("outline fixture should deserialize");

        let names: Vec<&str> = walk_outline(&outline).map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Calculator", "add", "sub"],
            "walk should be parent-first, source-ordered"
        );
        assert_eq!(outline[0].qualified_child(&outline[0].children[0]), "Calculator.add");
    }
}
