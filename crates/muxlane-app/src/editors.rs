//! Detect local code editors and open a project directory in them.
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Editor {
    VsCode,
    Zed,
    Idea,
}

impl Editor {
    pub(crate) const ALL: [Self; 3] = [Self::VsCode, Self::Zed, Self::Idea];

    pub(crate) fn label_key(self) -> &'static str {
        match self {
            Self::VsCode => "menu.open_in_vscode",
            Self::Zed => "menu.open_in_zed",
            Self::Idea => "menu.open_in_idea",
        }
    }

    pub(crate) fn button_id(self) -> &'static str {
        match self {
            Self::VsCode => "tree-open-vscode",
            Self::Zed => "tree-open-zed",
            Self::Idea => "tree-open-idea",
        }
    }

    fn commands(self) -> &'static [&'static str] {
        match self {
            Self::VsCode => &["code", "code-insiders", "codium"],
            Self::Idea => &[
                "idea",
                "idea.sh",
                "intellij-idea",
                "intellij-idea-ultimate",
                "intellij-idea-community",
                "idea-ultimate",
                "idea-community",
            ],
            // Linux packages use `zeditor`; prefer it over a potentially stale
            // user-installed `zed` (or the unrelated ZFS daemon of that name).
            #[cfg(target_os = "linux")]
            Self::Zed => &["zeditor", "zed", "zedit"],
            #[cfg(not(target_os = "linux"))]
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
            Self::Idea => &[
                "/Applications/IntelliJ IDEA.app/Contents/MacOS/idea",
                "/Applications/IntelliJ IDEA CE.app/Contents/MacOS/idea",
                "/Applications/IntelliJ IDEA Ultimate.app/Contents/MacOS/idea",
            ],
        }
    }

    /// Toolbox scripts may not be on PATH when launched from the desktop.
    fn home_binaries(self) -> &'static [&'static str] {
        match self {
            Self::Idea => &[
                ".local/share/JetBrains/Toolbox/scripts/idea",
                "Library/Application Support/JetBrains/Toolbox/scripts/idea",
            ],
            _ => &[],
        }
    }

    fn flatpak_ids(self) -> &'static [&'static str] {
        match self {
            Self::VsCode => &["com.visualstudio.code", "com.vscodium.codium"],
            Self::Zed => &["dev.zed.Zed"],
            Self::Idea => &[
                "com.jetbrains.IntelliJ-IDEA-Ultimate",
                "com.jetbrains.IntelliJ-IDEA-Community",
            ],
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
            .stderr(std::process::Stdio::piped());
        command
    }

    /// Run off the UI thread: creating a process does not mean its CLI succeeded.
    /// Waiting also reaps the child instead of leaving a zombie after each click.
    pub(crate) fn open(&self, project: &Path) -> Result<(), String> {
        let output = self
            .command(project)
            .output()
            .map_err(|error| format!("{}: {error}", self.program.display()))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "{} ({}): {}",
            self.program.display(),
            output.status,
            stderr.trim()
        ))
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
            // Prefer Toolbox's generated launcher: it forwards project arguments
            // and follows IDE upgrades, unlike older hand-written PATH wrappers.
            let mut candidates: Vec<PathBuf> = home
                .into_iter()
                .flat_map(|home| editor.home_binaries().iter().map(move |path| home.join(path)))
                .collect();
            candidates.extend(editor.commands().iter().flat_map(|command| {
                std::env::split_paths(path).map(move |dir| dir.join(command))
            }));
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
    #[cfg(target_os = "linux")]
    fn linux_package_launcher_takes_precedence_over_user_zed() {
        let directory = tempfile::tempdir().unwrap();
        let user_bin = directory.path().join("user-bin");
        let system_bin = directory.path().join("system-bin");
        executable(&user_bin.join("zed"));
        executable(&system_bin.join("zeditor"));
        let path = std::env::join_paths([&user_bin, &system_bin]).unwrap();
        let found = detect_with(&path, Some(directory.path()), &[]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].program, system_bin.join("zeditor"));

        std::fs::remove_file(system_bin.join("zeditor")).unwrap();
        let found = detect_with(&path, Some(directory.path()), &[]);
        assert_eq!(found[0].program, user_bin.join("zed"));
    }

    #[test]
    fn launch_reports_cli_exit_errors_and_missing_executables() {
        let directory = tempfile::tempdir().unwrap();
        let program = directory.path().join("zed");
        executable(&program);
        std::fs::write(&program, "#!/bin/sh\necho 'cannot open display' >&2\nexit 7\n")
            .unwrap();
        let launcher = EditorLauncher {
            editor: Editor::Zed,
            program: program.clone(),
            args: Vec::new(),
        };
        let error = launcher.open(directory.path()).unwrap_err();
        assert!(error.contains("cannot open display"), "{error}");
        assert!(error.contains("exit status: 7"), "{error}");
        assert!(error.contains(program.to_str().unwrap()), "{error}");

        std::fs::write(&program, "#!/bin/sh\nexit 0\n").unwrap();
        assert!(launcher.open(directory.path()).is_ok());
        std::fs::remove_file(&program).unwrap();
        assert!(launcher.open(directory.path()).is_err());
    }

    #[test]
    fn idea_toolbox_bundle_and_flatpak_launchers() {
        let directory = tempfile::tempdir().unwrap();
        let toolbox = directory.path().join(Editor::Idea.home_binaries()[0]);
        executable(&toolbox);
        let bin = directory.path().join("bin");
        executable(&bin.join("idea"));
        let path = std::env::join_paths([&bin]).unwrap();
        let found = detect_with(&path, Some(directory.path()), &[]);
        let idea = found.iter().find(|item| item.editor == Editor::Idea).unwrap();
        assert_eq!(idea.program, toolbox);
        assert_eq!(
            idea.command(Path::new("/tmp/project with spaces"))
                .get_args()
                .collect::<Vec<_>>(),
            ["/tmp/project with spaces"]
        );
        std::fs::remove_file(toolbox).unwrap();

        let bundle = directory
            .path()
            .join("Applications/IntelliJ IDEA.app/Contents/MacOS/idea");
        executable(&bundle);
        let found = detect_with(std::ffi::OsStr::new(""), Some(directory.path()), &[]);
        let idea = found.iter().find(|item| item.editor == Editor::Idea).unwrap();
        assert_eq!(idea.program, bundle);
        std::fs::remove_file(bundle).unwrap();

        for id in Editor::Idea.flatpak_ids() {
            let found = detect_with(
                std::ffi::OsStr::new(""),
                Some(directory.path()),
                &[(*id).into()],
            );
            let idea = found.iter().find(|item| item.editor == Editor::Idea).unwrap();
            let command = idea.command(Path::new("/tmp/project with spaces"));
            assert_eq!(command.get_program(), "flatpak");
            assert_eq!(
                command.get_args().collect::<Vec<_>>(),
                ["run", *id, "/tmp/project with spaces"]
            );
        }
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
