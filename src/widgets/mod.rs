//! Git-specific widgets layered on top of saudade's generic set.

mod commit_list;
mod diff_view;
mod graph;
mod heading;
pub mod layout;
mod search_bar;
mod shared;
mod shell;

pub use commit_list::{CommitList, CommitRow};
pub use diff_view::{DiffMode, DiffView};
pub use graph::{Graph, GraphRow, compute_graph};
pub use heading::Heading;
pub use search_bar::SearchBar;
pub use shared::Shared;
pub use shell::Shell;
