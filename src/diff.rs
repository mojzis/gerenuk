//! A parser for `git diff -U0` output.
//!
//! Pure text in, structured line ranges out — no subprocess, so every shape git
//! can emit (renames, deletions, binary files, zero-count hunks) is covered by
//! unit tests instead of by fixture repositories.
//!
//! With `-U0` there is no context, so a hunk header's ranges are exactly the
//! lines that changed. That is the whole reason this phase asks for `-U0`.

use std::path::PathBuf;

/// Prefixes gerenuk pins on the `git diff` invocation, so user configuration
/// (`diff.noprefix`, `diff.srcPrefix`) cannot change what this parser sees.
pub const SRC_PREFIX: &str = "a/";
/// See [`SRC_PREFIX`].
pub const DST_PREFIX: &str = "b/";

/// A contiguous run of lines, 1-based and inclusive of `start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineRange {
    pub start: u32,
    pub count: u32,
}

impl LineRange {
    /// The lines the range covers.
    pub fn lines(self) -> impl Iterator<Item = u32> {
        self.start..self.start.saturating_add(self.count)
    }
}

/// One file's worth of diff: where it was, where it is, and what moved.
///
/// `old_path` is `None` for an added file and `new_path` is `None` for a
/// deleted one. A rename has both, and they differ.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileDiff {
    pub old_path: Option<PathBuf>,
    pub new_path: Option<PathBuf>,
    /// Lines removed or replaced, numbered in the *old* file.
    pub old_ranges: Vec<LineRange>,
    /// Lines added or replaced, numbered in the *new* file.
    pub new_ranges: Vec<LineRange>,
    /// Git reported the contents as binary, so there are no line ranges.
    pub binary: bool,
}

/// Split a whole `git diff -U0` stream into per-file diffs.
#[must_use]
pub fn parse(diff: &str) -> Vec<FileDiff> {
    let mut files = Vec::new();
    let mut current: Option<FileDiff> = None;

    for line in diff.lines() {
        if let Some(header) = line.strip_prefix("diff --git ") {
            files.extend(current.take());
            current = Some(from_git_header(header));
            continue;
        }

        let Some(file) = current.as_mut() else { continue };

        if let Some(path) = line.strip_prefix("--- ") {
            file.old_path = side_path(path, SRC_PREFIX);
        } else if let Some(path) = line.strip_prefix("+++ ") {
            file.new_path = side_path(path, DST_PREFIX);
        } else if let Some(path) = line.strip_prefix("rename from ") {
            // Unprefixed and one per line, so unlike the `diff --git` header
            // these cannot be confused by a path that contains ` b/`.
            file.old_path = Some(PathBuf::from(path));
        } else if let Some(path) = line.strip_prefix("rename to ") {
            file.new_path = Some(PathBuf::from(path));
        } else if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
            file.binary = true;
        } else if line.starts_with("@@ ") {
            if let Some((old, new)) = parse_hunk_header(line) {
                // A zero-count side marks an insertion or deletion *point*, not
                // a touched line; recording it would attribute a symbol falsely.
                if old.count > 0 {
                    file.old_ranges.push(old);
                }
                if new.count > 0 {
                    file.new_ranges.push(new);
                }
            }
        }
    }

    files.extend(current);
    files
}

/// Seed a [`FileDiff`] from `diff --git a/<old> b/<new>`.
///
/// A best guess only: the header is ambiguous when a path contains ` b/`. The
/// `---`/`+++` and `rename from`/`rename to` lines override it whenever they
/// are present, which leaves the header authoritative only for binary and
/// mode-only changes — where the paths are equal, so the ambiguity cannot bite.
fn from_git_header(header: &str) -> FileDiff {
    let mut file = FileDiff::default();

    // Paths may contain spaces, so split on the ` b/` that starts the second
    // half rather than on whitespace.
    let separator = format!(" {DST_PREFIX}");
    if let Some((old, new)) = header.split_once(separator.as_str()) {
        file.old_path = old.strip_prefix(SRC_PREFIX).map(PathBuf::from);
        file.new_path = Some(PathBuf::from(new));
    }
    file
}

/// Path from a `---`/`+++` line, or `None` for `/dev/null`.
fn side_path(raw: &str, prefix: &str) -> Option<PathBuf> {
    // Git appends a tab and a timestamp under some diff drivers.
    let raw = raw.split('\t').next().unwrap_or(raw);
    if raw == "/dev/null" {
        return None;
    }
    Some(PathBuf::from(raw.strip_prefix(prefix).unwrap_or(raw)))
}

/// `@@ -12,3 +12,4 @@ optional context` → the two ranges.
fn parse_hunk_header(line: &str) -> Option<(LineRange, LineRange)> {
    let body = line.strip_prefix("@@ ")?;
    let body = body.split(" @@").next()?;
    let (old, new) = body.split_once(' ')?;
    Some((parse_range(old.strip_prefix('-')?)?, parse_range(new.strip_prefix('+')?)?))
}

