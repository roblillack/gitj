//! Git-specific widgets layered on top of retrogui's generic set.

mod commit_list;
mod diff_view;
mod graph;
mod heading;
pub mod layout;
mod search_bar;
mod shared;
mod shell;

pub use commit_list::{CommitList, CommitRow};
pub use diff_view::DiffView;
pub use graph::{compute_graph, Graph, GraphRow};
pub use heading::Heading;
pub use search_bar::SearchBar;
pub use shared::Shared;
pub use shell::Shell;
