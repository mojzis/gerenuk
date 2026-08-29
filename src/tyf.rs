//! Adapter around the `tyf` binary (from `ty-find`).
//!
//! `ty-find` ships a binary, not a library, so gerenuk talks to it over stdout
//! with `--format json`. [`Runner`] is the one impure seam in the crate: every
//! other module takes already-parsed data, which keeps analysis testable
//! without `tyf` (or `ty`) on `PATH`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{bail, Context, Result};

use crate::model::{DocumentSymbol, Location, ReferencesResult};

/// Binary name looked up on `PATH` when `GERENUK_TYF` is unset.
pub const DEFAULT_TYF_BIN: &str = "tyf";

/// Environment variable that overrides which `tyf` binary is used.
pub const TYF_BIN_ENV: &str = "GERENUK_TYF";

/// Spawns `tyf` inside a workspace and decodes its JSON output.
#[derive(Debug, Clone)]
pub struct Runner {
    bin: PathBuf,
    workspace: PathBuf,
}

impl Runner {
    /// Build a runner for `workspace`, resolving the binary from
    /// `GERENUK_TYF` if set and otherwise from `PATH`.
    pub fn discover(workspace: impl Into<PathBuf>) -> Result<Self> {
        let bin = match std::env::var_os(TYF_BIN_ENV) {
            Some(explicit) => PathBuf::from(explicit),
            None => which::which(DEFAULT_TYF_BIN).with_context(|| {
                format!(
                    "`{DEFAULT_TYF_BIN}` not found on PATH. Install it with `uv add --dev ty-find`, \
                     or point {TYF_BIN_ENV} at the binary."
                )
            })?,
        };
        Ok(Self { bin, workspace: workspace.into() })
    }

    /// Build a runner for an explicit binary path, skipping `PATH` lookup.
    pub fn with_binary(bin: impl Into<PathBuf>, workspace: impl Into<PathBuf>) -> Self {
        Self { bin: bin.into(), workspace: workspace.into() }
    }

    #[must_use]
    pub fn binary(&self) -> &Path {
        &self.bin
    }

    #[must_use]
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// Run `tyf --format json <args>` and return stdout.
    ///
    /// `tyf` prefixes some commands with a human-readable banner even under
    /// `--format json`, so the caller gets the raw text and [`extract_json`]
    /// trims it before parsing.
    pub fn run<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut cmd = Command::new(&self.bin);
        cmd.current_dir(&self.workspace).arg("--format").arg("json").args(args);

        let output: Output =
            cmd.output().with_context(|| format!("failed to spawn `{}`", self.bin.display()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("`{}` exited with {}: {}", self.bin.display(), output.status, stderr.trim());
        }

        String::from_utf8(output.stdout).context("`tyf` produced output that was not valid UTF-8")
    }

    /// Definition sites of `symbol` (`tyf find`).
    pub fn find(&self, symbol: &str) -> Result<Vec<Location>> {
        let raw = self.run(["find", symbol])?;
        parse_find(&raw)
    }

    /// Usages of `symbol` (`tyf refs`).
    ///
    /// `--tests` populates `test_references` (omitted by default) and
    /// `--references-limit 0` disables truncation, so the returned lists match
    /// the counts alongside them.
    pub fn refs(&self, symbol: &str) -> Result<ReferencesResult> {
        let raw = self.run(["refs", symbol, "--tests", "--references-limit", "0"])?;
        parse_refs(&raw)
    }

    /// Outline of a Python file (`tyf list`).
    pub fn list(&self, file: &Path) -> Result<Vec<DocumentSymbol>> {
        let raw = self.run([OsStr::new("list"), file.as_os_str()])?;
        parse_outline(&raw)
    }
}

/// Strip any leading human banner and return the JSON body.
///
/// `tyf list` prints e.g. `Document outline for main.py:` before the payload,
/// so we start at the first `[` or `{`.
pub fn extract_json(raw: &str) -> Result<&str> {
    let start = raw
        .find(['[', '{'])
        .with_context(|| format!("no JSON found in `tyf` output: {}", raw.trim()))?;
    Ok(raw[start..].trim_end())
}

