# journey

A gitk-style repository browser built on the
[saudade](../saudade) toolkit — a Windows 3.1–flavored,
software-rendered git history viewer.

```
journey            # browse the repository containing the current directory
journey /path/repo # browse the repository at (or above) a given path
```

journey has two screens — the gitk-style **browse** (history) screen and the
`git gui`-style **commit** (staging) screen. Switch between them from the
**View** menu, by double-clicking a working-tree entry in the log, or
automatically: committing drops you back to the log.

### Browse

* **Commit history** with a colored DAG **graph** column, branch / tag /
  HEAD **ref badges**, and author + date columns.
* **`git show`-style detail**: selecting a commit shows its SHA, refs,
  author, date, message and full diff; selecting a file narrows the diff to
  that file.
* **Diff view** with the usual coloring — green additions, red deletions,
  blue hunk headers, gray file headers.
* **Working-tree entries**: when there are local changes, the log leads with
  "Uncommitted changes" / "Staged changes" rows (connected into the graph at
  `HEAD`). Selecting one previews its files and diff; **double-clicking** it
  opens the commit screen.
* **Search / filter** the history live by message, author, ref or SHA.

### Commit

* **Unstaged** and **Staged** file lists (à la `git gui`). Double-click a
  file to stage / unstage it, or use the **Stage** / **Unstage** buttons.
* The **diff pane** shows the selected file's change — working-tree-vs-index
  for unstaged files, index-vs-`HEAD` for staged ones.
* A multi-line **message editor** and a **Commit** button.
* **Amend last commit**: ticking the box pre-fills the editor with `HEAD`'s
  message *and* re-bases the staging view on `HEAD`'s parent, so the changes
  already in the last commit show up as staged. Unstage any of them to drop
  them from the amended commit; committing then rewrites `HEAD` rather than
  adding a new commit.
* **Rescan** re-reads the working tree. Committing returns to the log view,
  which now shows the new commit.

### Throughout

* **Menu bar** (File ▸ Reload / Exit, View ▸ switch screen, Help ▸ About) with
  Alt-accelerators and a modal About / error dialog.
* Keyboard navigation: Tab cycles the panes, arrows/PageUp/Down drive the
  focused list or scroll the diff.

## Layout

Browse screen:

```
┌───────────────────────────────────────────────┐
│ File  View  Help                    (menu bar) │
│ Find: [ filter query ]              (toolbar)  │
├───────────────────────────────────────────────┤
│ ●│ Uncommitted changes (2)   (dbl-click → ⎘)   │  working-tree rows
│ ●│ Staged changes (1)                          │
│ ●│ refs  summary            author      date   │  commit history
│ ●│ ...                                          │  (graph + list)
├──────────────────────────┬────────────────────┤
│ commit detail + diff      │ M changed/file.rs  │  diff  │  files
│ (git show)                │ A another.rs       │
└──────────────────────────┴────────────────────┘
```

Commit screen:

```
┌───────────────────────────────────────────────┐
│ File  Commit  View  Help            (menu bar) │
├──────────────────────────┬────────────────────┤
│ Unstaged Changes          │ diff of the        │
│ M src/ui.rs               │ selected file      │
│ ? notes.md                │                    │
├──────────────────────────┤                    │
│ Staged Changes            ├────────────────────┤
│ A src/widgets/panel.rs    │ Commit Message     │
│ M Cargo.toml              │ [ ............... ] │
├──────────────────────────┤ ☐ Amend            │
│ [Stage][Unstage][Rescan]  │           [Commit] │
└──────────────────────────┴────────────────────┘
```

## Architecture

The UI never touches `git2` directly — it goes through a small backend
abstraction, which keeps everything testable without a live repository.

| Module | Contents |
|--------|----------|
| `backend` | `RepoBackend` trait + data types (`CommitInfo`, `FileChange`, `Diff`/`DiffLine`, `RefLabel`, `WorkingStatus`). Browse reads history/diffs; commit mode adds working-tree status, per-file diffs, `stage`/`unstage`, `commit` (with amend). Implementations: `Git2Backend` (live, libgit2) and `FixtureBackend` (deterministic, in-memory, with a simulated working tree). |
| `widgets` | git-specific widgets — `CommitList` (graph + badges + columns), `DiffView` (colored diff), `SearchBar`, `Heading`, `graph` (DAG lane assignment); `Shell`, a generic flat-focus container, plus a `layout` module giving the browse and commit screens their rectangles; generic `Shared<W>` adapter. |
| `ui` | `GitClient`, the top-level widget. It owns both screens (a `Shell` each), switches between them, and — since saudade widgets are callback-free — polls selections and a small command queue after each event to rebuild dependent panes. |

## Testing

* **Pixel snapshots** (`tests/snapshots.rs`) render the real UI through
  saudade's offscreen `MockBackend` at 1.0/1.25/1.5/2.0× against the
  in-memory `FixtureBackend` and bundled DejaVu fonts, comparing PNG bytes
  with `insta`. Regenerate with `INSTA_UPDATE=always cargo test --test
  snapshots`, then review with `cargo insta review`.
* **`tests/git2_backend.rs`** builds a throwaway repository with fixed
  signatures/timestamps and reads it back through `Git2Backend`, so the live
  backend is covered deterministically.
* **`tests/commit_backend.rs`** exercises commit mode end to end: it stages,
  unstages, commits and amends in a throwaway repository (and against the
  fixture), asserting the working-tree status at each step.
* **Unit tests** cover the graph lane algorithm and date formatting.

```
cargo test          # everything
cargo build         # the binary
```
