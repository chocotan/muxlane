use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub(crate) struct ProjectCheckpoint {
    root: PathBuf,
    worktree_ref: String,
    index_ref: String,
    untracked: BTreeMap<PathBuf, Vec<u8>>,
    omitted_untracked: BTreeSet<PathBuf>,
}

pub(crate) fn capture_checkpoint(root: &Path) -> Result<Option<ProjectCheckpoint>, String> {
    let root =
        std::fs::canonicalize(root).map_err(|error| format!("{}: {error}", root.display()))?;
    if !root.join(".git").exists() {
        return Ok(None);
    }
    let head = match git_text(&root, &["rev-parse", "--verify", "HEAD"]) {
        Ok(head) => head,
        Err(_) => return Ok(None),
    };
    let stash = git_text(&root, &["stash", "create"])?;
    let worktree_ref = if stash.is_empty() {
        head.clone()
    } else {
        stash
    };
    let index_ref = if worktree_ref == head {
        head
    } else {
        git_text(&root, &["rev-parse", &format!("{worktree_ref}^2")])?
    };
    let mut untracked = BTreeMap::new();
    let mut omitted_untracked = BTreeSet::new();
    for path in git_paths(&root, &["ls-files", "--others", "--exclude-standard", "-z"])? {
        let absolute = safe_join(&root, &path)?;
        let metadata = std::fs::symlink_metadata(&absolute)
            .map_err(|error| format!("{}: {error}", absolute.display()))?;
        if metadata.file_type().is_symlink() {
            omitted_untracked.insert(path);
        } else if metadata.is_file() {
            let bytes = std::fs::read(&absolute)
                .map_err(|error| format!("{}: {error}", absolute.display()))?;
            if bytes.len() <= 2 * 1024 * 1024 {
                untracked.insert(path, bytes);
            } else {
                omitted_untracked.insert(path);
            }
        }
    }
    Ok(Some(ProjectCheckpoint {
        root,
        worktree_ref,
        index_ref,
        untracked,
        omitted_untracked,
    }))
}

pub(crate) fn restore_checkpoint(
    checkpoint: &ProjectCheckpoint,
) -> Result<Option<ProjectCheckpoint>, String> {
    let undo = capture_checkpoint(&checkpoint.root)?;
    let preserved_untracked = undo
        .as_ref()
        .map(|checkpoint| checkpoint.omitted_untracked.clone())
        .unwrap_or_default();
    match apply_checkpoint(checkpoint, &preserved_untracked) {
        Ok(()) => Ok(undo),
        Err(error) => {
            let rollback = undo
                .as_ref()
                .map(|checkpoint| apply_checkpoint(checkpoint, &BTreeSet::new()))
                .transpose()
                .map(|_| "current files restored".to_string())
                .unwrap_or_else(|rollback_error| format!("rollback failed: {rollback_error}"));
            Err(format!("{error}; {rollback}"))
        }
    }
}

fn apply_checkpoint(
    checkpoint: &ProjectCheckpoint,
    preserved_untracked: &BTreeSet<PathBuf>,
) -> Result<(), String> {
    let mut changed: BTreeSet<PathBuf> = git_paths(
        &checkpoint.root,
        &["diff", "--name-only", "-z", &checkpoint.worktree_ref, "--"],
    )?
    .into_iter()
    .collect();
    changed.extend(git_paths(
        &checkpoint.root,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )?);

    for path in changed {
        let absolute = safe_join(&checkpoint.root, &path)?;
        if let Some(bytes) = checkpoint.untracked.get(&path) {
            write_atomic(&absolute, bytes)?;
            continue;
        }
        let spec = git_object_spec(&checkpoint.worktree_ref, &path);
        let output = Command::new("git")
            .arg("show")
            .arg(spec)
            .current_dir(&checkpoint.root)
            .output()
            .map_err(|error| format!("git show: {error}"))?;
        if output.status.success() {
            write_atomic(&absolute, &output.stdout)?;
        } else if checkpoint.omitted_untracked.contains(&path)
            || preserved_untracked.contains(&path)
        {
            continue;
        } else if absolute.exists() {
            move_to_trash(&checkpoint.root, &absolute, &path)?;
        }
    }
    for (path, bytes) in &checkpoint.untracked {
        write_atomic(&safe_join(&checkpoint.root, path)?, bytes)?;
    }
    git_status(&checkpoint.root, &["read-tree", &checkpoint.index_ref])?;
    Ok(())
}

fn move_to_trash(root: &Path, absolute: &Path, relative: &Path) -> Result<(), String> {
    let git_path = git_text(
        root,
        &["rev-parse", "--git-path", "muxlane-checkpoint-trash"],
    )?;
    let git_path = PathBuf::from(git_path);
    let trash_root = if git_path.is_absolute() {
        git_path
    } else {
        root.join(git_path)
    };
    let trash = trash_root
        .join(muxlane_core::model::new_id("restore"))
        .join(relative);
    if let Some(parent) = trash.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    std::fs::rename(absolute, &trash).map_err(|error| {
        format!(
            "move {} to checkpoint trash from {}: {error}",
            absolute.display(),
            root.display()
        )
    })
}