/// Parse a `tyf find` payload.
pub fn parse_find(raw: &str) -> Result<Vec<Location>> {
    serde_json::from_str(extract_json(raw)?).context("could not parse `tyf find` JSON")
}

/// Parse a `tyf refs` payload.
pub fn parse_refs(raw: &str) -> Result<ReferencesResult> {
    serde_json::from_str(extract_json(raw)?).context("could not parse `tyf refs` JSON")
}

/// Parse a `tyf list` payload.
pub fn parse_outline(raw: &str) -> Result<Vec<DocumentSymbol>> {
    serde_json::from_str(extract_json(raw)?).context("could not parse `tyf list` JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIND_JSON: &str = r#"[
      {
        "uri": "file:///proj/main.py",
        "range": {
          "start": {"line": 13, "character": 0},
          "end": {"line": 16, "character": 29}
        }
      }
    ]"#;

    const REFS_JSON: &str = r#"{
      "reference_count": 2,
      "references": [
        {"column": 5, "context": "list_animals", "file": "/proj/main.py", "line": 14},
        {"column": 5, "context": "demo_models", "file": "/proj/main.py", "line": 27}
      ],
      "symbol": "list_animals",
      "test_reference_count": 0,
      "test_references": []
    }"#;

    #[test]
    fn find_payload_yields_definition_sites() {
        let locations = parse_find(FIND_JSON).expect("valid find payload should parse");
        assert_eq!(locations.len(), 1, "fixture has exactly one definition");
        assert_eq!(locations[0].path(), Path::new("/proj/main.py"));
        assert_eq!(locations[0].display_line(), 14, "definition starts on line 14 (one-based)");
    }

    #[test]
    fn refs_payload_yields_counts_and_contexts() {
        let result = parse_refs(REFS_JSON).expect("valid refs payload should parse");
        assert_eq!(result.symbol, "list_animals");
        assert_eq!(result.reference_count, 2, "fixture reports two references");
        assert_eq!(result.references.len(), 2, "reference list length must match the count");
        assert_eq!(
            result.references[1].context, "demo_models",
            "context names the enclosing symbol"
        );
        assert!(!result.only_used_by_tests(), "fixture has production references");
    }

    #[test]
    fn a_human_banner_before_the_payload_is_skipped() {
        let raw = format!("Document outline for main.py:\n\n{FIND_JSON}");
        let locations = parse_find(&raw).expect("banner should be stripped before parsing");
        assert_eq!(locations.len(), 1, "banner must not affect the parsed payload");
    }

    #[test]
    fn output_without_json_is_a_clear_error() {
        let err = parse_find("no symbols found\n").expect_err("non-JSON output must be an error");
        assert!(
            err.to_string().contains("no JSON found"),
            "error should name the problem, got: {err}"
        );
    }

    #[test]
    fn malformed_json_is_reported_against_the_command() {
        let err = parse_refs("{ \"symbol\": ").expect_err("truncated JSON must be an error");
        assert!(
            err.to_string().contains("`tyf refs`"),
            "error should say which tyf command failed, got: {err}"
        );
    }

    #[test]
    fn missing_optional_fields_default_instead_of_failing() {
        let result = parse_refs(r#"{"symbol": "solo"}"#).expect("only `symbol` is required");
        assert_eq!(result.symbol, "solo");
        assert_eq!(result.reference_count, 0, "absent counts default to zero");
        assert!(result.references.is_empty(), "absent reference lists default to empty");
    }

    #[test]
    fn explicit_binary_skips_path_lookup() {
        let runner = Runner::with_binary("/opt/tyf", "/proj");
        assert_eq!(runner.binary(), Path::new("/opt/tyf"), "explicit binary is used as given");
        assert_eq!(runner.workspace(), Path::new("/proj"));
    }

    #[test]
    fn spawning_a_missing_binary_names_the_binary() {
        let runner = Runner::with_binary("/nonexistent/tyf-binary", ".");
        let err = runner.run(["find", "anything"]).expect_err("missing binary must error");
        assert!(
            err.to_string().contains("tyf-binary"),
            "error should name the binary it tried to spawn, got: {err}"
        );
    }
}
