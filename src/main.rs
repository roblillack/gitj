//! gitj — a gitk-style git repository browser and commit helper.
//!
//! Usage: `gitj [OPTIONS] [PATH]`. With no PATH it opens the repository
//! containing the current working directory; otherwise it discovers the
//! repository at (or above) PATH. `-c`/`--commit` starts on the staging
//! screen; `-r`/`--review [<branch>]` starts on the branch-review screen,
//! optionally pre-selecting a branch; `--version` and `--help` print and exit.

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
  -c, --commit            Open the commit (staging) screen instead of the history browser
  -r, --review [<branch>] Open the branch-review screen, selecting <branch> (or the
                          checked-out branch when none is given)
  -V, --version           Print version information and exit
  -h, --help              Print this help and exit";

/// Which screen gitj opens on. The screens are mutually exclusive, so this is
/// an enum rather than independent flags.
#[derive(Debug, PartialEq, Eq, Default)]
enum StartMode {
    /// The gitk-style history browser (the default).
    #[default]
    Browse,
    /// The `git gui`-style commit (staging) screen (`-c`/`--commit`).
    Commit,
    /// The branch-review screen (`-r`/`--review`), pre-selecting `branch` when
    /// one was given, otherwise the checked-out branch.
    Review { branch: Option<String> },
}

/// What the parsed command line asks gitj to do.
#[derive(Debug, PartialEq, Eq)]
enum Cli {
    /// Launch the GUI on `path`, opening on the `mode` screen.
    Run { path: String, mode: StartMode },
    /// Print `text` to stdout and exit successfully (`--version`, `--help`).
    Print(String),
    /// Print `text` to stderr and exit with failure (bad usage).
    Usage(String),
}

