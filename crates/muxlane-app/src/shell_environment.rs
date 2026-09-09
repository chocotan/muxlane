//! Recover the PATH configured by a macOS user's terminal shell before starting workers.
use anyhow::{bail, Context as _};
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStringExt;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;

const START: &[u8] = b"\0MUXLANE_PATH_START\0";
const END: &[u8] = b"\0MUXLANE_PATH_END\0";
const PATH_COMMAND: &str =
    "printf '\\0MUXLANE_PATH_START\\0'; /usr/bin/printenv PATH; printf '\\0MUXLANE_PATH_END\\0'";
const OUTPUT_LIMIT: u64 = 64 * 1024;

/// Call only during single-threaded startup. Do not mutate the process environment
/// from a GPUI callback or after the server's multi-threaded runtime has started.
#[cfg(target_os = "macos")]
pub(crate) fn import_login_path(shell: &str) -> anyhow::Result<()> {
    let inherited = std::env::var_os("PATH");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let shell_path = runtime.block_on(read_login_path(shell, Duration::from_secs(5)));
    drop(runtime);
    let shell_path = match shell_path {
        Ok(path) => path,
        Err(error) => {
            tracing::warn!(%error, "could not load login shell PATH; using inherited PATH and nvm fallback");
            inherited.clone().unwrap_or_default()
        }
    };
    let nvm_dir = std::env::var_os("NVM_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".nvm"))
        });
    let path = augment_path(&shell_path, inherited.as_deref(), nvm_dir.as_deref())?;
    std::env::set_var("PATH", path);
    Ok(())
}

async fn read_login_path(shell: &str, timeout: Duration) -> anyhow::Result<OsString> {
    // Interactive + login loads .zprofile and .zshrc (including Node version managers).
    // Only PATH is captured; shell startup output is separated by explicit markers.
    let mut child = tokio::process::Command::new(shell)
        .args(["-ilc", PATH_COMMAND])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("start login shell for PATH")?;
    let stdout = child.stdout.take().context("capture login shell PATH")?;
    let result = tokio::time::timeout(timeout, async {
        let mut output = Vec::new();
        stdout
            .take(OUTPUT_LIMIT + 1)
            .read_to_end(&mut output)
            .await?;
        if output.len() as u64 > OUTPUT_LIMIT {
            bail!("login shell output exceeded the PATH capture limit");
        }
        let status = child.wait().await?;
        if !status.success() {
            bail!("login shell exited with {status}");
        }
        parse_path(&output)
    })
    .await;
    match result {
        Ok(Ok(path)) => Ok(path),
        result => {
            let _ = child.kill().await;
            match result {
                Ok(Err(error)) => Err(error),
                Err(_) => bail!("login shell PATH lookup timed out"),
                Ok(Ok(_)) => unreachable!(),
            }
        }
    }
}

fn parse_path(output: &[u8]) -> anyhow::Result<OsString> {
    let start = output
        .windows(START.len())
        .position(|window| window == START)
        .context("login shell did not return a PATH marker")?
        + START.len();
    let rest = &output[start..];
    let end = rest
        .windows(END.len())
        .position(|window| window == END)
        .context("login shell did not finish returning PATH")?;
    let path = rest[..end]
        .strip_suffix(b"\n")
        .context("login shell returned an invalid PATH")?;
    if path.is_empty() || path.contains(&0) {
        bail!("login shell returned an empty or invalid PATH");
    }
    Ok(OsString::from_vec(path.to_vec()))
}

