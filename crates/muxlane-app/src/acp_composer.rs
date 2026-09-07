use muxlane_acp::AvailableCommand;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) const MAX_SKILL_BYTES: usize = 64 * 1024;
pub(crate) const MAX_CONTEXT_BYTES: u64 = 256 * 1024;
pub(crate) const MAX_CONTEXT_ITEMS: usize = 10_000;
pub(crate) const MAX_CONTEXT_COMPLETIONS: usize = 100;
pub(crate) const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
pub(crate) const MAX_AUDIO_BYTES: u64 = 20 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttachmentKind {
    Image,
    Audio,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Attachment {
    pub(crate) path: PathBuf,
    pub(crate) name: String,
    pub(crate) mime_type: String,
    pub(crate) size: u64,
    pub(crate) kind: AttachmentKind,
}

pub(crate) fn attachment_from_path(path: &Path) -> Result<Attachment, String> {
    let path = fs::canonicalize(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let metadata = fs::metadata(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("attachment is not a file: {}", path.display()));
    }
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let (mime_type, kind, limit) = match extension.as_str() {
        "png" => ("image/png", AttachmentKind::Image, MAX_IMAGE_BYTES),
        "jpg" | "jpeg" => ("image/jpeg", AttachmentKind::Image, MAX_IMAGE_BYTES),
        "webp" => ("image/webp", AttachmentKind::Image, MAX_IMAGE_BYTES),
        "gif" => ("image/gif", AttachmentKind::Image, MAX_IMAGE_BYTES),
        "wav" => ("audio/wav", AttachmentKind::Audio, MAX_AUDIO_BYTES),
        "mp3" => ("audio/mpeg", AttachmentKind::Audio, MAX_AUDIO_BYTES),
        "m4a" => ("audio/mp4", AttachmentKind::Audio, MAX_AUDIO_BYTES),
        "ogg" => ("audio/ogg", AttachmentKind::Audio, MAX_AUDIO_BYTES),
        _ => return Err(format!("unsupported attachment type: {}", path.display())),
    };
    if metadata.len() > limit {
        return Err(format!("attachment is too large: {}", path.display()));
    }
    Ok(Attachment {
        name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string()),
        path,
        mime_type: mime_type.into(),
        size: metadata.len(),
        kind,
    })
}

