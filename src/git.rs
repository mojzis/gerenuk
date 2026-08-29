//! Adapter around the `git` binary — gerenuk's second and last impure seam.
//!
//! Like [`crate::tyf::Runner`], everything this module returns is plain data:
//! diff text for [`crate::diff`] to parse, and file contents for
//! [`crate::pysource`]. Nothing downstream spawns a process, so the whole
//! change-classification path is unit-testable without a repository.
//!
//! Shelling out beats a git library here. `git diff -M` rename detection and
//! merge-base resolution are exactly the parts a reimplementation gets subtly
//! wrong, and phase 1 runs them a handful of times per invocation.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

/// Binary name looked up on `PATH` when `GERENUK_GIT` is unset.
pub const DEFAULT_GIT_BIN: &str = "git";

/// Environment variable that overrides which `git` binary is used.
pub const GIT_BIN_ENV: &str = "GERENUK_GIT";

/// Base refs tried in order when `--base` is not given.
pub const DEFAULT_BASES: &[&str] = &["origin/main", "main", "master"];

/// The base a run was diffed against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Base {
    /// The ref that was used, e.g. `origin/main`.
    pub name: String,
    /// `merge-base(HEAD, name)`, the commit the diff actually starts from.
    pub merge_base: String,
}

/// One completed `git` invocation.
struct GitOutput {
    success: bool,
    /// `ExitStatus`'s own rendering, e.g. `exit status: 128`.
    status: String,
    stdout: String,
    stderr: String,
}

/// Runs `git` inside one repository.
#[derive(Debug, Clone)]
pub struct Git {
    bin: PathBuf,
    repo: PathBuf,
}

impl Git {
    /// Resolve the `git` binary and bind it to `repo`.
    pub fn discover(repo: impl Into<PathBuf>) -> Result<Self> {
        let bin = match std::env::var_os(GIT_BIN_ENV) {
            Some(explicit) => PathBuf::from(explicit),
            None => which::which(DEFAULT_GIT_BIN)
                .with_context(|| format!("`{DEFAULT_GIT_BIN}` not found on PATH"))?,
        };
        Ok(Self { bin, repo: repo.into() })
    }

    /// The same binary, bound to a different repository.
    ///
    /// Cheaper than a second [`Self::discover`], which would re-scan `PATH`.
    #[must_use]
    pub fn rebind(&self, repo: impl Into<PathBuf>) -> Self {
        Self { bin: self.bin.clone(), repo: repo.into() }
    }

