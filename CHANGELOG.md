# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
While pre-1.0, the minor version is bumped for breaking changes.

<!-- next-header -->

## [Unreleased] - ReleaseDate

### Added

- Mouse-wheel and trackpad scrolling: the commit history, the review screen's
  branch list and the diff views scroll under the pointer — a wheel notch
  moves three lines, trackpad gestures scroll smoothly — leaving the selection
  untouched. The file lists and the commit message editor get the same
  behavior from the toolkit, so every scrollable pane now responds to the
  wheel. (#14)
- Branch review mode: a third screen (View ▸ Review Branches, Ctrl+3) lists
  every local and remote-tracking branch — the checked-out one first, and a
  remote folded into its local's row when both sit at the same tip — each with
  its tip commit's summary, author and date. Selecting a branch reviews
  everything it contains: the aggregated diff from its merge base with the
  repository's default branch (the remote's declared default when known,
  otherwise main/master) to its tip, so commits the default branch gained
  since the branch point don't show up as noise. The file list narrows the
  diff to a single file, and changed images get the graphical image diff here
  as well. Review diffs share the huge-diff safeguards: rename detection is
  skipped past the candidate-pair cap and rendering truncates at 50,000
  lines. (#10)
- Graphical image diffs: selecting a changed image (PNG, JPEG, GIF, WebP, BMP,
  TIFF, …) now shows the two versions visually instead of a "Binary files
  differ" line. Four comparison modes — 2-up side by side, a swipe split, an
  onion-skin cross-fade, and a per-pixel difference heatmap — are switched
  from the button row or the new View menu (Switch Mode, Ctrl+M), with a
  slider driving the swipe/onion position; Before/After Image (Ctrl+Left /
  Ctrl+Right) show just the old/new side full size, and Next/Previous Image
  (Ctrl+N / Ctrl+P) step between the image files in the active list. Works in
  both the browse and commit diff panes. Behind it, the backend gained
  raw-blob access to both sides of a change, decoding goes through the `image`
  crate, and the composed comparison canvas is cached and bulk-blitted to the
  framebuffer via Saudade 0.5's new API. (#11)
- More keyboard shortcuts: Ctrl+1 / Ctrl+2 / Ctrl+3 switch between the Browse
  History, Commit Changes and Review Branches screens — the active one is
  checkmarked in the View menu — and Ctrl+U unstages the selected file.
  (#10, #11)
- `gitj --commit` (or `-c`) opens the commit screen right away instead of the
  history browser — for when the working tree is already in shape and the only
  reason to launch is to stage and commit. With it, argument handling grew into
  a real parser: `-h`/`--help` and `-V`/`--version` print and exit, a bare `--`
  keeps paths that start with `-` reachable, and an unknown option or extra
  argument fails with usage help instead of being silently taken for a path.
  (#9)

### Changed

- The history browser now shows only commits reachable from the checked-out
  branch, like plain `gitk`, instead of walking every local branch,
  remote-tracking branch and tag. Other refs that point into the visible
  history still decorate their commits' rows. (#13)

### Fixed

- Selecting a huge merge commit no longer freezes the UI for up to a minute.
  Two costs are now capped in the libgit2 backend: rename/copy detection is
  skipped when the added × deleted candidate-pair product exceeds 100,000 —
  its cost tracks that product, not the file count, so renames among a few
  hundred files per side are still caught while a 4,000 × 3,500 merge isn't
  allowed to take ~49s — and the rendered diff is cut off at 50,000 lines with
  a trailing truncation marker instead of materializing millions of patch
  lines nobody scrolls through. Skipped renames show as add/delete pairs, the
  same fallback git's CLI uses past `diff.renameLimit`; the per-file lists
  still show every changed file. (#12)

## [0.2.0] - 2026-06-08

### Added

- Partial staging by line range: highlight rows in the commit screen's diff
  view and a floating Stage/Unstage button applies exactly those lines to the
  index; clicking a hunk header selects the whole hunk. Behind it, the
  displayed diff is rebuilt into a minimal unified-diff patch covering only the
  selection and applied forward to the index, `git apply --cached`-style, so
  staging and unstaging share one mechanism. (#8)
- File lists mark each entry's git status with a colored marker in the icon
  gutter — the same A/M/D/… letters the old text badge printed, but baked from
  `assets/status/*.svg` at compile time via saudade's `include_svg!`, so they
  stay crisp at any DPI and legible on the navy selection band. (#7)
- The window now enforces a minimum inner size of 450×320 logical pixels, so
  the browse and commit layouts can't be squeezed into collapse. (#6)

### Changed

- The browse and commit screens size their panes — buttons included —
  proportionally to the window width instead of using fixed splits, so
  widening the window actually distributes the extra space. (#5)
- Widgets on the commit screen are now vertically aligned. (#4)
- The scrollbar outline collapses into the surrounding border of the diff view
  and the commit list, so shared edges draw as a single 1px line instead of
  doubling up. (#3)

## [0.1.0] - 2026-06-07

Initial release.

<!-- next-url -->
[Unreleased]: https://github.com/roblillack/gitj/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/roblillack/gitj/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/roblillack/gitj/releases/tag/v0.1.0
