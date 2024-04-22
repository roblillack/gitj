use std::fmt::Debug;

use git2::Repository;

#[derive(Debug, Clone)]
pub struct BackendError {
    pub message: String,
}

impl From<git2::Error> for BackendError {
    fn from(error: git2::Error) -> Self {
        BackendError {
            message: format!("Git error: {}", error.message()),
        }
    }
}

pub struct Backend {
    pub path: String,
    pub repo: Repository,
    pub messages: Vec<String>,
}

impl Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backend").field("path", &self.path).finish()
    }
}

pub struct Commit {
    pub id: String,
    pub author: String,
    pub message: String,
}

impl Backend {
    pub fn new(path: String) -> Result<Backend, BackendError> {
        let repo = Repository::open(&path);
        if let Err(error) = repo {
            eprintln!("Failed to open repository: {}", error.message());
            return Err(BackendError {
                message: format!("Failed to open repository: {}", error.message()),
            });
        }
        let repo = repo.unwrap();

        Ok(Backend {
            path,
            messages: Vec::from_iter(Self::log(&repo).unwrap().iter().map(|x| x.message.clone())),
            repo,
        })
    }

    pub fn changed_files(&self, idx: usize) -> Vec<String> {
        let log = Self::log(&self.repo);
        match log {
            Err(error) => {
                eprintln!("Failed to get log: {}", error.message);
                return vec!["Failed to get log".to_string()];
            }
            Ok(commits) => match commits.get(idx) {
                None => vec!["Commit not found".to_string()],
                Some(commit) => {
                    let real = self.repo.find_commit(commit.id.parse().unwrap()).unwrap();
                    let diff = self.repo.diff_tree_to_tree(
                        Some(&real.tree().unwrap()),
                        Some(&real.parents().next().unwrap().tree().unwrap()),
                        None,
                    );
                    let mut files = Vec::new();
                    // println!("Diff: {:?}", diff);
                    diff.unwrap()
                        .print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
                            println!("Line: {:?}", line);
                            files.push(line.origin().to_string());
                            true
                        })
                        .unwrap();
                    files
                }
            },
        }
    }

    // look at https://github.com/rust-lang/git2-rs/blob/master/examples/log.rs
    pub fn log(repo: &Repository) -> Result<Vec<Commit>, BackendError> {
        let mut revwalk = repo.revwalk()?;
        let mut commits = Vec::new();
        revwalk.push_head()?;
        for commit_id in revwalk {
            let commit_id = commit_id?;
            let commit = repo.find_commit(commit_id)?;
            println!("commit: {}", commit.id());
            println!("author: {}", commit.author());
            println!("message: {}", commit.message().unwrap_or_default());
            commits.push(Commit {
                id: commit.id().to_string(),
                author: commit.author().name().unwrap_or_default().to_string(),
                message: commit.message().unwrap_or_default().to_string(),
            });
        }
        Ok(commits)
    }
}
