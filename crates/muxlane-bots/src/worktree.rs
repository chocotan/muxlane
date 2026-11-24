//! Safe git worktree isolation per Bot execution with clean teardown.
use std::path::{Path, PathBuf};
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct WorktreeLease {
    pub workdir: PathBuf,
    pub repo_toplevel: Option<PathBuf>,
    pub isolated: bool,
}

pub async fn prepare_worktree(repo_root: &Path, bot_id: &str, run_id: &str) -> WorktreeLease {
    let Ok(toplevel) = git_toplevel(repo_root).await else {
        return WorktreeLease {
            workdir: repo_root.to_path_buf(),
            repo_toplevel: None,
            isolated: false,
        };
    };

    // Only create a worktree if HEAD exists (empty initialized repos cannot checkout HEAD).
    let has_head = Command::new("git")
        .arg("-C")
        .arg(&toplevel)
        .args(["rev-parse", "--verify", "HEAD"])
        .output()
        .await
        .is_ok_and(|o| o.status.success());
    if !has_head {
        return WorktreeLease {
            workdir: repo_root.to_path_buf(),
            repo_toplevel: Some(toplevel),
            isolated: false,
        };
    }

    let safe_bot: String = bot_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let wt_path = toplevel
        .join(".muxlane-worktrees")
        .join(safe_bot)
        .join(run_id);
    if let Some(parent) = wt_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    let ok = Command::new("git")
        .arg("-C")
        .arg(&toplevel)
        .arg("worktree")
        .arg("add")
        .arg("--detach")
        .arg(&wt_path)
        .arg("HEAD")
        .output()
        .await
        .is_ok_and(|o| o.status.success());

    if ok {
        WorktreeLease {
            workdir: wt_path,
            repo_toplevel: Some(toplevel),
            isolated: true,
        }
    } else {
        WorktreeLease {
            workdir: repo_root.to_path_buf(),
            repo_toplevel: Some(toplevel),
            isolated: false,
        }
    }
}

/// Removes the isolated worktree automatically when clean (no modified/untracked files),
/// preserving dirty worktrees so the user can inspect or commit Bot changes.
pub async fn release_worktree_if_clean(lease: &WorktreeLease) -> bool {
    if !lease.isolated {
        return false;
    }
    let Some(toplevel) = lease.repo_toplevel.as_ref() else {
        return false;
    };
    let dirty = Command::new("git")
        .arg("-C")
        .arg(&lease.workdir)
        .args(["status", "--porcelain"])
        .output()
        .await
        .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(true);
    if dirty {
        return false;
    }
    let removed = Command::new("git")
        .arg("-C")
        .arg(toplevel)
        .arg("worktree")
        .arg("remove")
        .arg("--force")
        .arg(&lease.workdir)
        .output()
        .await
        .is_ok_and(|o| o.status.success());
    if !removed {
        let _ = tokio::fs::remove_dir_all(&lease.workdir).await;
    }
    let _ = Command::new("git")
        .arg("-C")
        .arg(toplevel)
        .args(["worktree", "prune"])
        .output()
        .await;
    true
}

async fn git_toplevel(dir: &Path) -> anyhow::Result<PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .await?;
    if !out.status.success() {
        anyhow::bail!("not a git repo");
    }
    Ok(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn worktree_isolates_clean_run_and_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        assert!(std::process::Command::new("git")
            .args(["init"])
            .current_dir(repo)
            .status()
            .unwrap()
            .success());
        assert!(std::process::Command::new("git")
            .args(["config", "user.email", "test@muxlane.dev"])
            .current_dir(repo)
            .status()
            .unwrap()
            .success());
        assert!(std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(repo)
            .status()
            .unwrap()
            .success());
        std::fs::write(repo.join("README.md"), "hi\n").unwrap();
        assert!(std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(repo)
            .status()
            .unwrap()
            .success());
        assert!(std::process::Command::new("git")
            .args(["-c", "commit.gpgsign=false", "commit", "-m", "init"])
            .current_dir(repo)
            .status()
            .unwrap()
            .success());

        let lease = prepare_worktree(repo, "bot_a", "run_1").await;
        assert!(lease.isolated);
        assert!(lease.workdir.join("README.md").exists());
        assert!(release_worktree_if_clean(&lease).await);
        assert!(!lease.workdir.exists());
    }
}
