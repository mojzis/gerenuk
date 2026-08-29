//! Rendering findings for humans and for machines.
//!
//! Both renderers are pure string builders so they can be asserted on directly
//! in tests, without capturing process output.

use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::analyze::{Finding, Severity};
use crate::model::relative_display;

/// Output shape selected by `--format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Format {
    /// Aligned, human-readable lines.
    #[default]
    Human,
    /// A single JSON object, for piping into other tools.
    Json,
}

/// Everything one audit run produced.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// Files that were audited, relative to the workspace root.
    pub files: Vec<String>,
    /// Total symbols considered across those files.
    pub symbols_checked: usize,
    pub findings: Vec<Finding>,
}

impl Report {
    #[must_use]
    pub fn new(files: Vec<String>, symbols_checked: usize, findings: Vec<Finding>) -> Self {
        Self { files, symbols_checked, findings }
    }

    /// True when nothing was flagged — the caller's exit-code signal.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }

    #[must_use]
    pub fn count(&self, severity: Severity) -> usize {
        self.findings.iter().filter(|f| f.severity == severity).count()
    }

    pub fn render(&self, format: Format, root: &Path) -> Result<String> {
        match format {
            Format::Human => Ok(self.render_human(root)),
            Format::Json => Ok(serde_json::to_string_pretty(self)?),
        }
    }

    fn render_human(&self, root: &Path) -> String {
        use std::fmt::Write as _;

        let mut out = String::new();

        for finding in &self.findings {
            let location = format!("{}:{}", relative_display(&finding.file, root), finding.line);
            // Writing into a String is infallible, so the Result carries nothing.
            let _ = writeln!(
                out,
                "{:<5} {location}  {} {}",
                finding.severity.label(),
                finding.kind.label(),
                finding.message,
            );
        }

        if self.findings.is_empty() {
            out.push_str("No findings.\n");
        }

        let _ = writeln!(
            out,
            "\n{} file(s), {} symbol(s) checked — {} warn, {} note",
            self.files.len(),
            self.symbols_checked,
            self.count(Severity::Warn),
            self.count(Severity::Note),
        );

        out
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::model::SymbolKind;

    fn finding(symbol: &str, severity: Severity, line: u32) -> Finding {
        Finding {
            symbol: symbol.to_string(),
            kind: SymbolKind::Function,
            file: PathBuf::from("/proj/pkg/service.py"),
            line,
            severity,
            message: format!("`{symbol}` has no references"),
        }
    }

    fn sample() -> Report {
        Report::new(
            vec!["pkg/service.py".to_string()],
            7,
            vec![finding("orphan", Severity::Warn, 12), finding("helper", Severity::Note, 30)],
        )
    }

    #[test]
    fn human_output_shows_repo_relative_locations() {
        let text = sample()
            .render(Format::Human, Path::new("/proj"))
            .expect("human rendering cannot fail");
        assert!(
            text.contains("pkg/service.py:12"),
            "paths should be relative to the workspace root, got:\n{text}"
        );
        assert!(
            !text.contains("/proj/pkg"),
            "absolute paths should not leak into human output, got:\n{text}"
        );
        assert!(text.contains("warn"), "severity label should be visible");
        assert!(text.contains("func"), "symbol kind should be visible");
    }

    #[test]
    fn human_output_ends_with_a_counted_summary() {
        let text = sample()
            .render(Format::Human, Path::new("/proj"))
            .expect("human rendering cannot fail");
        assert!(
            text.contains("1 file(s), 7 symbol(s) checked — 1 warn, 1 note"),
            "summary line should count files, symbols and severities, got:\n{text}"
        );
    }

    #[test]
    fn a_clean_report_says_so_explicitly() {
        let report = Report::new(vec!["pkg/service.py".to_string()], 4, vec![]);
        assert!(report.is_clean(), "no findings means clean");

        let text =
            report.render(Format::Human, Path::new("/proj")).expect("human rendering cannot fail");
        assert!(
            text.contains("No findings."),
            "an empty run must not print an empty body, got:\n{text}"
        );
        assert!(text.contains("0 warn, 0 note"), "the summary should still appear, got:\n{text}");
    }

    #[test]
    fn json_output_keeps_absolute_paths_and_round_trips() {
        let text =
            sample().render(Format::Json, Path::new("/proj")).expect("JSON rendering cannot fail");
        let value: serde_json::Value =
            serde_json::from_str(&text).expect("output must be valid JSON");

        assert_eq!(value["symbols_checked"], 7, "symbol count is carried through");
        assert_eq!(
            value["findings"].as_array().map(Vec::len),
            Some(2),
            "both findings are present"
        );
        assert_eq!(value["findings"][0]["severity"], "warn", "severity serialises lowercase");
        assert_eq!(
            value["findings"][0]["file"], "/proj/pkg/service.py",
            "JSON keeps absolute paths so downstream tools do not have to guess the root"
        );
    }

    #[test]
    fn counts_are_per_severity() {
        let report = sample();
        assert_eq!(report.count(Severity::Warn), 1, "one warning in the sample");
        assert_eq!(report.count(Severity::Note), 1, "one note in the sample");
        assert!(!report.is_clean(), "a report with findings is not clean");
    }
}
