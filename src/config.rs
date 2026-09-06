//! Project configuration, read from `[tool.gerenuk]` in the repo's
//! `pyproject.toml`.
//!
//! Unknown keys are accepted rather than rejected, so a newer gerenuk's config
//! file does not break an older binary.
//!
//! Budget keys are `Option`, not defaulted: "absent" has to stay
//! distinguishable from "set to the default value", because a CLI flag must be
//! able to override the file and the file must be able to override the
//! built-in.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::pysource::suffix_matches;

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

    /// How many BFS levels `impacted-tests` will walk before giving up.
    pub max_depth: Option<u32>,
    /// How many symbols it will visit before giving up.
    pub max_symbols: Option<usize>,
    /// Wall-clock budget for the whole walk, in milliseconds.
    pub budget_ms: Option<u64>,

    /// How `gerenuk run` invokes pytest.
    ///
    /// An argv rather than a string, because the common real-world value is a
    /// multi-word runner: `["uv", "run", "pytest"]`. Empty means "work it out",
    /// which is `GERENUK_PYTEST` and then `pytest` on `PATH`.
    pub pytest_command: Vec<String>,

    /// What `gerenuk run` execs instead of the full suite when the outcome is
    /// `run_all`.
    ///
    /// An argv, never a shell string. `None` is "not configured" and means the
    /// default — pytest with no selection. `Some(vec![])` is kept distinct
    /// from that on purpose: an empty argv is a configuration error, and
    /// [`crate::fallback::resolve`] refuses it, rather than silently meaning
    /// the default.
    pub fallback_command: Option<Vec<String>>,
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
            .find(|entry| suffix_matches(decorator, entry))
            .map(String::as_str)
    }
}

/// The field names `Config` accepts, asked of the deserializer rather than
/// written down twice. The guide tests check every key a page shows against
/// this list, and every key in this list against the pages. Test-only: a
/// shipped binary has no question to ask it.
#[cfg(test)]
pub(crate) mod keys {
    use std::collections::BTreeSet;
    use std::fmt;

    use serde::de::{self, Deserializer, Visitor};
    use serde::forward_to_deserialize_any;

    use super::Config;

