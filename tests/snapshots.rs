//! Pixel-snapshot tests for journey's UI, rendered against the deterministic
//! [`FixtureBackend`] so they never depend on a real repository.

mod common;

use std::rc::Rc;

use common::{snapshot_at_all_scales, snapshot_at_all_scales_with_events};
use journey::backend::FixtureBackend;
use journey::ui::GitClient;
use retrogui::{Event, Key, Modifiers, NamedKey, Widget};

const W: i32 = 760;
const H: i32 = 520;

fn sample_client() -> GitClient {
    GitClient::new(Rc::new(FixtureBackend::sample()))
}

fn key(k: NamedKey) -> Event {
    Event::KeyDown {
        key: Key::Named(k),
        modifiers: Modifiers::default(),
    }
}

/// The default main screen: commit list focused on the newest commit, its
/// single changed file shown in the bottom pane.
#[test]
fn main_screen() {
    snapshot_at_all_scales("main_screen", W, H, || {
        let mut client = sample_client();
        client.focus_first();
        Box::new(client)
    });
}

/// Arrow-down once selects the second commit (two changed files), exercising
/// the commit-selection → file-list sync.
#[test]
fn main_screen_files_synced() {
    snapshot_at_all_scales_with_events(
        "main_screen_files_synced",
        W,
        H,
        || Box::new(sample_client()),
        || vec![key(NamedKey::Down)],
    );
}
