use std::{
    collections::{BTreeMap, HashSet},
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

use agent_client_protocol::{AcpAgent, AcpAgentConfig};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};

use crate::{
    catalog::{canonical_id, documentation_url, Catalog, LOCAL_RECIPES},
    Profile,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Found(PathBuf),
    Missing,
    Unmapped,
    Unsupported,
    Invalid(String),
}

#[derive(Debug, Clone)]
pub struct AgentEntry {
    pub id: String,
    pub label: String,
    pub profile: Option<Profile>,
    pub availability: Availability,
    pub install: Option<&'static str>,
    pub docs: Option<String>,
    pub user_configured: bool,
}

impl AgentEntry {
    pub fn can_start(&self) -> bool {
        self.profile.is_some() && matches!(self.availability, Availability::Found(_))
    }
}

/// A fresh environment snapshot per detection, with no shell or process probes.
#[derive(Debug, Clone)]
pub struct LocalProbe {
    path: OsString,
}

impl LocalProbe {
    pub fn current() -> Self {
        Self::new(std::env::var_os("PATH").unwrap_or_default())
    }

    pub fn new(path: impl Into<OsString>) -> Self {
        Self { path: path.into() }
    }

    pub fn command(&self, command: &str) -> Availability {
        if !cfg!(unix) {
            return Availability::Unsupported;
        }
        let command = Path::new(command);
        if command.is_absolute() {
            return if executable(command) {
                Availability::Found(command.into())
            } else {
                Availability::Missing
            };
        }
        if command.components().count() != 1
            || command.as_os_str().is_empty()
            || command == Path::new(".")
            || command == Path::new("..")
        {
            return Availability::Invalid("ACP command must be a basename or absolute path".into());
        }
        for directory in
            std::env::split_paths(&self.path).filter(|directory| directory.is_absolute())
        {
            let candidate = directory.join(command);
            if executable(&candidate) {
                return Availability::Found(candidate);
            }
        }
        Availability::Missing
    }

    pub fn definition(&self, definition: &AgentDefinition) -> Availability {
        if let Err(error) = definition.validate() {
            return Availability::Invalid(error.to_string());
        }
        match definition.env.get("PATH") {
            Some(path) => Self::new(OsStr::new(path)).command(&definition.command),
            None => self.command(&definition.command),
        }
    }
}

fn executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

/// Recheck immediately before launch and pin the executable to its absolute PATH entry.
/// User command arguments/environment remain explicitly trusted, including download launchers.
pub fn local_profile(profile: &Profile, probe: &LocalProbe) -> Result<Profile> {
    let mut definition = crate::effective_definition(profile)?;
    match probe.definition(&definition) {
        Availability::Found(path) => {
            definition.command = path.to_str().context("non-UTF8 executable path")?.into();
            Ok(Profile::Custom(definition))
        }
        status => bail!("ACP agent {} is not locally available ({status:?}); install or configure it, then detect again", profile.id()),
    }
}