    /// Carries the captured field list out through serde's error channel,
    /// the only way out of a `Deserializer` that refuses to produce a value.
    #[derive(Debug)]
    struct Captured(Vec<&'static str>);

    impl fmt::Display for Captured {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "captured fields: {:?}", self.0)
        }
    }

    impl std::error::Error for Captured {}

    impl de::Error for Captured {
        fn custom<T: fmt::Display>(_msg: T) -> Self {
            Self(Vec::new())
        }
    }

    /// A `Deserializer` that answers "what fields does this struct have?" and
    /// nothing else.
    struct FieldCapture;

    impl<'de> Deserializer<'de> for FieldCapture {
        type Error = Captured;

        fn deserialize_struct<V: Visitor<'de>>(
            self,
            _name: &'static str,
            fields: &'static [&'static str],
            _visitor: V,
        ) -> Result<V::Value, Captured> {
            Err(Captured(fields.to_vec()))
        }

        fn deserialize_any<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value, Captured> {
            Err(Captured(Vec::new()))
        }

        forward_to_deserialize_any! {
            bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
            bytes byte_buf option unit unit_struct newtype_struct seq tuple
            tuple_struct map enum identifier ignored_any
        }
    }

    /// Every key `[tool.gerenuk]` may set, in the kebab-case a file writes.
    #[must_use]
    pub fn accepted_keys() -> BTreeSet<&'static str> {
        match <Config as serde::Deserialize>::deserialize(FieldCapture) {
            Err(Captured(fields)) => fields.into_iter().collect(),
            // Unreachable for a derived struct, which always reaches
            // `deserialize_struct`. An empty set fails the test below loudly
            // rather than letting the guide checks pass vacuously.
            Ok(_) => BTreeSet::new(),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::accepted_keys;

        #[test]
        fn the_keys_are_the_kebab_case_field_names() {
            let keys = accepted_keys();
            for expected in [
                "ignore-decorators",
                "max-depth",
                "max-symbols",
                "budget-ms",
                "pytest-command",
                "fallback-command",
            ] {
                assert!(keys.contains(expected), "`{expected}` should be accepted: {keys:?}");
            }
            assert!(!keys.contains("max_depth"), "serde renames, so the snake form is not a key");
            assert!(!keys.contains("timeout-s"), "an invented key must be absent");
        }
    }
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
    fn budget_keys_are_absent_by_default() {
        let config = Config::default();
        assert_eq!(config.max_depth, None, "unset must be distinguishable from the default");
        assert_eq!(config.max_symbols, None);
        assert_eq!(config.budget_ms, None);
    }

    #[test]
    fn budget_keys_are_read_in_kebab_case() {
        let tmp =
            with_pyproject("[tool.gerenuk]\nmax-depth = 4\nmax-symbols = 120\nbudget-ms = 5000\n");
        let config = Config::load(tmp.path()).expect("valid config parses");
        assert_eq!(config.max_depth, Some(4));
        assert_eq!(config.max_symbols, Some(120));
        assert_eq!(config.budget_ms, Some(5000));
    }

    #[test]
    fn setting_only_one_budget_leaves_the_others_unset() {
        let tmp = with_pyproject("[tool.gerenuk]\nmax-depth = 2\n");
        let config = Config::load(tmp.path()).expect("valid config parses");
        assert_eq!(config.max_depth, Some(2), "the one that was set");
        assert_eq!(config.max_symbols, None, "and the others stay at the built-in default");
    }

    #[test]
    fn the_fallback_command_is_absent_by_default() {
        let tmp = with_pyproject("[tool.gerenuk]\npytest-command = [\"pytest\"]\n");
        let config = Config::load(tmp.path()).expect("valid config parses");
        assert_eq!(config.fallback_command, None, "unset means the default, not an empty argv");
    }

    #[test]
    fn the_fallback_command_is_read_as_an_argv() {
        let tmp = with_pyproject(
            "[tool.gerenuk]\nfallback-command = [\"scripts/pick.sh\", \"--from-gerenuk\"]\n",
        );
        let config = Config::load(tmp.path()).expect("valid config parses");
        assert_eq!(
            config.fallback_command,
            Some(vec!["scripts/pick.sh".to_string(), "--from-gerenuk".to_string()]),
            "every element survives, in order"
        );
    }

    #[test]
    fn an_empty_fallback_command_is_kept_distinct_from_an_absent_one() {
        // Loading does not reject it: the config is read by every command, and
        // only `run` cares. Resolution is where it becomes an error.
        let tmp = with_pyproject("[tool.gerenuk]\nfallback-command = []\n");
        let config = Config::load(tmp.path()).expect("loading is not where it fails");
        assert_eq!(config.fallback_command, Some(Vec::new()), "present, and empty");
    }

    #[test]
    fn a_fallback_command_that_is_a_string_names_the_key() {
        let tmp = with_pyproject("[tool.gerenuk]\nfallback-command = \"scripts/pick.sh\"\n");
        let err = Config::load(tmp.path()).expect_err("a shell string is not an argv");
        let message = format!("{err:#}");
        assert!(message.contains("fallback-command"), "names the key, got: {message}");
    }

    #[test]
    fn a_bare_entry_matches_bare_and_dotted_decorators() {
        let config =
            Config { ignore_decorators: vec!["transformation".to_string()], ..Config::default() };
        assert_eq!(config.matching_decorator("transformation"), Some("transformation"), "bare");
        assert_eq!(
            config.matching_decorator("registry.transformation"),
            Some("transformation"),
            "a bare entry matches any dotted path ending in it"
        );
    }

    #[test]
    fn a_dotted_entry_does_not_match_a_bare_decorator() {
        let config = Config {
            ignore_decorators: vec!["registry.transformation".to_string()],
            ..Config::default()
        };
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
        let config =
            Config { ignore_decorators: vec!["formation".to_string()], ..Config::default() };
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
