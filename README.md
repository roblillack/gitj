# journey

A gitk-style repository browser built on the
[retrogui](../retrofetch/retrogui) toolkit — a Windows 3.1–flavored,
software-rendered git history viewer.

```
journey            # browse the repository containing the current directory
journey /path/repo # browse the repository at (or above) a given path
```

## Features

* **Commit history** with a colored DAG **graph** column, branch / tag /
  HEAD **ref badges**, and author + date columns.
* **`git show`-style detail**: selecting a commit shows its SHA, refs,
  author, date, message and full diff; selecting a file narrows the diff to
  that file.
* **Diff view** with the usual coloring — green additions, red deletions,
  blue hunk headers, gray file headers.
* **Search / filter** the history live by message, author, ref or SHA.
* **Menu bar** (File ▸ Reload / Exit, Help ▸ About) with Alt-accelerators and
  a modal About dialog.
* Keyboard navigation throughout: Tab cycles the panes, arrows/PageUp/Down
  drive the focused list or scroll the diff.

## Layout

```
┌───────────────────────────────────────────────┐
│ File  Help                          (menu bar) │
│ Find: [ filter query ]              (toolbar)  │
├───────────────────────────────────────────────┤
│ ●│ refs  summary            author      date   │  commit history
│ ●│ ...                                          │  (graph + list)
├──────────────────────────┬────────────────────┤
│ commit detail + diff      │ M changed/file.rs  │  diff  │  files
│ (git show)                │ A another.rs       │
└──────────────────────────┴────────────────────┘
```

## Architecture

The UI never touches `git2` directly — it goes through a small backend
abstraction, which keeps everything testable without a live repository.

| Module | Contents |
|--------|----------|
| `backend` | `RepoBackend` trait + data types (`CommitInfo`, `FileChange`, `Diff`/`DiffLine`, `RefLabel`). Implementations: `Git2Backend` (live, libgit2) and `FixtureBackend` (deterministic, in-memory). |
| `widgets` | git-specific widgets — `CommitList` (graph + badges + columns), `DiffView` (colored diff), `SearchBar`, `Panes` (the gitk shell layout / focus scope), `graph` (DAG lane assignment), generic `Shared<W>` adapter. |
| `ui` | `GitClient`, the top-level widget wiring the panes together and syncing them on selection / search changes. |

## Testing

* **Pixel snapshots** (`tests/snapshots.rs`) render the real UI through
  retrogui's offscreen `MockBackend` at 1.0/1.25/1.5/2.0× against the
  in-memory `FixtureBackend` and bundled DejaVu fonts, comparing PNG bytes
  with `insta`. Regenerate with `INSTA_UPDATE=always cargo test --test
  snapshots`, then review with `cargo insta review`.
* **`tests/git2_backend.rs`** builds a throwaway repository with fixed
  signatures/timestamps and reads it back through `Git2Backend`, so the live
  backend is covered deterministically.
* **Unit tests** cover the graph lane algorithm and date formatting.

```
cargo test          # everything
cargo build         # the binary
```
