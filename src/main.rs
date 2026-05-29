//! journey — a gitk-style repository browser.
//!
//! Usage: `journey [PATH]`. With no argument it opens the repository
//! containing the current working directory; otherwise it discovers the
//! repository at (or above) PATH.

use std::process::ExitCode;
use std::rc::Rc;

use journey::backend::{Git2Backend, RepoBackend};
use journey::ui::GitClient;
use retrogui::{App, Theme, WindowConfig};

const WINDOW_W: i32 = 900;
const WINDOW_H: i32 = 640;

fn main() -> ExitCode {
    let path = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());

    let backend: Rc<dyn RepoBackend> = match Git2Backend::open(&path) {
        Ok(backend) => Rc::new(backend),
        Err(err) => {
            eprintln!("journey: cannot open a git repository at {path:?}: {}", err.message());
            return ExitCode::FAILURE;
        }
    };

    let title = format!("Journey — {}", backend.path());
    let root = GitClient::new(backend);

    App::new(
        WindowConfig::new(title, WINDOW_W, WINDOW_H).resizable(true),
        root,
    )
    .with_theme(Theme::windows_31())
    .run();

    ExitCode::SUCCESS
}
