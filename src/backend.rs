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

        Ok(Backend {
            path,
            repo: repo.unwrap(),
        })
    }

    // look at https://github.com/rust-lang/git2-rs/blob/master/examples/log.rs
    pub fn log(&self) -> Result<Vec<Commit>, BackendError> {
        let mut revwalk = self.repo.revwalk()?;
        let mut commits = Vec::new();
        revwalk.push_head()?;
        for commit_id in revwalk {
            let commit_id = commit_id?;
            let commit = self.repo.find_commit(commit_id)?;
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