/// `12,4` or `12` (count defaults to 1).
fn parse_range(spec: &str) -> Option<LineRange> {
    let (start, count) = match spec.split_once(',') {
        Some((start, count)) => (start, count.parse().ok()?),
        None => (spec, 1),
    };
    Some(LineRange { start: start.parse().ok()?, count })
}

/// Every line touched on the given side, deduplicated and sorted.
#[must_use]
pub fn touched_lines(ranges: &[LineRange]) -> Vec<u32> {
    let mut lines: Vec<u32> = ranges.iter().copied().flat_map(LineRange::lines).collect();
    lines.sort_unstable();
    lines.dedup();
    lines
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn a_modification_yields_both_sides() {
        let diff = "\
diff --git a/pkg/mod.py b/pkg/mod.py
index 1111111..2222222 100644
--- a/pkg/mod.py
+++ b/pkg/mod.py
@@ -12,3 +12,4 @@ def run():
-old
-old
-old
+new
+new
+new
+new
";
        let files = parse(diff);
        assert_eq!(files.len(), 1, "one file in the stream");
        assert_eq!(files[0].old_path.as_deref(), Some(Path::new("pkg/mod.py")), "a/ is stripped");
        assert_eq!(files[0].new_path.as_deref(), Some(Path::new("pkg/mod.py")), "b/ is stripped");
        assert_eq!(files[0].old_ranges, vec![LineRange { start: 12, count: 3 }], "old side");
        assert_eq!(files[0].new_ranges, vec![LineRange { start: 12, count: 4 }], "new side");
    }

    #[test]
    fn an_omitted_count_means_one_line() {
        let diff = "\
diff --git a/a.py b/a.py
--- a/a.py
+++ b/a.py
@@ -7 +7 @@
-x = 1
+x = 2
";
        let files = parse(diff);
        assert_eq!(files[0].old_ranges, vec![LineRange { start: 7, count: 1 }], "`-7` is one line");
        assert_eq!(files[0].new_ranges, vec![LineRange { start: 7, count: 1 }], "`+7` is one line");
    }

    #[test]
    fn a_zero_count_side_is_an_insertion_point_not_a_touched_line() {
        let diff = "\
diff --git a/a.py b/a.py
--- a/a.py
+++ b/a.py
@@ -5,0 +6,2 @@ def f():
+    a = 1
+    b = 2
";
        let files = parse(diff);
        assert!(
            files[0].old_ranges.is_empty(),
            "a pure insertion touches no old line, got {:?}",
            files[0].old_ranges
        );
        assert_eq!(files[0].new_ranges, vec![LineRange { start: 6, count: 2 }], "two lines added");
    }

    #[test]
    fn an_added_file_has_no_old_path() {
        let diff = "\
diff --git a/new.py b/new.py
new file mode 100644
index 0000000..3333333
--- /dev/null
+++ b/new.py
@@ -0,0 +1,3 @@
+def f():
+    pass
+
";
        let files = parse(diff);
        assert_eq!(files[0].old_path, None, "/dev/null means the file did not exist");
        assert_eq!(files[0].new_path.as_deref(), Some(Path::new("new.py")), "new side named");
        assert!(files[0].old_ranges.is_empty(), "nothing to remove from a file that did not exist");
    }

    #[test]
    fn a_deleted_file_has_no_new_path() {
        let diff = "\
diff --git a/gone.py b/gone.py
deleted file mode 100644
index 3333333..0000000
--- a/gone.py
+++ /dev/null
@@ -1,3 +0,0 @@
-def f():
-    pass
-
";
        let files = parse(diff);
        assert_eq!(files[0].old_path.as_deref(), Some(Path::new("gone.py")), "old side named");
        assert_eq!(files[0].new_path, None, "/dev/null means the file is gone");
        assert_eq!(
            files[0].old_ranges,
            vec![LineRange { start: 1, count: 3 }],
            "all lines removed"
        );
    }

    #[test]
    fn a_rename_carries_both_paths() {
        let diff = "\
diff --git a/old/name.py b/new/name.py
similarity index 92%
rename from old/name.py
rename to new/name.py
index 4444444..5555555 100644
--- a/old/name.py
+++ b/new/name.py
@@ -3 +3 @@ def f():
-    return 1
+    return 2
";
        let files = parse(diff);
        assert_eq!(files[0].old_path.as_deref(), Some(Path::new("old/name.py")), "source path");
        assert_eq!(files[0].new_path.as_deref(), Some(Path::new("new/name.py")), "target path");
    }

    #[test]
    fn a_pure_rename_with_no_hunks_still_reports_both_paths() {
        let diff = "\
diff --git a/old/name.py b/new/name.py
similarity index 100%
rename from old/name.py
rename to new/name.py
";
        let files = parse(diff);
        assert_eq!(
            files[0].old_path.as_deref(),
            Some(Path::new("old/name.py")),
            "the `diff --git` header is the only path source when there are no ---/+++ lines"
        );
        assert_eq!(files[0].new_path.as_deref(), Some(Path::new("new/name.py")), "target path");
        assert!(files[0].old_ranges.is_empty(), "a 100% rename changes no lines");
    }

    #[test]
    fn a_rename_of_a_path_containing_the_separator_is_parsed_from_the_rename_lines() {
        // `git mv 'x b/y.py' 'z b/y.py'` really does emit this header, and the
        // first ` b/` in it is not the separator.
        let diff = "\
diff --git a/x b/y.py b/z b/y.py
similarity index 100%
rename from x b/y.py
rename to z b/y.py
";
        let files = parse(diff);
        assert_eq!(
            files[0].old_path.as_deref(),
            Some(Path::new("x b/y.py")),
            "` b/` inside a path must not be mistaken for the header separator"
        );
        assert_eq!(files[0].new_path.as_deref(), Some(Path::new("z b/y.py")), "target path");
    }

    #[test]
    fn an_unparseable_hunk_header_is_skipped_rather_than_fatal() {
        // Under-reporting beats crashing: a header gerenuk cannot read is
        // dropped, and parsing carries on with the next one.
        let diff = "\
diff --git a/a.py b/a.py
--- a/a.py
+++ b/a.py
@@ -notanumber +1 @@
+x = 1
@@ -5 +5 @@
-y
+z
";
        let files = parse(diff);
        assert_eq!(
            files[0].new_ranges,
            vec![LineRange { start: 5, count: 1 }],
            "the bad header is dropped and the good one still parses"
        );
    }

    #[test]
    fn a_binary_file_is_flagged_and_has_no_ranges() {
        let diff = "\
diff --git a/img.png b/img.png
index 6666666..7777777 100644
Binary files a/img.png and b/img.png differ
";
        let files = parse(diff);
        assert!(files[0].binary, "binary files must be recognisable");
        assert!(files[0].new_ranges.is_empty(), "no line information exists for binaries");
        assert_eq!(files[0].new_path.as_deref(), Some(Path::new("img.png")), "path from header");
    }

    #[test]
    fn multiple_files_are_split_apart() {
        let diff = "\
diff --git a/a.py b/a.py
--- a/a.py
+++ b/a.py
@@ -1 +1 @@
-a
+b
diff --git a/b.py b/b.py
--- a/b.py
+++ b/b.py
@@ -9,2 +9,2 @@
-c
-d
+e
+f
";
        let files = parse(diff);
        assert_eq!(files.len(), 2, "two `diff --git` headers means two files");
        assert_eq!(files[1].new_path.as_deref(), Some(Path::new("b.py")), "second file parsed");
        assert_eq!(files[1].new_ranges, vec![LineRange { start: 9, count: 2 }], "second hunk");
    }

    #[test]
    fn several_hunks_in_one_file_all_survive() {
        let diff = "\
diff --git a/a.py b/a.py
--- a/a.py
+++ b/a.py
@@ -1 +1 @@
-a
+b
@@ -20,2 +20,1 @@
-c
-d
+e
";
        let files = parse(diff);
        assert_eq!(
            files[0].new_ranges,
            vec![LineRange { start: 1, count: 1 }, LineRange { start: 20, count: 1 }],
            "both hunks are recorded in order"
        );
    }

    #[test]
    fn a_body_line_that_looks_like_a_header_is_not_one() {
        // With -U0 every body line carries a `+`/`-`, so a literal `@@` in the
        // source can never be mistaken for a hunk header.
        let diff = "\
diff --git a/a.py b/a.py
--- a/a.py
+++ b/a.py
@@ -1 +1 @@
-COMMENT = \"@@ -1 +1 @@\"
+COMMENT = \"diff --git a/x b/y\"
";
        let files = parse(diff);
        assert_eq!(files.len(), 1, "the quoted header text must not start a new file");
        assert_eq!(files[0].new_ranges.len(), 1, "and must not add a hunk");
    }

    #[test]
    fn an_empty_diff_yields_nothing() {
        assert!(parse("").is_empty(), "no diff, no files");
    }

    #[test]
    fn a_line_range_expands_to_its_lines() {
        let range = LineRange { start: 10, count: 3 };
        assert_eq!(range.lines().collect::<Vec<_>>(), vec![10, 11, 12], "inclusive of start");
    }

    #[test]
    fn touched_lines_merges_overlapping_ranges() {
        let ranges = [
            LineRange { start: 5, count: 3 },
            LineRange { start: 6, count: 1 },
            LineRange { start: 1, count: 1 },
        ];
        assert_eq!(
            touched_lines(&ranges),
            vec![1, 5, 6, 7],
            "output is sorted and free of duplicates"
        );
    }
}
