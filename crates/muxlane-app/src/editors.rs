//! Detect local code editors and open a project directory in them.
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Editor {
    VsCode,
    Zed,
}

impl Editor {
    pub(crate) const ALL: [Self; 2] = [Self::VsCode, Self::Zed];

    pub(crate) fn label_key(self) -> &'static str {
        match self {
            Self::VsCode => "menu.open_in_vscode",
            Self::Zed => "menu.open_in_zed",
        }
    }

    pub(crate) fn button_id(self) -> &'static str {
        match self {
            Self::VsCode => "tree-open-vscode",
            Self::Zed => "tree-open-zed",
        }
    }

    fn commands(self) -> &'static [&'static str] {
        match self {
            Self::VsCode => &["code", "code-insiders", "codium"],
            Self::Zed => &["zed", "zeditor", "zedit"],
        }
    }

    /// macOS bundle CLIs, used when the user has not installed the shell command.
    fn bundle_binaries(self) -> &'static [&'static str] {
        match self {
            Self::VsCode => &[
                "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code",
                "/Applications/Visual Studio Code - Insiders.app/Contents/Resources/app/bin/code",
            ],
            Self::Zed => &[
                "/Applications/Zed.app/Contents/MacOS/cli",
                "/Applications/Zed Preview.app/Contents/MacOS/cli",
            ],
        }
    }

    fn flatpak_ids(self) -> &'static [&'static str] {
        match self {
            Self::VsCode => &["com.visualstudio.code", "com.vscodium.codium"],
            Self::Zed => &["dev.zed.Zed"],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EditorLauncher {
    pub(crate) editor: Editor,
    program: PathBuf,
    args: Vec<String>,
}

impl EditorLauncher {
    pub(crate) fn command(&self, project: &Path) -> std::process::Command {
        let mut command = std::process::Command::new(&self.program);
        command
            .args(&self.args)
            .arg(project)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        command
    }
}

/// Find editors on this machine only; remote projects need the remote machine's tools.
pub(crate) fn detect_local_editors() -> Vec<EditorLauncher> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    detect_with(
        &std::env::var_os("PATH").unwrap_or_default(),
        home.as_deref(),
        &flatpak_apps(),
    )
}

fn flatpak_apps() -> Vec<String> {
    if !cfg!(target_os = "linux") {
        return Vec::new();
    }
    std::process::Command::new("flatpak")
        .args(["list", "--app", "--columns=application"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(|line| line.trim().to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn detect_with(
    path: &std::ffi::OsStr,
    home: Option<&Path>,
    flatpaks: &[String],
) -> Vec<EditorLauncher> {
    Editor::ALL
        .into_iter()
        .filter_map(|editor| {
            let mut candidates: Vec<PathBuf> = editor
                .commands()
                .iter()
                .flat_map(|command| std::env::split_paths(path).map(move |dir| dir.join(command)))
                .collect();
            for bundle in editor.bundle_binaries() {
                candidates.push(PathBuf::from(bundle));
                if let Some(home) = home {
                    candidates.push(home.join(bundle.trim_start_matches('/')));
                }
            }
            if let Some(program) = candidates
                .into_iter()
                .find(|candidate| is_executable(candidate))
            {
                return Some(EditorLauncher {
                    editor,
                    program,
                    args: Vec::new(),
                });
            }
            let app = editor
                .flatpak_ids()
                .iter()
                .find(|id| flatpaks.iter().any(|installed| installed == *id))?;
            Some(EditorLauncher {
                editor,
                program: "flatpak".into(),
                args: vec!["run".into(), (*app).into()],
            })
        })
        .collect()
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(test)]
pub(crate) fn detect_with_for_test(bin: &Path, home: &Path) -> Vec<EditorLauncher> {
    detect_with(&std::env::join_paths([bin]).unwrap(), Some(home), &[])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn executable(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn only_installed_editors_are_offered() {
        let directory = tempfile::tempdir().unwrap();
        let bin = directory.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("zed"), "not executable").unwrap();
        let path = std::env::join_paths([&bin]).unwrap();
        assert!(detect_with(&path, Some(directory.path()), &[]).is_empty());

        executable(&bin.join("code"));
        let found = detect_with(&path, Some(directory.path()), &[]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].editor, Editor::VsCode);
        let command = found[0].command(Path::new("/tmp/project"));
        assert_eq!(command.get_program(), bin.join("code"));
        assert_eq!(command.get_args().collect::<Vec<_>>(), ["/tmp/project"]);
    }

    #[test]
    fn macos_bundles_and_flatpaks_count_as_installed() {
        let directory = tempfile::tempdir().unwrap();
        let zed_cli = directory
            .path()
            .join("Applications/Zed.app/Contents/MacOS/cli");
        executable(&zed_cli);
        let found = detect_with(
            std::ffi::OsStr::new(""),
            Some(directory.path()),
            &["com.visualstudio.code".into()],
        );
        assert_eq!(
            found.iter().map(|item| item.editor).collect::<Vec<_>>(),
            [Editor::VsCode, Editor::Zed]
        );
        let vscode = found[0].command(Path::new("/tmp/p"));
        assert_eq!(vscode.get_program(), "flatpak");
        assert_eq!(
            vscode.get_args().collect::<Vec<_>>(),
            ["run", "com.visualstudio.code", "/tmp/p"]
        );
        assert_eq!(found[1].command(Path::new("/tmp/p")).get_program(), zed_cli);
    }
}
