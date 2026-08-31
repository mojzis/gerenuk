//! Turning `tyf` answers into gerenuk findings.
//!
//! This module is pure: it takes already-parsed [`crate::model`] values and
//! returns [`Finding`]s. Nothing here spawns a process, so the rules are
//! unit-testable without `tyf` installed.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{walk_outline, DocumentSymbol, Reference, ReferencesResult, SymbolKind};
use crate::workspace::is_test_path;

/// How serious a [`Finding`] is. Ordered least to most severe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Note,
    Warn,
}

impl Severity {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Warn => "warn",
        }
    }
}

/// What gerenuk noticed about one symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// Dotted symbol name, e.g. `Calculator.add`.
    pub symbol: String,
    pub kind: SymbolKind,
    pub file: PathBuf,
    /// One-based line of the symbol's name.
    pub line: u32,
    pub severity: Severity,
    pub message: String,
}

/// A symbol from a file outline, paired with the usages `tyf refs` reported.
///
/// Callers assemble these (one `tyf refs` call per symbol); [`audit`] consumes
/// them without knowing where they came from.
#[derive(Debug, Clone)]
pub struct SymbolUsage {
    pub name: String,
    pub kind: SymbolKind,
    pub line: u32,
    pub refs: ReferencesResult,
    /// Dotted names of the decorators applied to it, when the file could be
    /// parsed. A registering decorator is what makes "no references" a lie.
    pub decorators: Vec<String>,
}

/// Flatten a `tyf list` outline into the callable symbols worth auditing.
///
/// Dunder methods and private helpers (`_name`) are skipped: they are usually
/// referenced implicitly or deliberately internal, so flagging them is noise.
#[must_use]
pub fn auditable_symbols(outline: &[DocumentSymbol]) -> Vec<(String, SymbolKind, u32)> {
    let mut out = Vec::new();
    for top in outline {
        for symbol in top.walk() {
            if !symbol.kind.is_callable() || !is_auditable_name(&symbol.name) {
                continue;
            }
            let name = if std::ptr::eq(symbol, top) {
                symbol.name.clone()
            } else {
                top.qualified_child(symbol)
            };
            out.push((name, symbol.kind, symbol.selection_range.start.line + 1));
        }
    }
    out
}

/// Count every symbol in an outline, nested ones included.
#[must_use]
pub fn outline_size(outline: &[DocumentSymbol]) -> usize {
    walk_outline(outline).count()
}

fn is_auditable_name(name: &str) -> bool {
    !name.starts_with('_')
}

/// Apply gerenuk's rules to the symbols of one file.
///
/// Two rules today:
///
/// * a callable with no references at all is likely dead (`warn`);
/// * a callable referenced only from test files is untethered from production
///   code (`note`) — often a leftover after a refactor.
#[must_use]
pub fn audit(file: &Path, root: &Path, symbols: &[SymbolUsage]) -> Vec<Finding> {
    symbols
        .iter()
        .filter_map(|usage| {
            let (severity, message) = classify(usage, file, root)?;
            Some(Finding {
                symbol: usage.name.clone(),
                kind: usage.kind,
                file: file.to_path_buf(),
                line: usage.line,
                severity,
                message,
            })
        })
        .collect()
}

/// Split a symbol's references into production and test counts.
///
/// `tyf` buckets references itself, but its test heuristic looks at the whole
/// absolute path — so a project living under a `tests/` directory has every
/// reference filed as a test. When the returned lists are complete we re-derive
/// both counts from the paths, relative to `root`, and only fall back to
/// `tyf`'s own counts when the lists are truncated or withheld.
///
/// The symbol's own definition is not a usage, so a reference landing on
/// `definition` is dropped.
fn split_refs(usage: &SymbolUsage, file: &Path, root: &Path) -> (usize, usize) {
    let reported = usage.refs.reference_count + usage.refs.test_reference_count;
    let listed: Vec<&Reference> =
        usage.refs.references.iter().chain(&usage.refs.test_references).collect();

    if listed.len() < reported {
        // Lists are incomplete; trust the counts, imperfect buckets and all.
        return (usage.refs.reference_count, usage.refs.test_reference_count);
    }

    let usages = listed.into_iter().filter(|r| !is_definition(r, usage, file));
    let (tests, production): (Vec<_>, Vec<_>) = usages.partition(|r| is_test_path(&r.file, root));
    (production.len(), tests.len())
}