/// Parse gitj's arguments (the iterator should already exclude argv[0]).
///
/// Accepts at most one positional PATH plus the `-c`/`--commit`,
/// `-r`/`--review`, `-V`/`--version` and `-h`/`--help` flags. `--review` takes
/// an optional branch: the token right after `-r`/`--review` is read as the
/// branch unless it is another option or there is none (`--review=<branch>`
/// names it unambiguously). `-c` and `-r` are mutually exclusive. A bare `--`
/// forces everything after it to be treated as the positional PATH (so paths
/// that start with `-` stay reachable).
fn parse_args(args: impl IntoIterator<Item = String>) -> Cli {
    let mut path: Option<String> = None;
    let mut commit = false;
    let mut review = false;
    let mut review_branch: Option<String> = None;
    let mut positional_only = false;
    // Set right after `-r`/`--review`: the next token is its optional branch.
    let mut want_review_branch = false;

    for arg in args {
        // The token following a bare `-r`/`--review` is its branch, as long as
        // it isn't another option (or `--`) and we're not already past `--`.
        // Anything flag-like falls through to normal handling, leaving the
        // review on the checked-out branch.
        if want_review_branch {
            want_review_branch = false;
            let flag_like = arg.starts_with('-') && arg != "-";
            if !positional_only && !flag_like {
                review_branch = Some(arg);
                continue;
            }
        }

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
                "-r" | "--review" => {
                    review = true;
                    want_review_branch = true;
                    continue;
                }
                // `--review=<branch>` names the branch inline — the only way to
                // pass one that starts with '-'.
                s if s.starts_with("--review=") => {
                    review = true;
                    review_branch = Some(s["--review=".len()..].to_string());
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

    if commit && review {
        return Cli::Usage(format!(
            "gitj: --commit and --review cannot be combined\n\n{USAGE}"
        ));
    }

    let mode = if commit {
        StartMode::Commit
    } else if review {
        StartMode::Review {
            branch: review_branch,
        }
    } else {
        StartMode::Browse
    };

    Cli::Run {
        path: path.unwrap_or_else(|| ".".to_string()),
        mode,
    }
}

fn main() -> ExitCode {
    let (path, mode) = match parse_args(std::env::args().skip(1)) {
        Cli::Run { path, mode } => (path, mode),
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
    // `gitj -c` opens straight onto the staging screen, `gitj -r` onto the
    // branch reviewer; otherwise the history browser.
    match mode {
        StartMode::Browse => {}
        StartMode::Commit => root.enter_commit_mode(),
        StartMode::Review { branch } => {
            if !root.enter_review_mode(branch.as_deref())
                && let Some(branch) = branch
            {
                // The branch was named but matched no row; the reviewer still
                // opens (on the checked-out branch) so the list is reachable.
                eprintln!("gitj: no branch matching {branch:?}; opening the checked-out branch");
            }
        }
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
    use super::{Cli, StartMode, parse_args};

    fn parse(args: &[&str]) -> Cli {
        parse_args(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn no_args_opens_the_current_directory_in_browse_mode() {
        assert_eq!(
            parse(&[]),
            Cli::Run {
                path: ".".to_string(),
                mode: StartMode::Browse,
            }
        );
    }

    #[test]
    fn a_bare_path_is_the_repository_to_open() {
        assert_eq!(
            parse(&["/src/repo"]),
            Cli::Run {
                path: "/src/repo".to_string(),
                mode: StartMode::Browse,
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
                    mode: StartMode::Commit,
                }
            );
        }
    }

    #[test]
    fn commit_flag_and_path_combine_in_either_order() {
        let expected = Cli::Run {
            path: "/src/repo".to_string(),
            mode: StartMode::Commit,
        };
        assert_eq!(parse(&["-c", "/src/repo"]), expected);
        assert_eq!(parse(&["/src/repo", "--commit"]), expected);
    }

    #[test]
    fn review_flag_without_a_branch_opens_the_checked_out_branch() {
        for flag in ["-r", "--review"] {
            assert_eq!(
                parse(&[flag]),
                Cli::Run {
                    path: ".".to_string(),
                    mode: StartMode::Review { branch: None },
                }
            );
        }
    }

    #[test]
    fn review_flag_takes_the_following_token_as_the_branch() {
        for flag in ["-r", "--review"] {
            assert_eq!(
                parse(&[flag, "feature/x"]),
                Cli::Run {
                    path: ".".to_string(),
                    mode: StartMode::Review {
                        branch: Some("feature/x".to_string()),
                    },
                }
            );
        }
    }

    #[test]
    fn review_branch_and_path_combine() {
        assert_eq!(
            parse(&["--review", "feature/x", "/src/repo"]),
            Cli::Run {
                path: "/src/repo".to_string(),
                mode: StartMode::Review {
                    branch: Some("feature/x".to_string()),
                },
            }
        );
        // A path before the flag still leaves the review on the current branch.
        assert_eq!(
            parse(&["/src/repo", "-r"]),
            Cli::Run {
                path: "/src/repo".to_string(),
                mode: StartMode::Review { branch: None },
            }
        );
    }

    #[test]
    fn review_equals_form_names_the_branch_inline() {
        assert_eq!(
            parse(&["--review=feature/x"]),
            Cli::Run {
                path: ".".to_string(),
                mode: StartMode::Review {
                    branch: Some("feature/x".to_string()),
                },
            }
        );
    }

    #[test]
    fn review_does_not_swallow_a_following_flag_as_its_branch() {
        // `-r` before another option keeps the review on the current branch;
        // here the trailing path is what `-c -r` would conflict over, so check
        // the standalone case: `-r` then a path-less terminator.
        assert_eq!(
            parse(&["-r", "--", "/weird/-path"]),
            Cli::Run {
                path: "/weird/-path".to_string(),
                mode: StartMode::Review { branch: None },
            }
        );
    }

    #[test]
    fn commit_and_review_together_is_a_usage_error() {
        match parse(&["-c", "-r"]) {
            Cli::Usage(text) => {
                assert!(text.contains("cannot be combined"));
                assert!(text.contains("Usage: gitj"));
            }
            other => panic!("expected Usage, got {other:?}"),
        }
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
                mode: StartMode::Browse,
            }
        );
    }
}
