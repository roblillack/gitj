//! gitj — a gitk-style git repository browser and commit helper.
//!
//! Usage: `gitj [OPTIONS] [PATH]`. With no PATH it opens the repository
//! containing the current working directory; otherwise it discovers the
//! repository at (or above) PATH. `-c`/`--commit` starts on the staging
//! screen; `--version` and `--help` print and exit.

use std::process::ExitCode;
use std::rc::Rc;

use journey::backend::{Git2Backend, RepoBackend};
use journey::ui::GitClient;
use saudade::{App, Theme, WindowConfig};

const WINDOW_W: i32 = 900;
const WINDOW_H: i32 = 640;
/// Floor on the resizable window, so the panes never collapse past the point
/// where the narrow (third-width) commit/browse layout still works.
const MIN_WINDOW_W: i32 = 450;
const MIN_WINDOW_H: i32 = 320;

const USAGE: &str = "\
Usage: gitj [OPTIONS] [PATH]

A gitk-style git repository browser and commit helper.

Arguments:
  [PATH]  Path to (or inside) the repository to open [default: .]

Options:
  -c, --commit   Open the commit (staging) screen instead of the history browser
  -V, --version  Print version information and exit
  -h, --help     Print this help and exit";

/// What the parsed command line asks gitj to do.
#[derive(Debug, PartialEq, Eq)]
enum Cli {
    /// Launch the GUI on `path`, starting on the commit screen when `commit`.
    Run { path: String, commit: bool },
    /// Print `text` to stdout and exit successfully (`--version`, `--help`).
    Print(String),
    /// Print `text` to stderr and exit with failure (bad usage).
    Usage(String),
}

/// Parse gitj's arguments (the iterator should already exclude argv[0]).
///
/// Accepts at most one positional PATH plus the `-c`/`--commit`,
/// `-V`/`--version` and `-h`/`--help` flags. A bare `--` forces everything
/// after it to be treated as the positional PATH (so paths that start with `-`
/// stay reachable).
fn parse_args(args: impl IntoIterator<Item = String>) -> Cli {
    let mut path: Option<String> = None;
    let mut commit = false;
    let mut positional_only = false;

    for arg in args {
        if !positional_only {
            match arg.as_str() {
                "--" => {
                    positional_only = true;
                    continue;
                }
                "-h" | "--help" => return Cli::Print(USAGE.to_string()),
                "-V" | "--version" => {
                    return Cli::Print(format!("gitj {}", env!("CARGO_PKG_VERSION")));
                }
                "-c" | "--commit" => {
                    commit = true;
                    continue;
                }
                // Anything else starting with '-' (but not a lone "-") is an
                // unrecognized flag.
                s if s.starts_with('-') && s != "-" => {
                    return Cli::Usage(format!("gitj: unknown option {arg:?}\n\n{USAGE}"));
                }
                _ => {}
            }
        }

        if path.is_some() {
            return Cli::Usage(format!(
                "gitj: unexpected extra argument {arg:?}\n\n{USAGE}"
            ));
        }
        path = Some(arg);
    }

    Cli::Run {
        path: path.unwrap_or_else(|| ".".to_string()),
        commit,
    }
}

fn main() -> ExitCode {
    let (path, commit) = match parse_args(std::env::args().skip(1)) {
        Cli::Run { path, commit } => (path, commit),
        Cli::Print(text) => {
            println!("{text}");
            return ExitCode::SUCCESS;
        }
        Cli::Usage(text) => {
            eprintln!("{text}");
            return ExitCode::FAILURE;
        }
    };

    let backend: Rc<dyn RepoBackend> = match Git2Backend::open(&path) {
        Ok(backend) => Rc::new(backend),
        Err(err) => {
            eprintln!(
                "gitj: cannot open a git repository at {path:?}: {}",
                err.message()
            );
            return ExitCode::FAILURE;
        }
    };

    let title = format!("Git Journey — {}", backend.path());
    // File ▸ Reload re-discovers the repository at the same path.
    let reload_path = path.clone();
    let mut root = GitClient::new(backend).with_reopen(Box::new(move || {
        Git2Backend::open(&reload_path)
            .ok()
            .map(|b| Rc::new(b) as Rc<dyn RepoBackend>)
    }));
    // `gitj -c` opens straight onto the staging screen; otherwise the browser.
    if commit {
        root.enter_commit_mode();
    }

    App::new(
        WindowConfig::new(title, WINDOW_W, WINDOW_H)
            .resizable(true)
            .min_size(MIN_WINDOW_W, MIN_WINDOW_H),
        root,
    )
    .with_theme(Theme::windows_31())
    .run();

    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::{Cli, parse_args};

    fn parse(args: &[&str]) -> Cli {
        parse_args(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn no_args_opens_the_current_directory_in_browse_mode() {
        assert_eq!(
            parse(&[]),
            Cli::Run {
                path: ".".to_string(),
                commit: false,
            }
        );
    }

    #[test]
    fn a_bare_path_is_the_repository_to_open() {
        assert_eq!(
            parse(&["/src/repo"]),
            Cli::Run {
                path: "/src/repo".to_string(),
                commit: false,
            }
        );
    }

    #[test]
    fn commit_flag_opens_the_staging_screen() {
        for flag in ["-c", "--commit"] {
            assert_eq!(
                parse(&[flag]),
                Cli::Run {
                    path: ".".to_string(),
                    commit: true,
                }
            );
        }
    }

    #[test]
    fn commit_flag_and_path_combine_in_either_order() {
        let expected = Cli::Run {
            path: "/src/repo".to_string(),
            commit: true,
        };
        assert_eq!(parse(&["-c", "/src/repo"]), expected);
        assert_eq!(parse(&["/src/repo", "--commit"]), expected);
    }

    #[test]
    fn version_prints_the_crate_version() {
        for flag in ["-V", "--version"] {
            match parse(&[flag]) {
                Cli::Print(text) => {
                    assert_eq!(text, format!("gitj {}", env!("CARGO_PKG_VERSION")));
                }
                other => panic!("expected Print, got {other:?}"),
            }
        }
    }

    #[test]
    fn help_prints_usage() {
        for flag in ["-h", "--help"] {
            match parse(&[flag]) {
                Cli::Print(text) => assert!(text.contains("Usage: gitj")),
                other => panic!("expected Print, got {other:?}"),
            }
        }
    }

    #[test]
    fn unknown_option_is_a_usage_error() {
        match parse(&["--nope"]) {
            Cli::Usage(text) => {
                assert!(text.contains("unknown option"));
                assert!(text.contains("Usage: gitj"));
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn a_second_positional_argument_is_a_usage_error() {
        match parse(&["/one", "/two"]) {
            Cli::Usage(text) => assert!(text.contains("unexpected extra argument")),
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn double_dash_lets_a_path_start_with_a_dash() {
        assert_eq!(
            parse(&["--", "-weird-path"]),
            Cli::Run {
                path: "-weird-path".to_string(),
                commit: false,
            }
        );
    }
}
