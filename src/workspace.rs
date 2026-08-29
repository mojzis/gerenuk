//! Locating the Python project a command should run against.
//!
//! `tyf` does its own workspace detection, but gerenuk needs the root too — to
//! render paths relative to it and to decide which files count as tests.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Files that mark a directory as a Python project root, most specific first.
const ROOT_MARKERS: &[&str] = &["pyproject.toml", "setup.py", "setup.cfg", ".git"];

/// Walk up from `start` until a project marker is found.
///
/// Returns the deepest ancestor containing one of [`ROOT_MARKERS`]. Errors when
/// no ancestor qualifies, rather than silently falling back to `/`.
pub fn detect_root(start: &Path) -> Result<PathBuf> {
    let start =
        start.canonicalize().with_context(|| format!("cannot resolve {}", start.display()))?;

    let mut dir: Option<&Path> = if start.is_dir() { Some(&start) } else { start.parent() };

    while let Some(candidate) = dir {
        if ROOT_MARKERS.iter().any(|m| candidate.join(m).exists()) {
            return Ok(candidate.to_path_buf());
        }
        dir = candidate.parent();
    }

    anyhow::bail!(
        "no Python project root above {} (looked for {})",
        start.display(),
        ROOT_MARKERS.join(", ")
    )
}

/// Whether a path looks like test code, judged relative to `root`.
///
/// Deliberately conventional rather than clever: a `tests`/`test` directory
/// component, or a `test_*.py` / `*_test.py` filename.
///
/// The path is made relative to `root` first, and that matters: a project that
/// itself lives under a `tests/` directory (gerenuk's own fixture package does)
/// would otherwise have *every* file classified as a test.
#[must_use]
pub fn is_test_path(path: &Path, root: &Path) -> bool {
    let path = path.strip_prefix(root).unwrap_or(path);

    let in_test_dir =
        path.components().any(|c| matches!(c.as_os_str().to_str(), Some("tests" | "test")));
    if in_test_dir {
        return true;
    }

    if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("py")) {
        return false;
    }

    path.file_stem()
        .and_then(|n| n.to_str())
        .is_some_and(|stem| stem.starts_with("test_") || stem.ends_with("_test"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn root_is_the_nearest_ancestor_with_a_pyproject() {
        let tmp = TempDir::new().expect("temp dir");
        let root = tmp.path().join("proj");
        let nested = root.join("pkg").join("sub");
        fs::create_dir_all(&nested).expect("create nested dirs");
        fs::write(root.join("pyproject.toml"), "[project]\nname = \"p\"\n").expect("write marker");

        let found = detect_root(&nested).expect("marker exists above the start dir");
        assert_eq!(
            found,
            root.canonicalize().expect("canonicalize root"),
            "detection should stop at the directory holding pyproject.toml"
        );
    }

    #[test]
    fn starting_from_a_file_searches_its_directory_upward() {
        let tmp = TempDir::new().expect("temp dir");
        let root = tmp.path().join("proj");
        fs::create_dir_all(root.join("pkg")).expect("create pkg dir");
        fs::write(root.join("setup.py"), "").expect("write marker");
        let file = root.join("pkg").join("mod.py");
        fs::write(&file, "x = 1\n").expect("write module");

        let found = detect_root(&file).expect("marker exists above the file");
        assert_eq!(
            found,
            root.canonicalize().expect("canonicalize root"),
            "a file start uses its parent"
        );
    }

    #[test]
    fn a_directory_with_no_marker_anywhere_above_is_an_error() {
        let tmp = TempDir::new().expect("temp dir");
        let lonely = tmp.path().join("lonely");
        fs::create_dir_all(&lonely).expect("create dir");

        // TempDir lives under the system temp dir, which has no project markers.
        let err = detect_root(&lonely).expect_err("no marker should be an error, not a silent /");
        assert!(
            err.to_string().contains("no Python project root"),
            "error should explain what was missing, got: {err}"
        );
    }

    #[test]
    fn missing_paths_report_the_path() {
        let err = detect_root(Path::new("/definitely/not/here")).expect_err("missing path errors");
        assert!(
            err.to_string().contains("/definitely/not/here"),
            "error should name the unresolvable path, got: {err}"
        );
    }

    #[test]
    fn test_paths_are_recognised_by_directory_or_filename() {
        let root = Path::new("/proj");
        assert!(is_test_path(Path::new("tests/test_models.py"), root), "tests/ directory");
        assert!(is_test_path(Path::new("pkg/test/helpers.py"), root), "test/ directory");
        assert!(is_test_path(Path::new("pkg/test_service.py"), root), "test_ prefix");
        assert!(is_test_path(Path::new("pkg/service_test.py"), root), "_test suffix");
        assert!(is_test_path(Path::new("/proj/tests/test_x.py"), root), "absolute paths work too");
    }

    #[test]
    fn a_root_that_itself_sits_under_tests_does_not_taint_everything() {
        // gerenuk's own fixture package lives at tests/fixtures/sample_pkg.
        let root = Path::new("/repo/tests/fixtures/sample_pkg");
        assert!(
            !is_test_path(Path::new("/repo/tests/fixtures/sample_pkg/pkg/service.py"), root),
            "the `tests` component above the root must be ignored"
        );
        assert!(
            is_test_path(Path::new("/repo/tests/fixtures/sample_pkg/tests/test_service.py"), root),
            "a `tests` component below the root still counts"
        );
    }

    #[test]
    fn production_paths_are_not_mistaken_for_tests() {
        let root = Path::new("/proj");
        assert!(!is_test_path(Path::new("pkg/service.py"), root), "plain module");
        assert!(
            !is_test_path(Path::new("pkg/latest.py"), root),
            "'test' inside a word is not a test file"
        );
        assert!(!is_test_path(Path::new("pkg/testing.py"), root), "'testing.py' is not test_*.py");
        assert!(!is_test_path(Path::new("contest/main.py"), root), "'contest' dir is not 'test'");
    }
}