fn git_object_spec(reference: &str, path: &Path) -> std::ffi::OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
        let mut bytes = reference.as_bytes().to_vec();
        bytes.push(b':');
        bytes.extend_from_slice(path.as_os_str().as_bytes());
        std::ffi::OsString::from_vec(bytes)
    }
    #[cfg(not(unix))]
    {
        format!("{reference}:{}", path.to_string_lossy()).into()
    }
}

fn git_text(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|error| format!("git {}: {error}", args.join(" ")))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git_status(root: &Path, args: &[&str]) -> Result<(), String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|error| format!("git {}: {error}", args.join(" ")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn git_paths(root: &Path, args: &[&str]) -> Result<Vec<PathBuf>, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|error| format!("git {}: {error}", args.join(" ")))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(path_from_git_bytes)
        .collect())
}

#[cfg(unix)]
fn path_from_git_bytes(path: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt as _;
    PathBuf::from(std::ffi::OsString::from_vec(path.to_vec()))
}

#[cfg(not(unix))]
fn path_from_git_bytes(path: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(path).into_owned())
}

fn safe_join(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err("checkpoint path escapes project root".into());
    }
    Ok(root.join(relative))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    let temporary = path.with_extension(format!(
        "muxlane-checkpoint-{}.tmp",
        muxlane_core::model::new_id("restore")
    ));
    std::fs::write(&temporary, bytes)
        .map_err(|error| format!("{}: {error}", temporary.display()))?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("{}: {error}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(root)
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn checkpoint_skips_repositories_with_unborn_head() {
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "-q"]);
        assert!(capture_checkpoint(directory.path()).unwrap().is_none());
    }

    #[test]
    fn checkpoint_restores_tracked_staged_and_untracked_files() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-q"]);
        git(root, &["config", "user.email", "test@example.com"]);
        git(root, &["config", "user.name", "Test"]);
        std::fs::write(root.join("tracked.txt"), "base").unwrap();
        git(root, &["add", "tracked.txt"]);
        git(root, &["commit", "-qm", "base"]);
        std::fs::write(root.join("tracked.txt"), "before").unwrap();
        std::fs::write(root.join("untracked.txt"), "before untracked").unwrap();
        let checkpoint = capture_checkpoint(root).unwrap().unwrap();

        std::fs::write(root.join("tracked.txt"), "agent").unwrap();
        std::fs::write(root.join("untracked.txt"), "agent untracked").unwrap();
        std::fs::write(root.join("created.txt"), "created").unwrap();
        let undo = restore_checkpoint(&checkpoint).unwrap().unwrap();

        assert_eq!(
            std::fs::read_to_string(root.join("tracked.txt")).unwrap(),
            "before"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("untracked.txt")).unwrap(),
            "before untracked"
        );
        assert!(!root.join("created.txt").exists());

        let _redo = restore_checkpoint(&undo).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("tracked.txt")).unwrap(),
            "agent"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("untracked.txt")).unwrap(),
            "agent untracked"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("created.txt")).unwrap(),
            "created"
        );
    }

    #[test]
    fn restore_preserves_untracked_files_too_large_for_undo() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-q"]);
        git(root, &["config", "user.email", "test@example.com"]);
        git(root, &["config", "user.name", "Test"]);
        std::fs::write(root.join("tracked.txt"), "base").unwrap();
        git(root, &["add", "tracked.txt"]);
        git(root, &["commit", "-qm", "base"]);
        let checkpoint = capture_checkpoint(root).unwrap().unwrap();
        let large = vec![b'x'; 2 * 1024 * 1024 + 1];
        std::fs::write(root.join("large.bin"), &large).unwrap();

        restore_checkpoint(&checkpoint).unwrap();

        assert_eq!(std::fs::read(root.join("large.bin")).unwrap(), large);
    }

    #[cfg(unix)]
    #[test]
    fn restore_does_not_replace_untracked_symlinks_with_files() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-q"]);
        git(root, &["config", "user.email", "test@example.com"]);
        git(root, &["config", "user.name", "Test"]);
        std::fs::write(root.join("tracked.txt"), "base").unwrap();
        git(root, &["add", "tracked.txt"]);
        git(root, &["commit", "-qm", "base"]);
        symlink("tracked.txt", root.join("link.txt")).unwrap();
        let checkpoint = capture_checkpoint(root).unwrap().unwrap();

        restore_checkpoint(&checkpoint).unwrap();

        assert!(std::fs::symlink_metadata(root.join("link.txt"))
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn restore_preserves_non_utf8_untracked_paths() {
        use std::os::unix::ffi::OsStringExt as _;

        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-q"]);
        git(root, &["config", "user.email", "test@example.com"]);
        git(root, &["config", "user.name", "Test"]);
        std::fs::write(root.join("tracked.txt"), "base").unwrap();
        git(root, &["add", "tracked.txt"]);
        git(root, &["commit", "-qm", "base"]);
        let name = std::ffi::OsString::from_vec(vec![b'n', 0xff]);
        let path = root.join(name);
        std::fs::write(&path, "before").unwrap();
        let checkpoint = capture_checkpoint(root).unwrap().unwrap();
        std::fs::write(&path, "after").unwrap();

        restore_checkpoint(&checkpoint).unwrap();

        assert_eq!(std::fs::read_to_string(path).unwrap(), "before");
    }
}
