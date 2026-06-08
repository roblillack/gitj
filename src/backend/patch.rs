//! Reconstructs a minimal unified-diff patch covering only a *selected* range
//! of lines, so the commit screen can stage or unstage part of a file instead
//! of the whole thing — the engine behind the Stage/Unstage button that floats
//! over a highlighted region in the diff view.
//!
//! The displayed diff (one file, with its `diff --git` / hunk headers and `+`/
//! `-`/context body) is filtered down to the rows the user highlighted, and the
//! touched hunks are rebuilt with corrected line counts. The result is always
//! oriented to apply **forward** to the index (`git apply --cached`), so the
//! direction (stage vs. unstage) is baked into the patch rather than left to the
//! caller — see [`RepoBackend::apply_to_index`](super::RepoBackend::apply_to_index).

use std::collections::BTreeSet;

use super::{Diff, DiffLineKind};

/// Which way a partial patch runs. Both produce a patch applied forward to the
/// index; the difference is which displayed diff it is built from and how the
/// selected `+`/`-` lines are mapped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PartialMode {
    /// Stage selected lines, built from the unstaged (index→workdir) diff: the
    /// selected changes are carried straight through.
    Stage,
    /// Unstage selected lines, built from the staged (HEAD→index) diff and
    /// reversed, so applying it to the index backs the changes out.
    Unstage,
}

/// Whether a diff row is an actual change (`+`/`-`) — the only kind of row a
/// partial stage/unstage can act on. Header and context rows may fall inside a
/// selection but never make it stageable on their own.
pub fn is_change_line(kind: DiffLineKind) -> bool {
    matches!(kind, DiffLineKind::Addition | DiffLineKind::Deletion)
}

/// Build a patch staging/unstaging exactly the `selected` rows of `diff` (row
/// indices into `diff.lines`). Returns `None` when the selection covers no
/// changed line, so the caller can treat it as a no-op.
pub fn build_partial_patch(
    diff: &Diff,
    selected: &BTreeSet<usize>,
    mode: PartialMode,
) -> Option<String> {
    let lines = &diff.lines;
    let mut out = String::new();
    let mut emitted_any = false;

    // Header lines for the file currently being processed (verbatim, minus the
    // `index` line — see `keep_header_line`), written lazily before the file's
    // first emitted hunk so files with no selected change produce no output.
    let mut file_header: Vec<&str> = Vec::new();
    let mut file_header_written = false;
    // Running new-line offset accumulated over the hunks emitted for this file,
    // so each rebuilt hunk's `+` start stays consistent with the ones before it.
    let mut delta: i64 = 0;

    let mut i = 0;
    while i < lines.len() {
        match lines[i].kind {
            DiffLineKind::FileHeader => {
                file_header.clear();
                file_header_written = false;
                delta = 0;
                while i < lines.len() && lines[i].kind == DiffLineKind::FileHeader {
                    if keep_header_line(&lines[i].text) {
                        file_header.push(&lines[i].text);
                    }
                    i += 1;
                }
            }
            DiffLineKind::HunkHeader => {
                let parsed = parse_hunk_header(&lines[i].text);
                let body_start = i + 1;
                let mut body_end = body_start;
                while body_end < lines.len()
                    && !matches!(
                        lines[body_end].kind,
                        DiffLineKind::HunkHeader | DiffLineKind::FileHeader
                    )
                {
                    body_end += 1;
                }

                if let (Some((old_a, new_c)), Some(hunk)) = (
                    parsed,
                    rebuild_hunk(lines, body_start, body_end, selected, mode),
                ) {
                    if !file_header_written {
                        for h in &file_header {
                            out.push_str(h);
                            out.push('\n');
                        }
                        file_header_written = true;
                    }
                    // The patch applies forward to the index, so its "old" side
                    // is the index: the unstaged diff's `-` start when staging,
                    // the staged diff's `+` start when unstaging.
                    let old_start = match mode {
                        PartialMode::Stage => old_a,
                        PartialMode::Unstage => new_c,
                    };
                    let new_start = (old_start as i64 + delta).max(0);
                    out.push_str(&format!(
                        "@@ -{},{} +{},{} @@\n",
                        old_start, hunk.old_count, new_start, hunk.new_count
                    ));
                    out.push_str(&hunk.body);
                    delta += hunk.new_count as i64 - hunk.old_count as i64;
                    emitted_any = true;
                }
                i = body_end;
            }
            // Stray rows outside any hunk (e.g. the commit-detail header the
            // browse view prepends) are never part of a stageable file diff.
            _ => i += 1,
        }
    }

    emitted_any.then_some(out)
}