    /// Run `git <args>` in the repository and return stdout.
    ///
    /// This and [`Self::try_run`] are the only ways into [`Self::output`],
    /// which is the one place outside [`crate::tyf`] that spawns a process.
    /// A non-zero exit is an error carrying git's stderr.
    pub fn run<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let out = self.output(args)?;
        if !out.success {
            bail!("`git` {} — {}", out.status, out.stderr.trim());
        }
        Ok(out.stdout)
    }

    /// Like [`Self::run`], but a non-zero exit yields `None` instead of an error.
    ///
    /// Used for the "does this ref exist?" and "did this path exist then?"
    /// questions, where failure is an answer rather than a fault.
    pub fn try_run<I, S>(&self, args: I) -> Result<Option<String>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let out = self.output(args)?;
        Ok(out.success.then_some(out.stdout))
    }

    /// Spawn `git` and collect the whole result.
    fn output<I, S>(&self, args: I) -> Result<GitOutput>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut cmd = Command::new(&self.bin);
        // `core.quotePath=false` keeps non-ASCII paths literal instead of
        // C-escaped, so the diff parser never has to unescape.
        cmd.current_dir(&self.repo).arg("-c").arg("core.quotePath=false").args(args);

        let output =
            cmd.output().with_context(|| format!("failed to spawn `{}`", self.bin.display()))?;

        Ok(GitOutput {
            success: output.status.success(),
            status: output.status.to_string(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    /// Absolute path of the repository's top level.
    pub fn top_level(&self) -> Result<PathBuf> {
        let out = self
            .run(["rev-parse", "--show-toplevel"])
            .with_context(|| format!("{} is not inside a git repository", self.repo.display()))?;
        Ok(PathBuf::from(out.trim()))
    }

    /// Pick a base ref and compute `merge-base(HEAD, base)`.
    ///
    /// An explicit `--base` is used as given and its absence is an error; with
    /// no `--base`, [`DEFAULT_BASES`] are tried in order.
    pub fn resolve_base(&self, explicit: Option<&str>) -> Result<Base> {
        let candidates: Vec<&str> = explicit.map_or_else(|| DEFAULT_BASES.to_vec(), |e| vec![e]);

        for candidate in &candidates {
            if !self.rev_exists(candidate)? {
                continue;
            }
            let merge_base = self
                .run(["merge-base", "HEAD", candidate])
                .with_context(|| format!("no common ancestor between HEAD and `{candidate}`"))?;
            return Ok(Base {
                name: (*candidate).to_string(),
                merge_base: merge_base.trim().into(),
            });
        }

        bail!(
            "no usable base ref (tried {}). Pass --base with a ref that exists.",
            candidates.join(", ")
        )
    }

    /// Whether `rev` resolves to a commit.
    fn rev_exists(&self, rev: &str) -> Result<bool> {
        let spec = format!("{rev}^{{commit}}");
        Ok(self.try_run(["rev-parse", "--verify", "--quiet", &spec])?.is_some())
    }

    /// The whole `merge_base`-to-working-tree diff, with no context lines.
    ///
    /// No `--cached`, no second revision: this is staged *and* unstaged work,
    /// which is what phase 1 asked for. The prefixes are pinned so user
    /// configuration cannot change what [`crate::diff`] has to parse.
    pub fn diff(&self, merge_base: &str) -> Result<String> {
        self.run([
            "diff",
            "-U0",
            "--no-color",
            // `diff.external` (difftastic and friends) replaces git's diff
            // generator outright, and a `.gitattributes` textconv filter
            // renumbers every line. Both would be silent: an empty or
            // misattributed report with exit code 0.
            "--no-ext-diff",
            "--no-textconv",
            "-M",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            merge_base,
            "--",
        ])
        .context("could not diff the working tree against the merge base")
    }

    /// Files git is not tracking and is not ignoring.
    ///
    /// `git diff` cannot see these, but a brand-new module that has not been
    /// staged yet is the most common thing a pre-commit hook meets.
    pub fn untracked(&self) -> Result<Vec<PathBuf>> {
        self.paths(
            ["ls-files", "--others", "--exclude-standard", "-z"],
            "could not list untracked files",
        )
    }

    /// Every path git tracks, repository-relative.
    ///
    /// Used to enumerate the workspace's Python files for the textual scans:
    /// `.gitignore` already encodes which directories are not worth reading
    /// (`.venv`, `node_modules`, vendored trees), correctly and per project.
    pub fn ls_files(&self) -> Result<Vec<PathBuf>> {
        self.paths(["ls-files", "-z"], "could not list tracked files")
    }

    /// Run a `-z` listing command and split its NUL-separated paths.
    fn paths<const N: usize>(
        &self,
        args: [&str; N],
        context: &'static str,
    ) -> Result<Vec<PathBuf>> {
        let out = self.run(args).context(context)?;
        Ok(out.split('\0').filter(|s| !s.is_empty()).map(PathBuf::from).collect())
    }

    /// Contents of `path` at `rev`, or `None` if it did not exist there.
    pub fn show(&self, rev: &str, path: &Path) -> Result<Option<String>> {
        let spec = format!("{rev}:{}", path.display());
        self.try_run(["show", &spec])
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        reason = "these tests drive a real git binary; a failed setup step should abort loudly"
    )]

    use tempfile::TempDir;

    use super::*;

    /// A throwaway repository, isolated from the developer's git configuration.
    struct TestRepo {
        dir: TempDir,
    }

    impl TestRepo {
        fn new() -> Self {
            let repo = Self { dir: TempDir::new().expect("temp dir") };
            repo.git(&["init", "--initial-branch=main"]);
            repo.git(&["config", "user.email", "test@example.com"]);
            repo.git(&["config", "user.name", "Test"]);
            repo
        }

        fn path(&self) -> &Path {
            self.dir.path()
        }

        fn git(&self, args: &[&str]) -> String {
            let output = Command::new("git")
                .current_dir(self.path())
                // Ignore the developer's global config: signing keys, hooks and
                // a different default branch would all break these tests.
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .args(args)
                .output()
                .expect("git should be on PATH for the test suite");
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8_lossy(&output.stdout).into_owned()
        }

        fn write(&self, rel: &str, body: &str) {
            let path = self.path().join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create parent dir");
            }
            std::fs::write(path, body).expect("write file");
        }

        fn commit(&self, message: &str) {
            self.git(&["add", "-A"]);
            self.git(&["commit", "-m", message]);
        }

        fn gerenuk_git(&self) -> Git {
            Git::discover(self.path()).expect("git should be discoverable")
        }
    }

    #[test]
    fn the_default_bases_are_tried_in_order() {
        let repo = TestRepo::new();
        repo.write("a.py", "x = 1\n");
        repo.commit("base");

        let base = repo.gerenuk_git().resolve_base(None).expect("`main` exists");
        assert_eq!(base.name, "main", "origin/main is absent, so `main` wins");
        assert_eq!(
            base.merge_base.len(),
            40,
            "a full sha should come back, got {}",
            base.merge_base
        );
    }

    #[test]
    fn master_is_the_last_resort() {
        let repo = TestRepo::new();
        repo.write("a.py", "x = 1\n");
        repo.commit("base");
        repo.git(&["branch", "-m", "main", "master"]);

        let base = repo.gerenuk_git().resolve_base(None).expect("`master` exists");
        assert_eq!(base.name, "master", "the fallback chain reaches master");
    }

    #[test]
    fn an_explicit_base_is_used_verbatim() {
        let repo = TestRepo::new();
        repo.write("a.py", "x = 1\n");
        repo.commit("base");
        repo.git(&["branch", "release"]);

        let base = repo.gerenuk_git().resolve_base(Some("release")).expect("`release` exists");
        assert_eq!(base.name, "release", "--base overrides the default chain");
    }

    #[test]
    fn a_missing_explicit_base_names_what_was_tried() {
        let repo = TestRepo::new();
        repo.write("a.py", "x = 1\n");
        repo.commit("base");

        let err = repo
            .gerenuk_git()
            .resolve_base(Some("origin/nope"))
            .expect_err("a missing base must not fall back silently");
        assert!(
            err.to_string().contains("origin/nope"),
            "the error should name the ref that was asked for, got: {err}"
        );
    }

    #[test]
    fn the_merge_base_is_the_fork_point_not_the_branch_tip() {
        let repo = TestRepo::new();
        repo.write("a.py", "x = 1\n");
        repo.commit("base");
        let fork = repo.git(&["rev-parse", "HEAD"]).trim().to_string();

        repo.git(&["checkout", "-b", "feature"]);
        repo.write("a.py", "x = 2\n");
        repo.commit("feature work");

        repo.git(&["checkout", "main"]);
        repo.write("b.py", "y = 1\n");
        repo.commit("main moves on");
        repo.git(&["checkout", "feature"]);

        let base = repo.gerenuk_git().resolve_base(Some("main")).expect("main exists");
        assert_eq!(
            base.merge_base, fork,
            "diverged branches must diff from the fork point, or main's own work shows up"
        );
    }

    #[test]
    fn the_diff_covers_staged_and_unstaged_work_together() {
        let repo = TestRepo::new();
        repo.write("a.py", "x = 1\n");
        repo.write("b.py", "y = 1\n");
        repo.commit("base");

        repo.write("a.py", "x = 2\n");
        repo.git(&["add", "a.py"]);
        repo.write("b.py", "y = 2\n");

        let git = repo.gerenuk_git();
        let base = git.resolve_base(None).expect("main exists");
        let text = git.diff(&base.merge_base).expect("diff runs");

        assert!(text.contains("a/a.py"), "the staged change must appear:\n{text}");
        assert!(text.contains("a/b.py"), "the unstaged change must appear too:\n{text}");
        assert!(text.contains("@@ -1 +1 @@"), "-U0 should give exact single-line hunks:\n{text}");
    }

    #[test]
    fn untracked_files_are_listed_but_ignored_ones_are_not() {
        let repo = TestRepo::new();
        repo.write(".gitignore", "ignored.py\n");
        repo.commit("base");
        repo.write("fresh.py", "def f(): pass\n");
        repo.write("ignored.py", "def g(): pass\n");

        let untracked = repo.gerenuk_git().untracked().expect("ls-files runs");
        assert_eq!(
            untracked,
            vec![PathBuf::from("fresh.py")],
            "--exclude-standard must honour .gitignore"
        );
    }

    #[test]
    fn ls_files_lists_tracked_paths_and_skips_ignored_ones() {
        let repo = TestRepo::new();
        repo.write(".gitignore", "build/\n");
        repo.write("pkg/mod.py", "x = 1\n");
        repo.write("build/generated.py", "y = 1\n");
        repo.commit("base");

        let tracked = repo.gerenuk_git().ls_files().expect("ls-files runs");
        assert!(tracked.contains(&PathBuf::from("pkg/mod.py")), "tracked source is listed");
        assert!(
            !tracked.contains(&PathBuf::from("build/generated.py")),
            "an ignored file was never added, so it is not tracked either: {tracked:?}"
        );
    }

    #[test]
    fn show_returns_the_old_contents_of_a_deleted_file() {
        let repo = TestRepo::new();
        repo.write("gone.py", "def f():\n    return 1\n");
        repo.commit("base");
        std::fs::remove_file(repo.path().join("gone.py")).expect("delete the file");

        let git = repo.gerenuk_git();
        let base = git.resolve_base(None).expect("main exists");
        let body = git.show(&base.merge_base, Path::new("gone.py")).expect("show runs");
        assert_eq!(
            body.as_deref(),
            Some("def f():\n    return 1\n"),
            "deletion analysis reads the old blob, so this is the load-bearing call"
        );
    }

    #[test]
    fn show_returns_none_for_a_path_that_did_not_exist() {
        let repo = TestRepo::new();
        repo.write("a.py", "x = 1\n");
        repo.commit("base");

        let git = repo.gerenuk_git();
        let base = git.resolve_base(None).expect("main exists");
        assert_eq!(
            git.show(&base.merge_base, Path::new("added_later.py")).expect("show runs"),
            None,
            "an added file has no old blob, and that is not an error"
        );
    }

    #[test]
    fn top_level_finds_the_repository_root_from_a_subdirectory() {
        let repo = TestRepo::new();
        repo.write("pkg/mod.py", "x = 1\n");
        repo.commit("base");

        let git = Git::discover(repo.path().join("pkg")).expect("git discoverable");
        let top = git.top_level().expect("we are inside a repository");
        assert_eq!(
            std::fs::canonicalize(top).expect("canonicalize"),
            std::fs::canonicalize(repo.path()).expect("canonicalize"),
            "the root is the repo, not the subdirectory we started in"
        );
    }

    #[test]
    fn a_directory_outside_any_repository_is_an_error() {
        let tmp = TempDir::new().expect("temp dir");
        let git = Git::discover(tmp.path()).expect("git discoverable");

        match git.top_level() {
            // Some images have a git-tracked /tmp. Say so rather than passing
            // silently, or this test quietly stops covering anything.
            Ok(root) => {
                eprintln!("SKIPPED: the temp dir is inside a repository at {}", root.display());
            }
            Err(err) => assert!(
                err.to_string().contains("not inside a git repository"),
                "the error should say what is wrong, got: {err}"
            ),
        }
    }
}
