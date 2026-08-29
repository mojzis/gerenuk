//! Project configuration, read from `[tool.gerenuk]` in the repo's
//! `pyproject.toml`.
//!
//! Phase 1 has exactly one key. Unknown keys are accepted rather than rejected,
//! so a newer gerenuk's config file does not break an older binary.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// Everything gerenuk reads out of `pyproject.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Config {
    /// Dotted decorator names whose symbols are reported as ignored.
    ///
    /// Registry-style decorators (`@transformation`) mark functions that are
    /// invoked by a runner rather than called directly, so a change to one has
    /// no callers worth chasing.
    pub ignore_decorators: Vec<String>,
}

/// Wrapper types mirroring `pyproject.toml`'s nesting: `[tool.gerenuk]`.
#[derive(Debug, Default, Deserialize)]
struct PyProject {
    #[serde(default)]
    tool: Tool,
}

#[derive(Debug, Default, Deserialize)]
struct Tool {
    #[serde(default)]
    gerenuk: Config,
}

impl Config {
    /// Load `<root>/pyproject.toml`.
    ///
    /// A missing file or a missing `[tool.gerenuk]` table is not an error — both
    /// yield defaults. Malformed TOML is, and the error names the file.
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join("pyproject.toml");
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => return Err(err).with_context(|| format!("cannot read {}", path.display())),
        };

        let parsed: PyProject =
            toml::from_str(&text).with_context(|| format!("cannot parse {}", path.display()))?;
        Ok(parsed.tool.gerenuk)
    }

    /// The decorator entry that causes `decorator` to be ignored, if any.
    ///
    /// Matching is syntactic dotted-suffix matching on the decorator
    /// expression: entry `transformation` matches `@transformation` and
    /// `@registry.transformation`, while `registry.transformation` matches only
    /// the latter. Import aliases are deliberately not resolved.
    #[must_use]
    pub fn matching_decorator(&self, decorator: &str) -> Option<&str> {
        self.ignore_decorators
            .iter()
            .find(|entry| decorator_matches(decorator, entry))
            .map(String::as_str)
    }
}

/// True when the dotted decorator name ends with `entry` on a component boundary.
fn decorator_matches(decorator: &str, entry: &str) -> bool {
    if decorator == entry {
        return true;
    }
    decorator.strip_suffix(entry).is_some_and(|prefix| prefix.ends_with('.'))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn with_pyproject(body: &str) -> TempDir {
        let tmp = TempDir::new().expect("temp dir");
        std::fs::write(tmp.path().join("pyproject.toml"), body).expect("write pyproject");
        tmp
    }

    #[test]
    fn a_missing_pyproject_yields_defaults() {
        let tmp = TempDir::new().expect("temp dir");
        let config = Config::load(tmp.path()).expect("a missing file is not an error");
        assert!(config.ignore_decorators.is_empty(), "no config means no ignored decorators");
    }

    #[test]
    fn a_pyproject_without_the_table_yields_defaults() {
        let tmp = with_pyproject("[project]\nname = \"p\"\n");
        let config = Config::load(tmp.path()).expect("a missing table is not an error");
        assert!(config.ignore_decorators.is_empty(), "no [tool.gerenuk] means no ignored");
    }

    #[test]
    fn the_ignore_list_is_read_from_the_tool_table() {
        let tmp = with_pyproject(
            "[project]\nname = \"p\"\n\n\
             [tool.gerenuk]\nignore-decorators = [\"transformation\", \"registry.task\"]\n",
        );
        let config = Config::load(tmp.path()).expect("valid config parses");
        assert_eq!(
            config.ignore_decorators,
            vec!["transformation".to_string(), "registry.task".to_string()],
            "both entries should survive the kebab-case rename"
        );
    }

    #[test]
    fn unknown_keys_are_tolerated() {
        let tmp = with_pyproject("[tool.gerenuk]\nfuture-key = 3\nignore-decorators = [\"t\"]\n");
        let config = Config::load(tmp.path()).expect("unknown keys must not break older binaries");
        assert_eq!(config.ignore_decorators, vec!["t".to_string()], "the known key still loads");
    }

    #[test]
    fn malformed_toml_names_the_file() {
        let tmp = with_pyproject("[tool.gerenuk\n");
        let err = Config::load(tmp.path()).expect_err("broken TOML is an error");
        assert!(
            format!("{err:#}").contains("pyproject.toml"),
            "the error should name the offending file, got: {err:#}"
        );
    }

    #[test]
    fn a_wrongly_typed_key_names_the_file_and_the_key() {
        // Writing a bare string instead of a list is the likeliest mistake, so
        // the error has to be readable.
        let tmp = with_pyproject("[tool.gerenuk]\nignore-decorators = \"transformation\"\n");
        let err = Config::load(tmp.path()).expect_err("a string is not a list of strings");
        let message = format!("{err:#}");
        assert!(message.contains("pyproject.toml"), "names the offending file, got: {message}");
        assert!(message.contains("ignore-decorators"), "names the key, got: {message}");
    }

    #[test]
    fn a_bare_entry_matches_bare_and_dotted_decorators() {
        let config = Config { ignore_decorators: vec!["transformation".to_string()] };
        assert_eq!(config.matching_decorator("transformation"), Some("transformation"), "bare");
        assert_eq!(
            config.matching_decorator("registry.transformation"),
            Some("transformation"),
            "a bare entry matches any dotted path ending in it"
        );
    }

    #[test]
    fn a_dotted_entry_does_not_match_a_bare_decorator() {
        let config = Config { ignore_decorators: vec!["registry.transformation".to_string()] };
        assert_eq!(
            config.matching_decorator("transformation"),
            None,
            "a dotted entry is more specific than a bare decorator"
        );
        assert_eq!(
            config.matching_decorator("registry.transformation"),
            Some("registry.transformation"),
            "exact match"
        );
        assert_eq!(
            config.matching_decorator("a.registry.transformation"),
            Some("registry.transformation"),
            "suffix matching still applies to dotted entries"
        );
    }

    #[test]
    fn suffix_matching_respects_component_boundaries() {
        let config = Config { ignore_decorators: vec!["formation".to_string()] };
        assert_eq!(
            config.matching_decorator("transformation"),
            None,
            "`formation` must not match inside the word `transformation`"
        );
    }

    #[test]
    fn nothing_matches_an_empty_ignore_list() {
        let config = Config::default();
        assert_eq!(config.matching_decorator("transformation"), None, "empty config ignores none");
    }
}