/// A single rebuilt hunk: its body text and the line counts for its header.
struct RebuiltHunk {
    body: String,
    old_count: usize,
    new_count: usize,
}

/// Rebuild one hunk body (`lines[start..end]`) keeping only the selected
/// changes; unselected changes are folded into context or dropped per `mode`.
/// Returns `None` if no changed line in the hunk was selected.
fn rebuild_hunk(
    lines: &[super::DiffLine],
    start: usize,
    end: usize,
    selected: &BTreeSet<usize>,
    mode: PartialMode,
) -> Option<RebuiltHunk> {
    let mut body = String::new();
    let mut old_count = 0;
    let mut new_count = 0;
    let mut has_change = false;
    let mut prev_emitted = false;

    for (idx, line) in lines.iter().enumerate().take(end).skip(start) {
        // A "\ No newline at end of file" marker rides along with the line it
        // annotates: keep it only when that line was emitted.
        if line.kind == DiffLineKind::Meta {
            if prev_emitted && line.text.starts_with('\\') {
                body.push_str(&line.text);
                body.push('\n');
            }
            continue;
        }

        let selected_here = selected.contains(&idx);
        let new_origin = map_origin(line.kind, selected_here, mode);
        let Some(origin) = new_origin else {
            prev_emitted = false;
            continue;
        };

        if selected_here && is_change_line(line.kind) {
            has_change = true;
        }
        // Body rows always carry a leading origin byte (' '/'+'/'-'); swap it
        // for the rebuilt one.
        let content = &line.text[1..];
        body.push(origin);
        body.push_str(content);
        body.push('\n');
        match origin {
            ' ' => {
                old_count += 1;
                new_count += 1;
            }
            '-' => old_count += 1,
            '+' => new_count += 1,
            _ => {}
        }
        prev_emitted = true;
    }

    has_change.then_some(RebuiltHunk {
        body,
        old_count,
        new_count,
    })
}

/// The origin character a body row gets in the rebuilt (forward-to-index)
/// patch, or `None` to drop the row entirely.
fn map_origin(kind: DiffLineKind, selected: bool, mode: PartialMode) -> Option<char> {
    match (kind, mode) {
        (DiffLineKind::Context, _) => Some(' '),
        // Stage: selected changes carry through; an unselected deletion stays
        // present (context), an unselected addition isn't staged (dropped).
        (DiffLineKind::Addition, PartialMode::Stage) => selected.then_some('+'),
        (DiffLineKind::Deletion, PartialMode::Stage) => Some(if selected { '-' } else { ' ' }),
        // Unstage reverses the staged diff: a selected addition is removed from
        // the index (`-`), a selected deletion is restored (`+`); unselected
        // additions stay (context) and unselected deletions are already gone.
        (DiffLineKind::Addition, PartialMode::Unstage) => Some(if selected { '-' } else { ' ' }),
        (DiffLineKind::Deletion, PartialMode::Unstage) => selected.then_some('+'),
        _ => None,
    }
}

/// Keep every file-header line except `index <old>..<new>`: the rebuilt content
/// won't match those blob OIDs, and libgit2's apply is happy without it for text
/// patches (the `new file` / `deleted file` mode lines, which it does need, stay).
fn keep_header_line(text: &str) -> bool {
    !text.starts_with("index ")
}

