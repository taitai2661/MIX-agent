//! Minimal unified-diff generator used by the file-write preview.
//!
//! Strategy: trim the common prefix/suffix (which covers the common "small
//! edit inside a large file" case), run an LCS over the remaining middle, and
//! group the result into hunks with a fixed context size. Inputs beyond the
//! size cap fall back to a single whole-file hunk so the preview stays O(1)
//! regardless of what the model tries to write.

/// Maximum number of lines per side fed into the LCS table. Beyond this the
/// diff degrades to a whole-file replacement hunk.
const MAX_LCS_LINES: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    pub unified: String,
    pub added: u32,
    pub removed: u32,
    pub is_new: bool,
}

pub fn unified_diff(path: &str, old: &str, new: &str, context: usize) -> FileDiff {
    let old_lines = split_lines(old);
    let new_lines = split_lines(new);
    let is_new = old_lines.is_empty() && !new_lines.is_empty();

    let mut added = 0u32;
    let mut removed = 0u32;

    let ops = diff_ops(&old_lines, &new_lines);
    for op in &ops {
        match op.tag {
            Tag::Keep => {}
            Tag::Delete => removed += 1,
            Tag::Insert => added += 1,
        }
    }

    let hunks = group_hunks(&ops, context);
    let mut unified = String::new();
    if !hunks.is_empty() {
        unified.push_str(&format!("--- a/{path}\n+++ b/{path}\n"));
        for hunk in &hunks {
            unified.push_str(&format!(
                "@@ -{} +{} @@\n",
                range_label(hunk.old_start, hunk.old_count),
                range_label(hunk.new_start, hunk.new_count),
            ));
            for line in &hunk.lines {
                match line.tag {
                    Tag::Keep => unified.push(' '),
                    Tag::Delete => unified.push('-'),
                    Tag::Insert => unified.push('+'),
                }
                unified.push_str(line.text);
                unified.push('\n');
            }
        }
    }

    FileDiff {
        unified,
        added,
        removed,
        is_new,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tag {
    Keep,
    Delete,
    Insert,
}

#[derive(Debug, Clone)]
struct Op<'a> {
    tag: Tag,
    text: &'a str,
    /// 1-based line number in the old file (None for inserts).
    old_line: Option<usize>,
    /// 1-based line number in the new file (None for deletes).
    new_line: Option<usize>,
}

fn split_lines(input: &str) -> Vec<&str> {
    if input.is_empty() {
        return Vec::new();
    }
    input
        .split_inclusive('\n')
        .map(|l| l.strip_suffix('\n').unwrap_or(l))
        .collect()
}

/// Longest-common-subsequence diff over the two sides, with common
/// prefix/suffix trimming and a whole-file fallback past the size cap.
fn diff_ops<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<Op<'a>> {
    let mut ops: Vec<Op<'a>> = Vec::with_capacity(old.len() + new.len());

    let mut prefix = 0usize;
    while prefix < old.len() && prefix < new.len() && old[prefix] == new[prefix] {
        prefix += 1;
    }
    let mut suffix = 0usize;
    while suffix < old.len() - prefix
        && suffix < new.len() - prefix
        && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let mut old_line = 1usize;
    let mut new_line = 1usize;
    for text in old.iter().take(prefix) {
        ops.push(Op {
            tag: Tag::Keep,
            text,
            old_line: Some(old_line),
            new_line: Some(new_line),
        });
        old_line += 1;
        new_line += 1;
    }

    let mid_old = &old[prefix..old.len() - suffix];
    let mid_new = &new[prefix..new.len() - suffix];

    if mid_old.len() > MAX_LCS_LINES || mid_new.len() > MAX_LCS_LINES {
        for text in mid_old {
            ops.push(Op {
                tag: Tag::Delete,
                text,
                old_line: Some(old_line),
                new_line: None,
            });
            old_line += 1;
        }
        for text in mid_new {
            ops.push(Op {
                tag: Tag::Insert,
                text,
                old_line: None,
                new_line: Some(new_line),
            });
            new_line += 1;
        }
    } else {
        for op in lcs_ops(mid_old, mid_new, old_line, new_line) {
            ops.push(op);
        }
        old_line += mid_old.len();
        new_line += mid_new.len();
    }

    for i in 0..suffix {
        ops.push(Op {
            tag: Tag::Keep,
            text: old[old.len() - suffix + i],
            old_line: Some(old_line),
            new_line: Some(new_line),
        });
        old_line += 1;
        new_line += 1;
    }
    ops
}

/// Classic DP LCS. Both slices must be within [`MAX_LCS_LINES`].
fn lcs_ops<'a>(
    old: &[&'a str],
    new: &[&'a str],
    old_offset: usize,
    new_offset: usize,
) -> Vec<Op<'a>> {
    let n = old.len();
    let m = new.len();
    // table[i][j] = LCS length of old[i..] and new[j..]
    let mut table = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i][j] = if old[i] == new[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }

    let mut ops = Vec::new();
    let mut i = 0usize;
    let mut j = 0usize;
    let mut old_line = old_offset;
    let mut new_line = new_offset;
    while i < n && j < m {
        if old[i] == new[j] {
            ops.push(Op {
                tag: Tag::Keep,
                text: old[i],
                old_line: Some(old_line),
                new_line: Some(new_line),
            });
            i += 1;
            j += 1;
            old_line += 1;
            new_line += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            ops.push(Op {
                tag: Tag::Delete,
                text: old[i],
                old_line: Some(old_line),
                new_line: None,
            });
            i += 1;
            old_line += 1;
        } else {
            ops.push(Op {
                tag: Tag::Insert,
                text: new[j],
                old_line: None,
                new_line: Some(new_line),
            });
            j += 1;
            new_line += 1;
        }
    }
    while i < n {
        ops.push(Op {
            tag: Tag::Delete,
            text: old[i],
            old_line: Some(old_line),
            new_line: None,
        });
        i += 1;
        old_line += 1;
    }
    while j < m {
        ops.push(Op {
            tag: Tag::Insert,
            text: new[j],
            old_line: None,
            new_line: Some(new_line),
        });
        j += 1;
        new_line += 1;
    }
    ops
}

