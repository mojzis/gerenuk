//! The path patterns `ignore-paths` is written in.
//!
//! Deliberately small: `*` and `?` within one path segment, `**` as a whole
//! segment for any number of them, everything else literal. A pattern is
//! matched against the whole repository-relative path, so `*.md` means the
//! root's Markdown and `**/*.md` means all of it. The narrow reading is the
//! safe one: a pattern that matches less leaves more files to say `run_all`.
//!
//! Anything that would read as glob syntax we do not implement — a character
//! class, a brace alternation — is refused by [`validate`] rather than taken
//! literally, because a pattern that silently matches nothing is a
//! configuration that looks like it works.

use anyhow::{bail, Result};

/// Check one pattern, so a bad one fails at startup rather than never matching.
pub fn validate(pattern: &str) -> Result<()> {
    if pattern.is_empty() {
        bail!("an empty pattern matches nothing");
    }
    if pattern.starts_with('/') {
        bail!("`{pattern}` is absolute; patterns are relative to the repository root");
    }
    if let Some(found) = pattern.chars().find(|c| matches!(c, '[' | ']' | '{' | '}' | '\\')) {
        bail!("`{pattern}` uses `{found}`, which is not supported: only `*`, `?` and `**` are");
    }
    if pattern.split('/').any(|segment| segment.contains("**") && segment != "**") {
        bail!("`{pattern}`: `**` must be a whole path segment, as in `docs/**` or `**/*.md`");
    }
    Ok(())
}

/// Whether `path`, repository-relative and `/`-separated, matches `pattern`.
#[must_use]
pub fn matches(pattern: &str, path: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    let path: Vec<&str> = path.split('/').collect();
    segments(&pattern, &path)
}

fn segments(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|skip| segments(rest, &path[skip..])),
        Some((first, rest)) => path
            .split_first()
            .is_some_and(|(name, tail)| segment(first, name) && segments(rest, tail)),
    }
}

/// One segment against one name: `*` is any run of characters, `?` exactly one.
fn segment(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    // The classic two-pointer wildcard match: on a mismatch, let the last `*`
    // swallow one more character and retry from there.
    let (mut p, mut n) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p, n));
                p += 1;
            }
            Some(&c) if c == '?' || c == name[n] => {
                p += 1;
                n += 1;
            }
            _ => match star {
                Some((sp, sn)) => {
                    p = sp + 1;
                    n = sn + 1;
                    star = Some((sp, sn + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_double_star_prefix_matches_at_any_depth_including_the_root() {
        assert!(matches("**/*.md", "README.md"), "zero directories");
        assert!(matches("**/*.md", "docs/guide/setup.md"), "several");
        assert!(!matches("**/*.md", "docs/guide/setup.mdx"), "the suffix is anchored");
    }

    #[test]
    fn a_double_star_suffix_matches_everything_under_a_directory() {
        assert!(matches("docs/**", "docs/index.md"));
        assert!(matches("docs/**", "docs/src/img/logo.png"));
        assert!(!matches("docs/**", "documentation/index.md"), "a segment is not a prefix");
        assert!(!matches("docs/**", "src/docs/index.md"), "the pattern is anchored at the root");
    }

    #[test]
    fn a_double_star_in_the_middle_spans_zero_or_more_segments() {
        assert!(matches(".claude/**/SKILL.md", ".claude/SKILL.md"));
        assert!(matches(".claude/**/SKILL.md", ".claude/skills/steward/SKILL.md"));
        assert!(!matches(".claude/**/SKILL.md", ".claude/skills/steward/README.md"));
    }

    #[test]
    fn a_single_star_stays_within_one_segment() {
        assert!(matches("*.md", "CHANGELOG.md"));
        assert!(
            !matches("*.md", "docs/index.md"),
            "the narrow reading: a pattern with no `/` is not a basename match"
        );
        assert!(matches("docs/*.md", "docs/index.md"));
        assert!(!matches("docs/*.md", "docs/guide/index.md"));
        assert!(matches("CHANGE*", "CHANGELOG.md"));
        assert!(matches("*LOG*", "CHANGELOG.md"));
        assert!(matches("*", "LICENSE"));
    }

    #[test]
    fn a_question_mark_is_exactly_one_character() {
        assert!(matches("doc?.md", "docs.md"));
        assert!(!matches("doc?.md", "doc.md"));
        assert!(!matches("a?b", "a/b"), "and never a separator");
    }

    #[test]
    fn a_literal_pattern_matches_only_itself() {
        assert!(matches("README.md", "README.md"));
        assert!(!matches("README.md", "README.md.bak"));
        assert!(!matches("README.md", "docs/README.md"));
    }

    #[test]
    fn stars_backtrack() {
        assert!(matches("*a*b", "xaxxab"), "the first `a` is not the one that matters");
        assert!(!matches("*a*b", "xaxxa"));
    }

    #[test]
    fn the_patterns_the_issue_names_are_valid() {
        for pattern in ["**/*.md", "docs/**", "CHANGELOG.md", "*.rst", ".claude/**"] {
            assert!(validate(pattern).is_ok(), "`{pattern}` should be accepted");
        }
    }

    #[test]
    fn unsupported_syntax_is_refused_rather_than_taken_literally() {
        for (pattern, why) in [
            ("", "empty"),
            ("/docs/**", "absolute"),
            ("docs/[ab].md", "a character class"),
            ("**/*.{md,rst}", "a brace alternation"),
            ("docs\\*.md", "a backslash"),
            ("docs/**.md", "`**` inside a segment"),
        ] {
            assert!(validate(pattern).is_err(), "`{pattern}` should be refused: {why}");
        }
    }
}
