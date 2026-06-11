# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
While pre-1.0, the minor version is bumped for breaking changes.

<!-- next-header -->

## [Unreleased] - ReleaseDate

### Added

- `gitj --commit` (or `-c`) opens the commit screen right away instead of the
  history browser — for when the working tree is already in shape and the only
  reason to launch is to stage and commit. With it, argument handling grew into
  a real parser: `-h`/`--help` and `-V`/`--version` print and exit, a bare `--`
  keeps paths that start with `-` reachable, and an unknown option or extra
  argument fails with usage help instead of being silently taken for a path.
  (#9)

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