struct Hunk<'a> {
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
    lines: Vec<Op<'a>>,
}

/// Group ops into unified-diff hunks with `context` lines around each change.
fn group_hunks<'a>(ops: &[Op<'a>], context: usize) -> Vec<Hunk<'a>> {
    let changed: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| op.tag != Tag::Keep)
        .map(|(i, _)| i)
        .collect();
    if changed.is_empty() {
        return Vec::new();
    }

    // Merge change indices whose context windows overlap.
    let mut groups: Vec<(usize, usize)> = Vec::new(); // [first_change, last_change]
    for &idx in &changed {
        let lo = idx.saturating_sub(context);
        let hi = (idx + context).min(ops.len() - 1);
        match groups.last_mut() {
            Some((glo, ghi)) if lo <= *ghi + 1 => {
                *ghi = (*ghi).max(hi);
            }
            _ => groups.push((lo, hi)),
        }
    }

    groups
        .into_iter()
        .map(|(lo, hi)| {
            let slice = &ops[lo..=hi];
            let old_start = slice
                .iter()
                .find_map(|op| op.old_line)
                .unwrap_or_else(|| slice.first().and_then(|op| op.old_line).unwrap_or(1));
            let new_start = slice
                .iter()
                .find_map(|op| op.new_line)
                .unwrap_or_else(|| slice.first().and_then(|op| op.new_line).unwrap_or(1));
            Hunk {
                old_start,
                old_count: slice.iter().filter(|op| op.old_line.is_some()).count(),
                new_start,
                new_count: slice.iter().filter(|op| op.new_line.is_some()).count(),
                lines: slice.to_vec(),
            }
        })
        .collect()
}

/// `l,c` label; the count is omitted when it is exactly 1.
fn range_label(start: usize, count: usize) -> String {
    if count == 1 {
        start.to_string()
    } else if count == 0 {
        // An empty range points at the line just before it.
        format!("{},0", start.saturating_sub(1))
    } else {
        format!("{},{}", start, count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_inputs_produce_no_hunks() {
        let diff = unified_diff("a.txt", "one\ntwo\n", "one\ntwo\n", 3);
        assert_eq!(diff.unified, "");
        assert_eq!((diff.added, diff.removed), (0, 0));
        assert!(!diff.is_new);
    }

    #[test]
    fn new_file_is_flagged_and_fully_added() {
        let diff = unified_diff("new.txt", "", "hello\nworld\n", 3);
        assert!(diff.is_new);
        assert_eq!(diff.added, 2);
        assert_eq!(diff.removed, 0);
        assert!(diff.unified.contains("+hello"));
        assert!(diff.unified.contains("@@ -0,0 +1,2 @@"));
    }

    #[test]
    fn single_line_change() {
        let diff = unified_diff("a.txt", "alpha\nbeta\ngamma\n", "alpha\nBETA\ngamma\n", 3);
        assert_eq!(diff.added, 1);
        assert_eq!(diff.removed, 1);
        assert!(diff.unified.contains("-beta"));
        assert!(diff.unified.contains("+BETA"));
        assert!(diff.unified.contains(" alpha"));
        assert!(diff.unified.contains(" gamma"));
    }

    #[test]
    fn append_only() {
        let diff = unified_diff("a.txt", "one\n", "one\ntwo\n", 3);
        assert_eq!(diff.added, 1);
        assert_eq!(diff.removed, 0);
        assert!(diff.unified.contains("+two"));
    }

    #[test]
    fn delete_only() {
        let diff = unified_diff("a.txt", "one\ntwo\n", "one\n", 3);
        assert_eq!(diff.added, 0);
        assert_eq!(diff.removed, 1);
        assert!(diff.unified.contains("-two"));
    }

    #[test]
    fn prefix_and_suffix_are_trimmed() {
        let old: String = (0..500).map(|i| format!("line{i}\n")).collect();
        let mut new = old.clone();
        new.push_str("tail\n");
        let diff = unified_diff("big.txt", &old, &new, 3);
        assert_eq!(diff.added, 1);
        // Only the tail hunk should be present, not 500 context lines.
        assert!(diff.unified.lines().count() < 10);
    }

    #[test]
    fn oversize_input_falls_back_to_whole_file() {
        let old: String = (0..3_000).map(|i| format!("a{i}\n")).collect();
        let new: String = (0..3_000).map(|i| format!("b{i}\n")).collect();
        let diff = unified_diff("huge.txt", &old, &new, 3);
        assert_eq!(diff.added, 3_000);
        assert_eq!(diff.removed, 3_000);
        // One header pair + exactly two hunks (delete-all, add-all).
        assert_eq!(diff.unified.matches("@@").count(), 2);
    }

    #[test]
    fn empty_to_empty() {
        let diff = unified_diff("a.txt", "", "", 3);
        assert_eq!(diff.unified, "");
        assert!(!diff.is_new);
    }

    #[test]
    fn no_trailing_newline_is_handled() {
        let diff = unified_diff("a.txt", "one", "one\ntwo", 3);
        assert_eq!(diff.added, 1);
        assert!(diff.unified.contains("+two"));
    }
}