/// Trusted user-level process configuration, never discovered in a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentDefinition {
    pub id: String,
    pub label: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl AgentDefinition {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.id.is_empty()
                && self.id.trim() == self.id
                && !self.id.chars().any(char::is_control),
            "agent id must be nonempty without surrounding whitespace or control characters"
        );
        ensure!(
            !self.label.trim().is_empty(),
            "agent {:?}: label must not be empty",
            self.id
        );
        ensure!(
            !self.command.trim().is_empty(),
            "agent {:?}: command must not be empty",
            self.id
        );
        ensure!(
            !self.command.contains('\0') && self.args.iter().all(|arg| !arg.contains('\0')),
            "agent {:?}: command and args must not contain NUL",
            self.id
        );
        ensure!(
            self.env.iter().all(|(key, value)| !key.is_empty()
                && !key.contains(['=', '\0'])
                && !value.contains('\0')),
            "agent {:?}: invalid environment variable",
            self.id
        );
        Ok(())
    }

    pub(crate) fn agent(&self) -> Result<AcpAgent> {
        self.validate()?;
        Ok(AcpAgent::new(
            AcpAgentConfig::new(&self.command)
                .args(self.args.clone())
                .envs(self.env.clone()),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct AgentRegistry {
    profiles: Vec<Profile>,
    custom_ids: HashSet<String>,
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self {
            profiles: LOCAL_RECIPES
                .iter()
                .map(|recipe| {
                    Profile::from_id(recipe.id)
                        .unwrap_or_else(|| Profile::Custom(recipe.definition()))
                })
                .collect(),
            custom_ids: HashSet::new(),
        }
    }
}

impl AgentRegistry {
    pub fn from_definitions(definitions: Vec<AgentDefinition>) -> Result<Self> {
        let mut registry = Self::default();
        let mut ids = HashSet::new();
        for mut definition in definitions {
            definition.validate()?;
            definition.id = canonical_id(&definition.id).into();
            ensure!(
                ids.insert(definition.id.clone()),
                "duplicate agent id {:?}",
                definition.id
            );
            registry.custom_ids.insert(definition.id.clone());
            if let Some(existing) = registry
                .profiles
                .iter_mut()
                .find(|profile| profile.id() == definition.id)
            {
                *existing = Profile::Custom(definition);
            } else {
                registry.profiles.push(Profile::Custom(definition));
            }
        }
        Ok(registry)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let json = match std::fs::read_to_string(path) {
            Ok(json) => json,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Config {
            agents: Vec<AgentDefinition>,
        }
        let config: Config =
            serde_json::from_str(&json).with_context(|| format!("parse {}", path.display()))?;
        Self::from_definitions(config.agents).with_context(|| format!("invalid {}", path.display()))
    }

    /// Stable local recipe order, then additional user definitions in file order.
    pub fn profiles(&self) -> &[Profile] {
        &self.profiles
    }

    /// Restoration must never fall back to the creation default.
    pub fn resolve(&self, id: &str) -> Option<Profile> {
        self.profiles
            .iter()
            .find(|profile| profile.id() == canonical_id(id))
            .cloned()
    }

    pub fn entries(&self, catalog: Option<&Catalog>, probe: &LocalProbe) -> Vec<AgentEntry> {
        let mut entries: Vec<_> = self
            .profiles
            .iter()
            .map(|profile| {
                let definition = crate::effective_definition(profile);
                let user_configured = self.custom_ids.contains(profile.id())
                    || definition
                        .as_ref()
                        .is_ok_and(|definition| *definition != profile.definition());
                let recipe = LOCAL_RECIPES
                    .iter()
                    .find(|recipe| recipe.id == profile.id());
                AgentEntry {
                    id: profile.id().into(),
                    label: profile.label().into(),
                    profile: Some(profile.clone()),
                    availability: match definition {
                        Ok(definition) => probe.definition(&definition),
                        Err(error) => Availability::Invalid(format!("{error:#}")),
                    },
                    install: recipe
                        .filter(|_| !user_configured)
                        .map(|recipe| recipe.install),
                    docs: recipe.map(|recipe| recipe.docs.into()),
                    user_configured,
                }
            })
            .collect();
        if let Some(catalog) = catalog {
            for agent in &catalog.agents {
                let id = canonical_id(&agent.id);
                if entries.iter().any(|entry| entry.id == id) {
                    continue;
                }
                entries.push(AgentEntry {
                    id: id.into(),
                    label: agent.name.clone(),
                    profile: None,
                    availability: Availability::Unmapped,
                    install: None,
                    docs: agent
                        .website
                        .as_deref()
                        .and_then(documentation_url)
                        .or_else(|| agent.repository.as_deref().and_then(documentation_url)),
                    user_configured: false,
                });
            }
        }
        entries
    }

    pub fn require(&self, id: &str) -> Result<Profile> {
        match self.resolve(id) {
            Some(profile) => Ok(profile),
            None => bail!("ACP agent {id:?} is unavailable; check your user-level agents.json"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom(id: &str) -> AgentDefinition {
        AgentDefinition {
            id: id.into(),
            label: "My Agent".into(),
            command: "/path with spaces/adapter".into(),
            args: vec!["two words".into(), "$HOME; $(touch nope)".into(), "".into()],
            env: [("MODE".into(), "literal $HOME ; value".into())].into(),
        }
    }

    #[test]
    fn validates_definitions_and_rejects_duplicate_ids() {
        let definition = custom("my-agent");
        definition.validate().unwrap();
        for field in ["id", "label", "command"] {
            let mut json = serde_json::to_value(&definition).unwrap();
            json[field] = " ".into();
            let invalid: AgentDefinition = serde_json::from_value(json).unwrap();
            assert!(invalid.validate().is_err(), "{field}");
        }
        let mut invalid = definition.clone();
        invalid.args.push("bad\0arg".into());
        assert!(invalid.validate().is_err());
        for key in ["", "BAD=KEY", "BAD\0KEY"] {
            let mut invalid = definition.clone();
            invalid.env.insert(key.into(), "value".into());
            assert!(invalid.validate().is_err());
        }
        assert!(
            AgentRegistry::from_definitions(vec![definition.clone(), definition])
                .unwrap_err()
                .to_string()
                .contains("duplicate agent id")
        );
        for id in ["claude", "codex", "pi"] {
            assert_eq!(
                AgentRegistry::from_definitions(vec![custom(id)])
                    .unwrap()
                    .require(id)
                    .unwrap(),
                Profile::Custom(custom(id))
            );
        }
    }

    #[test]
    fn creation_order_is_fixed_and_unknown_restoration_never_uses_default() {
        let registry =
            AgentRegistry::from_definitions(vec![custom("z-last"), custom("a-first")]).unwrap();
        assert_eq!(
            registry
                .profiles()
                .iter()
                .map(Profile::id)
                .collect::<Vec<_>>(),
            ["claude", "codex", "pi", "opencode", "z-last", "a-first"]
        );
        for profile in [Profile::Claude, Profile::Codex, Profile::Pi] {
            assert_eq!(registry.resolve(profile.id()), Some(profile));
        }
        assert_eq!(registry.resolve("a-first").unwrap().label(), "My Agent");
        for id in ["", "unknown", "Claude"] {
            assert!(registry.resolve(id).is_none());
            assert!(registry
                .require(id)
                .unwrap_err()
                .to_string()
                .contains("unavailable"));
        }
    }

    #[test]
    fn custom_launch_preserves_argv_env_and_json_roundtrip() {
        let definition = custom("custom");
        let agent = crate::profile_agent(Profile::Custom(definition.clone())).unwrap();
        assert_eq!(agent.config().command(), Path::new(&definition.command));
        assert_eq!(agent.config().arguments(), definition.args);
        assert_eq!(agent.config().environment(), &definition.env);
        let profile = Profile::Custom(definition);
        assert_eq!(
            serde_json::from_str::<Profile>(&serde_json::to_string(&profile).unwrap()).unwrap(),
            profile
        );
    }

    #[cfg(unix)]
    fn executable_file(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, "not an executable program; detection must not run it").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn probe_preserves_path_order_spaces_and_rejects_relative_entries() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first bin");
        let second = root.path().join("second");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        executable_file(&first.join("adapter name"), 0o755);
        executable_file(&second.join("adapter name"), 0o755);
        let probe = LocalProbe::new(
            std::env::join_paths([Path::new(""), Path::new("relative"), &first, &second]).unwrap(),
        );
        assert_eq!(
            probe.command("adapter name"),
            Availability::Found(first.join("adapter name"))
        );
        assert_eq!(
            probe.command(first.join("adapter name").to_str().unwrap()),
            Availability::Found(first.join("adapter name"))
        );
        assert_eq!(
            LocalProbe::new(":.:relative").command("adapter name"),
            Availability::Missing
        );
        for command in ["./adapter", "dir/adapter", "../adapter", "", ".", ".."] {
            assert!(
                matches!(probe.command(command), Availability::Invalid(_)),
                "{command}"
            );
        }
        executable_file(&first.join("adapter name"), 0o644);
        assert_eq!(
            probe.command("adapter name"),
            Availability::Found(second.join("adapter name"))
        );
        std::fs::remove_file(first.join("adapter name")).unwrap();
        std::os::unix::fs::symlink(root.path().join("absent"), first.join("adapter name")).unwrap();
        assert_eq!(
            probe.command("adapter name"),
            Availability::Found(second.join("adapter name"))
        );
        assert_eq!(
            probe.command(first.to_str().unwrap()),
            Availability::Missing
        );
    }

    #[cfg(unix)]
    #[test]
    fn detection_rereads_filesystem_and_pins_launch_path_without_running() {
        let directory = tempfile::tempdir().unwrap();
        let probe = LocalProbe::new(directory.path());
        let mut definition = custom("local");
        definition.command = "adapter".into();
        let profile = Profile::Custom(definition.clone());
        assert!(local_profile(&profile, &probe).is_err());
        executable_file(&directory.path().join("adapter"), 0o755);
        let resolved = local_profile(&profile, &probe).unwrap().definition();
        assert_eq!(
            resolved.command,
            directory.path().join("adapter").to_str().unwrap()
        );
        assert_eq!(resolved.args, definition.args);
        assert_eq!(resolved.env, definition.env);
        definition.env.insert("PATH".into(), "relative".into());
        assert_eq!(probe.definition(&definition), Availability::Missing);
        std::fs::remove_file(directory.path().join("adapter")).unwrap();
        assert!(local_profile(&profile, &probe).is_err());
    }

    #[test]
    fn catalog_aliases_custom_priority_and_offline_opencode() {
        let registry =
            AgentRegistry::from_definitions(vec![custom("opencode"), custom("claude-acp")])
                .unwrap();
        let catalog = Catalog::parse(br#"{"version":"1.0.0","agents":[{"id":"claude-acp","name":"Remote Claude"},{"id":"opencode","name":"Remote OpenCode"},{"id":"unknown","name":"Unknown","distribution":{"binary":{"linux-x86_64":{"cmd":"evil"}}}}]}"#).unwrap();
        let entries = registry.entries(Some(&catalog), &LocalProbe::new(""));
        assert_eq!(
            entries.iter().filter(|entry| entry.id == "claude").count(),
            1
        );
        let claude = entries.iter().find(|entry| entry.id == "claude").unwrap();
        assert!(claude.user_configured);
        assert!(claude.install.is_none());
        assert_eq!(
            registry.require("claude-acp").unwrap(),
            registry.require("claude").unwrap()
        );
        assert_eq!(
            registry.require("opencode").unwrap().definition().command,
            custom("opencode").command
        );
        assert_eq!(entries.last().unwrap().availability, Availability::Unmapped);
        assert!(!entries.last().unwrap().can_start());
        assert!(
            AgentRegistry::from_definitions(vec![custom("claude"), custom("claude-acp")]).is_err()
        );
        let offline = AgentRegistry::default()
            .require("opencode")
            .unwrap()
            .definition();
        assert_eq!(offline.command, "opencode");
        assert_eq!(offline.args, ["acp"]);
    }

    #[test]
    fn reports_actual_local_opencode_without_executing_it() {
        let availability = LocalProbe::current().command("opencode");
        eprintln!("Actual local OpenCode filesystem detection: {availability:?}");
        if let Availability::Found(path) = availability {
            assert!(path.is_absolute());
        }
    }

    #[test]
    fn loads_only_explicit_user_file_and_reports_path_and_validation_errors() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agents.json");
        assert_eq!(AgentRegistry::load(&path).unwrap().profiles().len(), 4);
        // A project-local file is not consulted when the user-level path is absent.
        let project = directory.path().join("project");
        std::fs::create_dir(&project).unwrap();
        std::fs::write(project.join("agents.json"), "invalid project configuration").unwrap();
        assert_eq!(AgentRegistry::load(&path).unwrap().profiles().len(), 4);
        std::fs::write(
            &path,
            serde_json::json!({"agents": [custom("mine")]}).to_string(),
        )
        .unwrap();
        assert_eq!(
            AgentRegistry::load(&path)
                .unwrap()
                .require("mine")
                .unwrap()
                .definition(),
            custom("mine")
        );
        for json in [
            "not json",
            r#"{"agents":[{"id":"mine","label":"Mine","command":" "}]}"#,
            r#"{"agents":[],"typo":true}"#,
        ] {
            std::fs::write(&path, json).unwrap();
            let error = format!("{:#}", AgentRegistry::load(&path).unwrap_err());
            assert!(error.contains(&path.display().to_string()));
        }
    }
}