/// Whether a reference points at the symbol's own definition line.
fn is_definition(reference: &Reference, usage: &SymbolUsage, file: &Path) -> bool {
    reference.line == usage.line && reference.file.ends_with(file)
}

fn classify(usage: &SymbolUsage, file: &Path, root: &Path) -> Option<(Severity, String)> {
    let (production, tests) = split_refs(usage, file, root);

    if production > 0 {
        return None;
    }

    // A framework holds the reference to a registered function: `@app.command`
    // and `@router.get` are called by Typer and FastAPI, and no reference to
    // them exists to be counted. Reporting these as dead is not a near-miss —
    // it is every CLI command and every route in the project, which buries the
    // findings that are real. Skipped for the same reason `_private` names are.
    if usage.decorators.iter().any(|d| !crate::closure::is_inert(d)) {
        return None;
    }

    if tests == 0 {
        Some((Severity::Warn, format!("`{}` has no references", usage.name)))
    } else {
        Some((Severity::Note, format!("`{}` is referenced only from tests ({tests})", usage.name)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Position, Range};

    fn refs(name: &str, production: &[&str], tests: &[&str]) -> ReferencesResult {
        let make = |paths: &[&str]| -> Vec<Reference> {
            paths
                .iter()
                .enumerate()
                .map(|(i, p)| Reference {
                    file: PathBuf::from(p),
                    line: u32::try_from(i).expect("fixture indices are small") + 1,
                    column: 1,
                    context: "caller".to_string(),
                })
                .collect()
        };
        let production = make(production);
        let test_references = make(tests);
        ReferencesResult {
            symbol: name.to_string(),
            reference_count: production.len(),
            references: production,
            test_reference_count: test_references.len(),
            test_references,
        }
    }

    fn usage(name: &str, refs: ReferencesResult) -> SymbolUsage {
        SymbolUsage {
            name: name.to_string(),
            kind: SymbolKind::Function,
            line: 10,
            refs,
            decorators: Vec::new(),
        }
    }

    fn span(line: u32) -> Range {
        Range {
            start: Position { line, character: 0 },
            end: Position { line: line + 1, character: 0 },
        }
    }

    fn symbol(name: &str, kind: u8, line: u32, children: Vec<DocumentSymbol>) -> DocumentSymbol {
        DocumentSymbol {
            name: name.to_string(),
            kind: SymbolKind::from(kind),
            range: span(line),
            selection_range: span(line),
            children,
        }
    }

    const ROOT: &str = "/proj";

    #[test]
    fn a_symbol_with_production_refs_produces_no_finding() {
        let findings = audit(
            Path::new("pkg/service.py"),
            Path::new(ROOT),
            &[usage("used", refs("used", &["pkg/main.py"], &[]))],
        );
        assert!(findings.is_empty(), "a used symbol should not be reported, got {findings:?}");
    }

    #[test]
    fn a_symbol_with_no_refs_is_warned_about() {
        let findings = audit(
            Path::new("pkg/service.py"),
            Path::new(ROOT),
            &[usage("orphan", refs("orphan", &[], &[]))],
        );
        assert_eq!(findings.len(), 1, "an unreferenced symbol yields exactly one finding");
        assert_eq!(
            findings[0].severity,
            Severity::Warn,
            "no references at all is the stronger signal"
        );
        assert_eq!(findings[0].symbol, "orphan");
        assert_eq!(
            findings[0].file,
            Path::new("pkg/service.py"),
            "findings carry the audited file"
        );
        assert_eq!(findings[0].line, 10, "findings carry the symbol's line");
        assert!(findings[0].message.contains("no references"), "message should say what is wrong");
    }

    #[test]
    fn a_symbol_used_only_by_tests_is_a_note() {
        let findings = audit(
            Path::new("pkg/service.py"),
            Path::new(ROOT),
            &[usage("helper", refs("helper", &[], &["tests/test_service.py"]))],
        );
        assert_eq!(findings.len(), 1, "a test-only symbol yields one finding");
        assert_eq!(findings[0].severity, Severity::Note, "test-only is weaker than fully unused");
        assert!(findings[0].message.contains("only from tests"), "message should name the cause");
    }

    #[test]
    fn refs_that_live_in_test_files_do_not_count_as_production() {
        // tyf sometimes reports test hits in `references` rather than
        // `test_references`; path-based classification must still catch them.
        let findings = audit(
            Path::new("pkg/service.py"),
            Path::new(ROOT),
            &[usage("helper", refs("helper", &["tests/test_service.py"], &[]))],
        );
        assert_eq!(findings.len(), 1, "a ref inside tests/ is not a production use");
        assert_eq!(findings[0].severity, Severity::Note, "it is still a test-only symbol");
    }

    #[test]
    fn findings_preserve_input_order() {
        let findings = audit(
            Path::new("pkg/service.py"),
            Path::new(ROOT),
            &[
                usage("a", refs("a", &[], &[])),
                usage("b", refs("b", &["pkg/main.py"], &[])),
                usage("c", refs("c", &[], &["tests/test_x.py"])),
            ],
        );
        let names: Vec<&str> = findings.iter().map(|f| f.symbol.as_str()).collect();
        assert_eq!(names, vec!["a", "c"], "used symbols drop out, order is otherwise preserved");
    }

    #[test]
    fn a_test_reference_count_without_a_list_is_still_a_test_only_symbol() {
        // Real `tyf` reports `test_reference_count: 7` with an empty
        // `test_references` array unless `--tests` was passed. Trusting the
        // list length alone would misreport this as fully unused.
        let refs = ReferencesResult {
            symbol: "describe".to_string(),
            reference_count: 0,
            references: vec![],
            test_reference_count: 7,
            test_references: vec![],
        };
        let findings =
            audit(Path::new("pkg/service.py"), Path::new(ROOT), &[usage("describe", refs)]);
        assert_eq!(findings.len(), 1, "the symbol should still be reported");
        assert_eq!(
            findings[0].severity,
            Severity::Note,
            "a nonzero test_reference_count means test-only, not unused"
        );
        assert!(
            findings[0].message.contains("(7)"),
            "the count should come from tyf, got: {}",
            findings[0].message
        );
    }

    #[test]
    fn a_reference_count_without_a_list_still_counts_as_used() {
        // `--references-limit` can truncate the list; the count is the truth.
        let refs = ReferencesResult {
            symbol: "popular".to_string(),
            reference_count: 40,
            references: vec![],
            test_reference_count: 0,
            test_references: vec![],
        };
        let findings =
            audit(Path::new("pkg/service.py"), Path::new(ROOT), &[usage("popular", refs)]);
        assert!(
            findings.is_empty(),
            "a truncated list must not read as zero references, got {findings:?}"
        );
    }

    #[test]
    fn auditable_symbols_qualifies_methods_and_skips_private_ones() {
        let outline = vec![
            symbol(
                "Calculator",
                5,
                0,
                vec![
                    symbol("add", 6, 2, vec![]),
                    symbol("_carry", 6, 5, vec![]),
                    symbol("__init__", 6, 8, vec![]),
                ],
            ),
            symbol("main", 12, 20, vec![]),
            symbol("CONFIG", 14, 25, vec![]),
        ];

        let found: Vec<(String, u32)> =
            auditable_symbols(&outline).into_iter().map(|(name, _, line)| (name, line)).collect();

        assert_eq!(
            found,
            vec![("Calculator.add".to_string(), 3), ("main".to_string(), 21)],
            "methods are qualified, private/dunder names and non-callables are skipped"
        );
    }

    #[test]
    fn outline_size_counts_nested_symbols() {
        let outline =
            vec![symbol("C", 5, 0, vec![symbol("a", 6, 1, vec![]), symbol("b", 6, 2, vec![])])];
        assert_eq!(outline_size(&outline), 3, "one class plus two methods");
    }

    #[test]
    fn severity_orders_notes_below_warnings() {
        assert!(Severity::Note < Severity::Warn, "warn must sort as more severe than note");
    }
}