/// Parse the old/new *start* line numbers from a `@@ -a,b +c,d @@` header.
/// Counts are ignored (rebuilt hunks recompute them); a missing `,count` is
/// fine. Returns `None` for a malformed header.
fn parse_hunk_header(text: &str) -> Option<(usize, usize)> {
    let rest = text.strip_prefix("@@ ")?;
    let mut parts = rest.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let a = old.split(',').next()?.parse().ok()?;
    let c = new.split(',').next()?.parse().ok()?;
    Some((a, c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::DiffLine;

    fn diff(rows: &[(DiffLineKind, &str)]) -> Diff {
        Diff {
            lines: rows
                .iter()
                .map(|(k, t)| DiffLine::new(*k, t.to_string()))
                .collect(),
        }
    }

    // A modified file with one hunk: one deletion, two additions, plus context.
    fn sample() -> Diff {
        use DiffLineKind::*;
        diff(&[
            (FileHeader, "diff --git a/src/x.rs b/src/x.rs"),
            (FileHeader, "index 1111111..2222222 100644"),
            (FileHeader, "--- a/src/x.rs"),
            (FileHeader, "+++ b/src/x.rs"),
            (HunkHeader, "@@ -10,4 +10,5 @@ fn f() {"),
            (Context, "     let a = 1;"),
            (Deletion, "-    let b = 2;"),
            (Addition, "+    let b = 3;"),
            (Addition, "+    let c = 4;"),
            (Context, "     done();"),
        ])
    }

    fn rows(range: std::ops::RangeInclusive<usize>) -> BTreeSet<usize> {
        range.collect()
    }

    #[test]
    fn empty_selection_yields_nothing() {
        assert!(build_partial_patch(&sample(), &BTreeSet::new(), PartialMode::Stage).is_none());
        // A selection that only covers header/context rows is not stageable.
        assert!(build_partial_patch(&sample(), &rows(0..=5), PartialMode::Stage).is_none());
    }

    #[test]
    fn stage_only_the_deletion_turns_additions_into_nothing() {
        // Select just the `-` row (index 6).
        let patch = build_partial_patch(&sample(), &rows(6..=6), PartialMode::Stage).unwrap();
        let expected = "\
diff --git a/src/x.rs b/src/x.rs
--- a/src/x.rs
+++ b/src/x.rs
@@ -10,3 +10,2 @@
     let a = 1;
-    let b = 2;
     done();
";
        assert_eq!(patch, expected);
    }

    #[test]
    fn stage_only_the_additions_keeps_deletion_as_context() {
        // Select the two `+` rows (indices 7,8) but not the `-`.
        let patch = build_partial_patch(&sample(), &rows(7..=8), PartialMode::Stage).unwrap();
        let expected = "\
diff --git a/src/x.rs b/src/x.rs
--- a/src/x.rs
+++ b/src/x.rs
@@ -10,3 +10,5 @@
     let a = 1;
     let b = 2;
+    let b = 3;
+    let c = 4;
     done();
";
        assert_eq!(patch, expected);
    }

    #[test]
    fn unstage_reverses_origins() {
        // Same displayed hunk, but interpret it as the staged diff and unstage
        // the two additions: they become deletions, the deletion (unselected)
        // drops out, and the hunk's old side is the index (`+10`).
        let patch = build_partial_patch(&sample(), &rows(7..=8), PartialMode::Unstage).unwrap();
        let expected = "\
diff --git a/src/x.rs b/src/x.rs
--- a/src/x.rs
+++ b/src/x.rs
@@ -10,4 +10,2 @@
     let a = 1;
-    let b = 3;
-    let c = 4;
     done();
";
        assert_eq!(patch, expected);
    }

    #[test]
    fn second_hunk_new_start_tracks_prior_emitted_delta() {
        use DiffLineKind::*;
        let d = diff(&[
            (FileHeader, "diff --git a/f b/f"),
            (FileHeader, "--- a/f"),
            (FileHeader, "+++ b/f"),
            (HunkHeader, "@@ -10,2 +10,3 @@"),
            (Context, " keep1"),
            (Addition, "+added-a"),
            (Context, " keep2"),
            (HunkHeader, "@@ -50,2 +51,3 @@"),
            (Context, " keep3"),
            (Addition, "+added-b"),
            (Context, " keep4"),
        ]);
        // Select both additions (indices 5 and 9).
        let mut sel = BTreeSet::new();
        sel.insert(5);
        sel.insert(9);
        let patch = build_partial_patch(&d, &sel, PartialMode::Stage).unwrap();
        // First hunk adds one line (delta +1), so the second hunk's new start is
        // its old start (50) shifted by that delta → 51.
        let expected = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -10,2 +10,3 @@
 keep1
+added-a
 keep2
@@ -50,2 +51,3 @@
 keep3
+added-b
 keep4
";
        assert_eq!(patch, expected);
    }

    #[test]
    fn unselected_hunk_is_omitted_entirely() {
        use DiffLineKind::*;
        let d = diff(&[
            (FileHeader, "diff --git a/f b/f"),
            (FileHeader, "--- a/f"),
            (FileHeader, "+++ b/f"),
            (HunkHeader, "@@ -10,2 +10,3 @@"),
            (Context, " keep1"),
            (Addition, "+added-a"),
            (Context, " keep2"),
            (HunkHeader, "@@ -50,2 +51,3 @@"),
            (Context, " keep3"),
            (Addition, "+added-b"),
            (Context, " keep4"),
        ]);
        // Select only the second hunk's addition (index 9).
        let patch = build_partial_patch(&d, &rows(9..=9), PartialMode::Stage).unwrap();
        let expected = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -50,2 +50,3 @@
 keep3
+added-b
 keep4
";
        assert_eq!(patch, expected);
    }
}
