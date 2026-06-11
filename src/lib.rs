//! journey — a gitk-style repository browser built on the saudade toolkit.
//!
//! The crate is split so the UI is testable without a live repository:
//!
//! * [`backend`] — the [`RepoBackend`](backend::RepoBackend) trait plus a live
//!   `git2` implementation and an in-memory fixture for snapshot tests;
//! * [`imagediff`] — decodes image blobs and composes them into a graphical
//!   comparison shown in place of a text diff for image files;
//! * [`widgets`] — git-specific widgets (diff view, commit list, …) layered on
//!   top of saudade's generic widget set;
//! * [`ui`] — the top-level [`GitClient`](ui::GitClient) widget that wires the
//!   panes together.

pub mod backend;
pub mod imagediff;
pub mod ui;
pub mod widgets;