fn merge_paths(shell: &OsStr, inherited: Option<&OsStr>) -> anyhow::Result<OsString> {
    let mut paths = Vec::new();
    for value in std::iter::once(shell).chain(inherited) {
        for path in std::env::split_paths(value) {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    std::env::join_paths(paths).context("merge shell and inherited PATH")
}

fn augment_path(
    shell: &OsStr,
    inherited: Option<&OsStr>,
    nvm_dir: Option<&std::path::Path>,
) -> anyhow::Result<OsString> {
    let merged = merge_paths(shell, inherited)?;
    let mut paths: Vec<_> = std::env::split_paths(&merged).collect();
    if let Some(entries) = nvm_dir.and_then(|dir| std::fs::read_dir(dir.join("versions/node")).ok())
    {
        let mut versions: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
        // Keep the active shell's Node first. Other versions only fill missing tools.
        versions.sort_by_cached_key(|path| {
            std::cmp::Reverse(
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .trim_start_matches('v')
                    .split('.')
                    .map(|part| part.parse::<u64>().unwrap_or(0))
                    .collect::<Vec<_>>(),
            )
        });
        for bin in versions.into_iter().map(|version| version.join("bin")) {
            if bin.is_dir() && !paths.contains(&bin) {
                paths.push(bin);
            }
        }
    }
    std::env::join_paths(paths).context("append nvm tool directories to PATH")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn path_capture_ignores_shell_greetings_and_preserves_spaces() {
        let output = b"welcome\n\0MUXLANE_PATH_START\0/Users/test/Library/pnpm:/Users/test/my tools/bin:/usr/bin\n\0MUXLANE_PATH_END\0goodbye\n";
        assert_eq!(
            parse_path(output).unwrap(),
            "/Users/test/Library/pnpm:/Users/test/my tools/bin:/usr/bin"
        );
        for output in [
            b"no marker".as_slice(),
            b"\0MUXLANE_PATH_START\0",
            b"\0MUXLANE_PATH_START\0\n\0MUXLANE_PATH_END\0",
        ] {
            assert!(parse_path(output).is_err());
        }
    }

    #[test]
    fn shell_path_has_priority_without_losing_inherited_tools() {
        assert_eq!(
            merge_paths(
                OsStr::new("/Users/test/.nvm/bin:/opt/homebrew/bin:/usr/bin"),
                Some(OsStr::new("/usr/bin:/bin:/custom/bin"))
            )
            .unwrap(),
            "/Users/test/.nvm/bin:/opt/homebrew/bin:/usr/bin:/bin:/custom/bin"
        );
    }

    #[test]
    fn nvm_fallback_finds_pi_and_its_node_without_replacing_active_paths() {
        let directory = tempfile::tempdir().unwrap();
        let nvm = directory.path().join("custom nvm");
        let bin = nvm.join("versions/node/v22.10.0/bin");
        let older = nvm.join("versions/node/v22.9.0/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&older).unwrap();
        for (name, content) in [
            ("pi", "#!/usr/bin/env node\n"),
            ("node", "#!/bin/sh\nprintf 'pi runtime ready'\n"),
        ] {
            let path = bin.join(name);
            std::fs::write(&path, content).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let inherited = OsStr::new("/usr/bin:/bin");
        let path = augment_path(inherited, Some(inherited), Some(&nvm)).unwrap();
        assert_eq!(
            std::env::split_paths(&path).collect::<Vec<_>>(),
            vec![
                std::path::PathBuf::from("/usr/bin"),
                "/bin".into(),
                bin.clone(),
                older
            ]
        );
        // An npm executable's /usr/bin/env node shebang must work with the same PATH.
        let empty = directory.path().join("empty-bin");
        std::fs::create_dir(&empty).unwrap();
        let fallback_path = augment_path(empty.as_os_str(), None, Some(&nvm)).unwrap();
        let output = std::process::Command::new("pi")
            .env("PATH", fallback_path)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"pi runtime ready");
        let active = std::env::join_paths([bin.clone(), "/usr/bin".into()]).unwrap();
        let path = augment_path(&active, Some(inherited), Some(&nvm)).unwrap();
        let paths: Vec<_> = std::env::split_paths(&path).collect();
        assert_eq!(paths[0], bin);
        assert_eq!(paths.iter().filter(|path| **path == bin).count(), 1);
    }

    fn fake_shell(directory: &std::path::Path, body: &str) -> std::path::PathBuf {
        let path = directory.join("shell");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[tokio::test]
    async fn reads_login_shell_path_and_bounds_slow_startup() {
        let directory = tempfile::tempdir().unwrap();
        let shell = fake_shell(directory.path(), "[ \"$1\" = '-ilc' ] || exit 2\nprintf 'greeting\\n'\nPATH='/custom/node/bin:/usr/bin:/bin'\nexport PATH\neval \"$2\"");
        let path = read_login_path(shell.to_str().unwrap(), Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(path, "/custom/node/bin:/usr/bin:/bin");

        let shell = fake_shell(directory.path(), "exec /bin/sleep 5");
        let error = read_login_path(shell.to_str().unwrap(), Duration::from_millis(50))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("timed out"));
    }
}