pub(crate) fn load_attachment(attachment: &Attachment) -> Result<muxlane_acp::PromptBlock, String> {
    use base64::Engine as _;
    let bytes = fs::read(&attachment.path)
        .map_err(|error| format!("{}: {error}", attachment.path.display()))?;
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(match attachment.kind {
        AttachmentKind::Image => muxlane_acp::PromptBlock::Image {
            data,
            mime_type: attachment.mime_type.clone(),
        },
        AttachmentKind::Audio => muxlane_acp::PromptBlock::Audio {
            data,
            mime_type: attachment.mime_type.clone(),
        },
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CompletionKind {
    Command,
    Skill,
    Context,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompletionItem {
    pub(crate) label: String,
    pub(crate) description: String,
    pub(crate) source: String,
    pub(crate) insert_text: String,
    pub(crate) kind: CompletionKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Skill {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) path: PathBuf,
    pub(crate) source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SkillDiscovery {
    pub(crate) skills: Vec<Skill>,
    pub(crate) errors: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ContextKind {
    File,
    Directory,
    Thread,
    Terminal,
    Diagnostic,
    Url,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContextItem {
    pub(crate) relative_path: String,
    pub(crate) search_key: String,
    pub(crate) path: PathBuf,
    pub(crate) kind: ContextKind,
    pub(crate) content: Option<String>,
}

impl ContextItem {
    pub(crate) fn new(
        relative_path: impl Into<String>,
        path: PathBuf,
        kind: ContextKind,
        content: Option<String>,
    ) -> Self {
        let relative_path = relative_path.into();
        let search_key = relative_path.to_ascii_lowercase();
        Self {
            relative_path,
            search_key,
            path,
            kind,
            content,
        }
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn within(root: &Path, candidate: &Path) -> bool {
    candidate == root || candidate.starts_with(root.join(""))
}

fn skill_roots(project_root: &Path) -> Vec<(PathBuf, String)> {
    let mut roots = vec![
        (
            project_root.join(".agents/skills"),
            "project .agents".to_string(),
        ),
        (project_root.join(".pi/skills"), "project .pi".to_string()),
    ];
    if let Some(home) = home_dir() {
        roots.push((home.join(".agents/skills"), "user .agents".to_string()));
        roots.push((home.join(".pi/agent/skills"), "user .pi".to_string()));
    }
    roots
}

fn parse_frontmatter(text: &str) -> Option<(String, String)> {
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut name = None;
    let mut description = String::new();
    for line in lines {
        let line = line.trim();
        if line == "---" {
            let name = name?;
            return Some((name, description));
        }
        let (key, value) = line.split_once(':')?;
        let value = value.trim().trim_matches(['"', '\'']);
        match key.trim() {
            "name" if !value.is_empty() => name = Some(value.to_string()),
            "description" => description = value.to_string(),
            _ => {}
        }
    }
    None
}

pub(crate) fn load_skill(skill: &Skill) -> Result<String, String> {
    let canonical = fs::canonicalize(&skill.path)
        .map_err(|error| format!("{}: {error}", skill.path.display()))?;
    let root = canonical
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| format!("invalid skill path: {}", skill.path.display()))?;
    let canonical_root =
        fs::canonicalize(root).map_err(|error| format!("{}: {error}", root.display()))?;
    if !within(&canonical_root, &canonical)
        || canonical.file_name().and_then(|name| name.to_str()) != Some("SKILL.md")
    {
        return Err(format!(
            "skill path escapes its skill directory: {}",
            skill.path.display()
        ));
    }
    let metadata =
        fs::metadata(&canonical).map_err(|error| format!("{}: {error}", canonical.display()))?;
    if metadata.len() as usize > MAX_SKILL_BYTES {
        return Err(format!(
            "skill exceeds {} KiB: {}",
            MAX_SKILL_BYTES / 1024,
            canonical.display()
        ));
    }
    fs::read_to_string(&canonical).map_err(|error| format!("{}: {error}", canonical.display()))
}

pub(crate) fn discover_skills(project_root: &Path) -> SkillDiscovery {
    let mut skills = Vec::new();
    let mut errors = Vec::new();
    for (root, source) in skill_roots(project_root) {
        let Ok(root_canonical) = fs::canonicalize(&root) else {
            continue;
        };
        let Ok(entries) = fs::read_dir(&root_canonical) else {
            errors.push(format!("cannot read skill directory: {}", root.display()));
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path().join("SKILL.md");
            let Ok(canonical) = fs::canonicalize(&path) else {
                continue;
            };
            if !within(&root_canonical, &canonical) {
                errors.push(format!("rejected skill outside root: {}", path.display()));
                continue;
            }
            let Ok(metadata) = fs::metadata(&canonical) else {
                errors.push(format!("cannot read skill: {}", canonical.display()));
                continue;
            };
            if metadata.len() as usize > MAX_SKILL_BYTES {
                errors.push(format!("skill exceeds 64 KiB: {}", canonical.display()));
                continue;
            }
            let Ok(text) = fs::read_to_string(&canonical) else {
                errors.push(format!("cannot read skill: {}", canonical.display()));
                continue;
            };
            let Some((name, description)) = parse_frontmatter(&text) else {
                continue;
            };
            skills.push(Skill {
                name,
                description,
                path: canonical,
                source: source.clone(),
            });
        }
    }
    skills.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then(left.source.cmp(&right.source))
    });
    SkillDiscovery { skills, errors }
}

pub(crate) fn merged_completions(
    commands: &[AvailableCommand],
    skills: &[Skill],
    query: &str,
) -> Vec<CompletionItem> {
    let query = query.to_ascii_lowercase();
    let skill_only = query.strip_prefix(':');
    let command_query = skill_only.unwrap_or(&query);
    let mut result = Vec::new();
    if skill_only.is_none() {
        for command in commands {
            if !command.name.to_ascii_lowercase().contains(command_query) {
                continue;
            }
            result.push(CompletionItem {
                label: format!("/{}", command.name),
                description: command.description.clone(),
                source: "ACP".into(),
                insert_text: format!("/{} ", command.name),
                kind: CompletionKind::Command,
            });
        }
    }
    let skill_query = skill_only.unwrap_or(&query);
    for skill in skills {
        if !skill.name.to_ascii_lowercase().contains(skill_query) {
            continue;
        }
        result.push(CompletionItem {
            label: format!("/:{}", skill.name),
            description: skill.description.clone(),
            source: skill.source.clone(),
            insert_text: format!("/:{} ", skill.name),
            kind: CompletionKind::Skill,
        });
    }
    result
}

pub(crate) fn context_items(project_root: &Path) -> Result<Vec<ContextItem>, String> {
    let root = fs::canonicalize(project_root)
        .map_err(|error| format!("{}: {error}", project_root.display()))?;
    let mut result = Vec::new();
    visit_context(&root, &root, &mut result)?;
    if result.len() < MAX_CONTEXT_ITEMS
        && (root.join("Cargo.toml").is_file() || root.join("tsconfig.json").is_file())
    {
        result.push(ContextItem::new(
            "diagnostics:project",
            root.clone(),
            ContextKind::Diagnostic,
            None,
        ));
    }
    result.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(result)
}

fn visit_context(
    root: &Path,
    directory: &Path,
    result: &mut Vec<ContextItem>,
) -> Result<bool, String> {
    if result.len() >= MAX_CONTEXT_ITEMS {
        return Ok(true);
    }
    let entries =
        fs::read_dir(directory).map_err(|error| format!("{}: {error}", directory.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("{}: {error}", directory.display()))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if matches!(
            name.as_str(),
            ".git" | "target" | "node_modules" | "dist" | "artifacts" | ".cache"
        ) {
            continue;
        }
        let file_type = entry
            .file_type()
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if file_type.is_symlink() {
            continue;
        }
        let canonical =
            fs::canonicalize(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        if !within(root, &canonical) {
            continue;
        }
        if result.len() >= MAX_CONTEXT_ITEMS {
            return Ok(true);
        }
        let relative = canonical
            .strip_prefix(root)
            .map_err(|_| "context path escaped project".to_string())?;
        let relative_path = relative
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        let is_directory = file_type.is_dir();
        result.push(ContextItem::new(
            relative_path,
            canonical.clone(),
            if is_directory {
                ContextKind::Directory
            } else {
                ContextKind::File
            },
            None,
        ));
        if is_directory && visit_context(root, &canonical, result)? {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn filter_context(items: &[ContextItem], query: &str) -> Vec<CompletionItem> {
    let raw_query = query.trim();
    let query = raw_query.to_ascii_lowercase();
    let mut result = Vec::with_capacity(MAX_CONTEXT_COMPLETIONS.min(items.len()));
    for item in items {
        if !item.search_key.contains(&query) {
            continue;
        }
        result.push(CompletionItem {
            label: format!("@{}", item.relative_path),
            description: match item.kind {
                ContextKind::File => "file",
                ContextKind::Directory => "directory",
                ContextKind::Thread => "thread",
                ContextKind::Terminal => "terminal",
                ContextKind::Diagnostic => "diagnostics",
                ContextKind::Url => "URL",
            }
            .into(),
            source: "project".into(),
            insert_text: format!("@{} ", item.relative_path),
            kind: CompletionKind::Context,
        });
        if result.len() == MAX_CONTEXT_COMPLETIONS {
            break;
        }
    }
    if valid_context_url(raw_query) {
        if result.len() == MAX_CONTEXT_COMPLETIONS {
            result.pop();
        }
        result.insert(
            0,
            CompletionItem {
                label: format!("@{raw_query}"),
                description: "URL".into(),
                source: "web".into(),
                insert_text: format!("@{raw_query} "),
                kind: CompletionKind::Context,
            },
        );
    }
    result
}

fn valid_context_url(value: &str) -> bool {
    url::Url::parse(value)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
}

pub(crate) fn load_project_diagnostics(root: &Path) -> String {
    let command = if root.join("Cargo.toml").is_file() {
        Some(("cargo", vec!["check", "--message-format=short"]))
    } else if root.join("tsconfig.json").is_file() {
        Some(("npx", vec!["tsc", "--noEmit", "--pretty", "false"]))
    } else {
        None
    };
    let Some((program, args)) = command else {
        return "No diagnostics provider is available for this project.".into();
    };
    match std::process::Command::new(program)
        .args(args)
        .current_dir(root)
        .output()
    {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            if text.len() > 128 * 1024 {
                text.truncate(128 * 1024);
                text.push_str("\n[diagnostics truncated]");
            }
            if text.trim().is_empty() {
                "No diagnostics.".into()
            } else {
                text
            }
        }
        Err(error) => format!("Failed to run diagnostics: {error}"),
    }
}

pub(crate) fn replace_active_token(text: &str, insert_text: &str) -> String {
    let Some((start, character)) = text
        .char_indices()
        .rev()
        .find(|(_, character)| character.is_whitespace())
    else {
        return insert_text.to_string();
    };
    format!("{}{}", &text[..start + character.len_utf8()], insert_text)
}
pub(crate) fn active_token(text: &str) -> Option<(char, String)> {
    if text.chars().next_back().is_some_and(char::is_whitespace) {
        return None;
    }
    let token = text.split_whitespace().last().unwrap_or_default();
    let trigger = token.chars().next()?;
    if trigger == '/' || trigger == '@' {
        Some((trigger, token[1..].to_string()))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_token_preserves_complete_whitespace_characters() {
        for (text, expected) in [
            ("ask /old", "ask /new"),
            ("ask　/old", "ask　/new"),
            ("ask\u{a0}/old", "ask\u{a0}/new"),
            ("ask\n/old", "ask\n/new"),
            ("你好 /old", "你好 /new"),
            ("/old", "/new"),
            ("", "/new"),
        ] {
            assert_eq!(replace_active_token(text, "/new"), expected);
        }
    }

    #[test]
    fn frontmatter_reads_name_and_description_without_yaml() {
        assert_eq!(
            parse_frontmatter("---\nname: review\ndescription: Check code\n---\nbody"),
            Some(("review".into(), "Check code".into()))
        );
    }

    #[test]
    fn active_token_ignores_completed_tokens() {
        assert_eq!(active_token("/:review "), None);
        assert_eq!(active_token("@src/main.rs\n"), None);
        assert_eq!(active_token("ask /:rev"), Some(('/', ":rev".into())));
    }

    #[test]
    fn commands_win_name_precedence_and_filtering() {
        let commands = vec![AvailableCommand {
            name: "review".into(),
            description: "ACP".into(),
            input_hint: None,
        }];
        let skills = vec![
            Skill {
                name: "review".into(),
                description: "skill".into(),
                path: PathBuf::new(),
                source: "project".into(),
            },
            Skill {
                name: "run".into(),
                description: "skill".into(),
                path: PathBuf::new(),
                source: "project".into(),
            },
        ];
        let result = merged_completions(&commands, &skills, "rev");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].label, "/review");
        assert_eq!(result[0].source, "ACP");
        assert_eq!(result[1].label, "/:review");

        let skills_only = merged_completions(&commands, &skills, ":run");
        assert_eq!(skills_only.len(), 1);
        assert_eq!(skills_only[0].label, "/:run");
    }

    #[test]
    fn discovery_rejects_escaped_symlinks_and_oversized_skills() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("project");
        let skills_root = root.join(".agents/skills");
        let valid = skills_root.join("valid");
        std::fs::create_dir_all(&valid).unwrap();
        std::fs::write(
            valid.join("SKILL.md"),
            "---\nname: valid\ndescription: Valid skill\n---\nbody",
        )
        .unwrap();
        let oversized = skills_root.join("oversized");
        std::fs::create_dir_all(&oversized).unwrap();
        std::fs::write(oversized.join("SKILL.md"), vec![b'x'; MAX_SKILL_BYTES + 1]).unwrap();

        #[cfg(unix)]
        {
            let outside = directory.path().join("outside");
            std::fs::create_dir_all(&outside).unwrap();
            std::fs::write(
                outside.join("SKILL.md"),
                "---\nname: escaped\ndescription: Escaped\n---\nbody",
            )
            .unwrap();
            std::os::unix::fs::symlink(&outside, skills_root.join("escaped")).unwrap();
        }

        let discovery = discover_skills(&root);
        assert!(discovery.skills.iter().any(|skill| skill.name == "valid"));
        assert!(!discovery
            .skills
            .iter()
            .any(|skill| matches!(skill.name.as_str(), "escaped" | "oversized")));
        assert!(!discovery.errors.is_empty());
    }

    #[test]
    fn context_discovers_files_without_reading_symbols_and_preserves_url_case() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("main.rs");
        std::fs::write(&source, "pub struct AgentThread;\nfn run() {}\n").unwrap();
        let items = context_items(directory.path()).unwrap();
        assert!(items
            .iter()
            .any(|item| { item.kind == ContextKind::File && item.relative_path == "main.rs" }));
        assert!(items
            .iter()
            .all(|item| { item.search_key == item.relative_path.to_ascii_lowercase() }));

        let url = filter_context(&items, "https://Example.com/Path");
        assert_eq!(url[0].insert_text, "@https://Example.com/Path ");
    }

    #[test]
    fn context_skips_generated_directories() {
        let directory = tempfile::tempdir().unwrap();
        for name in [
            ".git",
            "target",
            "node_modules",
            "dist",
            "artifacts",
            ".cache",
        ] {
            let generated = directory.path().join(name);
            std::fs::create_dir_all(&generated).unwrap();
            std::fs::write(generated.join("generated.txt"), b"ignored").unwrap();
        }
        std::fs::write(directory.path().join("source.txt"), b"kept").unwrap();

        let items = context_items(directory.path()).unwrap();
        assert!(items.iter().any(|item| item.relative_path == "source.txt"));
        assert!(!items.iter().any(|item| {
            item.relative_path.split('/').any(|part| {
                matches!(
                    part,
                    ".git" | "target" | "node_modules" | "dist" | "artifacts" | ".cache"
                )
            })
        }));
    }

    #[test]
    fn context_items_stop_at_the_result_cap() {
        let directory = tempfile::tempdir().unwrap();
        for index in 0..=MAX_CONTEXT_ITEMS {
            std::fs::write(directory.path().join(format!("file-{index}.txt")), b"file").unwrap();
        }

        let items = context_items(directory.path()).unwrap();
        assert_eq!(items.len(), MAX_CONTEXT_ITEMS);
    }

    #[test]
    fn ordinary_token_does_not_activate_context_completion() {
        assert_eq!(active_token("plain text"), None);
    }

    #[test]
    fn context_filter_is_capped() {
        let items: Vec<_> = (0..150)
            .map(|index| {
                ContextItem::new(
                    format!("src/file-{index}.rs"),
                    PathBuf::new(),
                    ContextKind::File,
                    None,
                )
            })
            .collect();

        let completions = filter_context(&items, "file");
        assert_eq!(completions.len(), MAX_CONTEXT_COMPLETIONS);
    }

    #[test]
    fn attachments_are_typed_and_size_limited() {
        let directory = tempfile::tempdir().unwrap();
        let image = directory.path().join("screen.png");
        std::fs::write(&image, b"png").unwrap();
        let attachment = attachment_from_path(&image).unwrap();
        assert_eq!(attachment.kind, AttachmentKind::Image);
        assert!(matches!(
            load_attachment(&attachment).unwrap(),
            muxlane_acp::PromptBlock::Image { mime_type, .. } if mime_type == "image/png"
        ));

        let unsupported = directory.path().join("notes.txt");
        std::fs::write(&unsupported, b"text").unwrap();
        assert!(attachment_from_path(&unsupported).is_err());
    }

    #[test]
    fn context_filter_inserts_relative_path() {
        let items = vec![ContextItem::new(
            "src/main.rs",
            PathBuf::new(),
            ContextKind::File,
            None,
        )];
        assert_eq!(
            filter_context(&items, "main")[0].insert_text,
            "@src/main.rs "
        );
    }
}
