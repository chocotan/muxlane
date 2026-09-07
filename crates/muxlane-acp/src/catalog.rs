//! Untrusted discovery metadata. Distribution commands are intentionally not modeled.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    io::{Read, Write},
    path::Path,
    time::Duration,
};

pub const REGISTRY_URL: &str =
    "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";
pub const MAX_CATALOG_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Catalog {
    pub version: String,
    pub agents: Vec<CatalogAgent>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CatalogAgent {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub website: Option<String>,
    pub repository: Option<String>,
}

impl Catalog {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() <= MAX_CATALOG_BYTES,
            "ACP catalog exceeds 2 MiB"
        );
        let catalog: Self = serde_json::from_slice(bytes).context("parse ACP catalog")?;
        let version: Vec<_> = catalog.version.split('.').collect();
        ensure!(
            version.len() == 3
                && version[0] == "1"
                && version
                    .iter()
                    .all(|part| !part.is_empty() && part.bytes().all(|c| c.is_ascii_digit())),
            "unsupported ACP catalog schema {}",
            catalog.version
        );
        let mut ids = HashSet::new();
        for agent in &catalog.agents {
            ensure!(
                !agent.id.is_empty()
                    && agent
                        .id
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'),
                "invalid catalog agent id"
            );
            ensure!(
                !agent.name.trim().is_empty() && !agent.name.chars().any(char::is_control),
                "invalid catalog agent name"
            );
            ensure!(
                ids.insert(&agent.id),
                "duplicate catalog agent id {}",
                agent.id
            );
        }
        Ok(catalog)
    }

    pub fn load_cache(path: &Path) -> Result<Option<Self>> {
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("read ACP catalog cache"),
        };
        let mut bytes = Vec::new();
        file.take((MAX_CATALOG_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        Self::parse(&bytes).map(Some)
    }

    pub fn save_cache(&self, path: &Path) -> Result<()> {
        let parent = path.parent().context("catalog cache needs parent")?;
        std::fs::create_dir_all(parent)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        temp.write_all(&serde_json::to_vec(self)?)?;
        temp.as_file().sync_all()?;
        temp.persist(path).context("replace ACP catalog cache")?;
        Ok(())
    }

    pub async fn fetch() -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let mut response = client.get(REGISTRY_URL).send().await?.error_for_status()?;
        ensure!(
            response.content_length().unwrap_or(0) <= MAX_CATALOG_BYTES as u64,
            "ACP catalog exceeds 2 MiB"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len().saturating_add(chunk.len()) <= MAX_CATALOG_BYTES,
                "ACP catalog exceeds 2 MiB"
            );
            bytes.extend_from_slice(&chunk);
        }
        Self::parse(&bytes)
    }
}

/// URLs are only opened on user request, never fetched as discovery assets.
pub fn documentation_url(value: &str) -> Option<String> {
    let url = reqwest::Url::parse(value).ok()?;
    (url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}

#[derive(Debug, Clone, Copy)]
pub struct LocalRecipe {
    pub id: &'static str,
    pub catalog_id: &'static str,
    pub label: &'static str,
    pub command: &'static str,
    pub args: &'static [&'static str],
    pub install: &'static str,
    pub docs: &'static str,
}

impl LocalRecipe {
    pub fn definition(&self) -> crate::AgentDefinition {
        crate::AgentDefinition {
            id: self.id.into(),
            label: self.label.into(),
            command: self.command.into(),
            args: self.args.iter().map(|arg| (*arg).into()).collect(),
            env: Default::default(),
        }
    }
}

pub const LOCAL_RECIPES: &[LocalRecipe] = &[
    LocalRecipe {
        id: "claude",
        catalog_id: "claude-acp",
        label: "Claude",
        command: "claude-agent-acp",
        args: &[],
        install: "npm install -g @agentclientprotocol/claude-agent-acp",
        docs: "https://github.com/agentclientprotocol/claude-agent-acp",
    },
    LocalRecipe {
        id: "codex",
        catalog_id: "codex-acp",
        label: "Codex",
        command: "codex-acp",
        args: &[],
        install: "npm install -g @agentclientprotocol/codex-acp",
        docs: "https://github.com/agentclientprotocol/codex-acp",
    },
    LocalRecipe {
        id: "pi",
        catalog_id: "pi-acp",
        label: "Pi",
        command: "pi-acp",
        args: &[],
        install: "npm install -g pi-acp",
        docs: "https://github.com/svkozak/pi-acp",
    },
    LocalRecipe {
        id: "opencode",
        catalog_id: "opencode",
        label: "OpenCode",
        command: "opencode",
        args: &["acp"],
        install: "npm install -g opencode-ai",
        docs: "https://opencode.ai/docs/",
    },
];

pub fn canonical_id(id: &str) -> &str {
    LOCAL_RECIPES
        .iter()
        .find(|recipe| recipe.catalog_id == id)
        .map_or(id, |recipe| recipe.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    const GOOD: &[u8] = br#"{"version":"1.0.0","agents":[{"id":"x","name":"X","distribution":{"npx":{"package":"malicious;cmd"}},"future":true}],"extensions":[]}"#;
    #[test]
    fn validates_bounded_metadata_and_ignores_execution_fields() {
        let catalog = Catalog::parse(GOOD).unwrap();
        assert!(!serde_json::to_string(&catalog)
            .unwrap()
            .contains("malicious"));
        for bytes in [
            b"bad".as_slice(),
            br#"{"version":"2.0.0","agents":[]}"#,
            br#"{"version":"1.bad.0","agents":[]}"#,
            br#"{"version":"1.0.0","agents":[{"id":"x","name":"X"},{"id":"x","name":"Y"}]}"#,
        ] {
            assert!(Catalog::parse(bytes).is_err());
        }
        assert!(Catalog::parse(&vec![b' '; MAX_CATALOG_BYTES + 1]).is_err());
    }
    #[test]
    fn offline_cache_survives_failed_refresh() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("catalog.json");
        assert!(Catalog::load_cache(&path).unwrap().is_none());
        Catalog::parse(GOOD).unwrap().save_cache(&path).unwrap();
        let failed = Catalog::parse(b"broken response");
        assert!(failed.is_err());
        assert_eq!(
            Catalog::load_cache(&path).unwrap().unwrap().agents[0].id,
            "x"
        );
        let old = std::fs::read(&path).unwrap();
        assert!(Catalog::parse(GOOD)
            .unwrap()
            .save_cache(&path.join("child"))
            .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), old);
    }
    #[tokio::test]
    #[ignore = "explicit live metadata-only smoke test"]
    async fn fetch_official_metadata_only() {
        let catalog = Catalog::fetch().await.unwrap();
        eprintln!(
            "Official ACP catalog schema {}, {} agents",
            catalog.version,
            catalog.agents.len()
        );
        assert!(catalog.agents.iter().any(|agent| agent.id == "opencode"));
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("cache.json");
        catalog.save_cache(&path).unwrap();
        assert_eq!(
            Catalog::load_cache(&path).unwrap().unwrap().agents.len(),
            catalog.agents.len()
        );
    }

    #[test]
    fn only_https_documentation_links() {
        for url in [
            "javascript:alert(1)",
            "file:///tmp/a",
            "https://user:pw@example.com",
            "not a url",
        ] {
            assert!(documentation_url(url).is_none());
        }
        assert!(documentation_url("https://opencode.ai/docs/").is_some());
    }
}
