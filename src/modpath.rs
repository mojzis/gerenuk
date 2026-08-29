//! Turning a repository-relative file path into a dotted Python module path.
//!
//! The rule is the one the interpreter itself uses: a module's package is the
//! unbroken chain of directories above it that contain `__init__.py`. That
//! handles src-layout (`src/mypkg/a.py` → `mypkg.a`) and flat layout
//! (`mypkg/a.py` → `mypkg.a`) without needing to know which one is in play,
//! because `src/` has no `__init__.py` and so ends the chain on its own.

use std::path::Path;

/// Directory component that is stripped when no `__init__.py` chain is found.
const SRC_DIR: &str = "src";

/// Dotted module path for `rel`, a path relative to `root`.
///
/// Returns `None` for anything that is not a `.py` file. Namespace packages
/// (PEP 420, no `__init__.py`) fall back to the literal path with a leading
/// `src/` removed, which is right often enough to be useful and wrong quietly
/// enough not to matter — phase 2 re-resolves symbols through `ty` anyway.
#[must_use]
pub fn module_path(root: &Path, rel: &Path) -> Option<String> {
    if rel.extension().is_none_or(|e| e != "py") {
        return None;
    }
    let stem = rel.file_stem()?.to_str()?;

    let mut package = package_chain(root, rel);
    if package.is_empty() {
        package = fallback_chain(rel);
    }

    if stem == "__init__" {
        // The file *is* the package; `mypkg/__init__.py` is module `mypkg`.
        if package.is_empty() {
            return None;
        }
    } else {
        package.push(stem.to_string());
    }

    Some(package.join("."))
}

/// Directories above `rel` that hold an `__init__.py`, outermost first.
fn package_chain(root: &Path, rel: &Path) -> Vec<String> {
    let mut chain = Vec::new();
    let mut dir = rel.parent();

    while let Some(current) = dir {
        let name = current.file_name().and_then(|n| n.to_str());
        let Some(name) = name else { break };
        if !root.join(current).join("__init__.py").exists() {
            break;
        }
        chain.push(name.to_string());
        dir = current.parent();
    }

    chain.reverse();
    chain
}

/// Every directory component of `rel`, minus a leading `src/`.
fn fallback_chain(rel: &Path) -> Vec<String> {
    let mut parts: Vec<String> = rel
        .parent()
        .into_iter()
        .flat_map(Path::components)
        .filter_map(|c| c.as_os_str().to_str().map(ToString::to_string))
        .collect();

    if parts.first().is_some_and(|first| first == SRC_DIR) {
        parts.remove(0);
    }
    parts
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::*;

    /// Build a tree under a temp root; every path ending in `/` is a package
    /// directory (gets an `__init__.py`), everything else is an empty file.
    fn tree(entries: &[&str]) -> TempDir {
        let tmp = TempDir::new().expect("temp dir");
        for entry in entries {
            let path = tmp.path().join(entry.trim_end_matches('/'));
            if entry.ends_with('/') {
                std::fs::create_dir_all(&path).expect("create package dir");
                std::fs::write(path.join("__init__.py"), "").expect("write __init__");
            } else {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).expect("create parent");
                }
                std::fs::write(&path, "").expect("write file");
            }
        }
        tmp
    }

    fn resolve(tmp: &TempDir, rel: &str) -> Option<String> {
        module_path(tmp.path(), &PathBuf::from(rel))
    }

    #[test]
    fn src_layout_drops_the_src_directory() {
        let tmp = tree(&["src/mypkg/", "src/mypkg/pipelines/", "src/mypkg/pipelines/enrich.py"]);
        assert_eq!(
            resolve(&tmp, "src/mypkg/pipelines/enrich.py").as_deref(),
            Some("mypkg.pipelines.enrich"),
            "the chain stops at src/, which has no __init__.py"
        );
    }

    #[test]
    fn flat_layout_keeps_the_top_level_package() {
        let tmp = tree(&["mypkg/", "mypkg/utils.py"]);
        assert_eq!(
            resolve(&tmp, "mypkg/utils.py").as_deref(),
            Some("mypkg.utils"),
            "a flat layout package resolves the same way"
        );
    }

    #[test]
    fn an_init_file_is_the_package_itself() {
        let tmp = tree(&["src/mypkg/"]);
        assert_eq!(
            resolve(&tmp, "src/mypkg/__init__.py").as_deref(),
            Some("mypkg"),
            "__init__.py must not appear as a module component"
        );
    }

    #[test]
    fn a_nested_init_file_keeps_the_full_package_path() {
        let tmp = tree(&["src/mypkg/", "src/mypkg/sub/"]);
        assert_eq!(
            resolve(&tmp, "src/mypkg/sub/__init__.py").as_deref(),
            Some("mypkg.sub"),
            "the chain above the __init__ still contributes"
        );
    }

    #[test]
    fn a_directory_without_an_init_falls_back_to_the_path() {
        let tmp = tree(&["scripts/loose.py"]);
        assert_eq!(
            resolve(&tmp, "scripts/loose.py").as_deref(),
            Some("scripts.loose"),
            "no __init__.py chain means the path is used verbatim"
        );
    }

    #[test]
    fn the_fallback_still_strips_a_leading_src() {
        let tmp = tree(&["src/nspkg/mod.py"]);
        assert_eq!(
            resolve(&tmp, "src/nspkg/mod.py").as_deref(),
            Some("nspkg.mod"),
            "a namespace package under src/ should not carry `src` in its name"
        );
    }

    #[test]
    fn a_top_level_module_is_just_its_stem() {
        let tmp = tree(&["conftest.py"]);
        assert_eq!(
            resolve(&tmp, "conftest.py").as_deref(),
            Some("conftest"),
            "a module at the repo root has no package"
        );
    }

    #[test]
    fn a_chain_break_stops_the_walk() {
        // `mypkg` is a package, `vendored` below it is not.
        let tmp = tree(&["mypkg/", "mypkg/vendored/thing.py"]);
        assert_eq!(
            resolve(&tmp, "mypkg/vendored/thing.py").as_deref(),
            Some("mypkg.vendored.thing"),
            "the fallback uses the whole path once the chain is broken at the leaf"
        );
    }

    #[test]
    fn non_python_files_have_no_module_path() {
        let tmp = tree(&["mypkg/", "mypkg/data.sql"]);
        assert_eq!(resolve(&tmp, "mypkg/data.sql"), None, "only .py files are modules");
        assert_eq!(resolve(&tmp, "pyproject.toml"), None, "extensionless-ish files too");
    }

    #[test]
    fn a_path_whose_package_no_longer_exists_still_resolves() {
        // Deleted files are looked up against a tree that no longer holds them.
        let tmp = TempDir::new().expect("temp dir");
        assert_eq!(
            module_path(tmp.path(), Path::new("src/mypkg/gone.py")).as_deref(),
            Some("mypkg.gone"),
            "deletion analysis must not depend on the directory still being there"
        );
    }
}
