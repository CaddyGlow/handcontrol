use crate::config;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use uuid::Uuid;

const REGISTRY_FILE_NAME: &str = "servers.toml";

/// Represents the persistent registry of enrolled servers.
#[derive(Debug, Clone, Default)]
pub struct ServerRegistry {
    entries: Vec<ServerRegistryEntry>,
}

impl ServerRegistry {
    /// Load registry entries from disk. Missing files yield an empty registry.
    pub fn load() -> Result<Self> {
        let path = registry_path()?;

        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        if contents.trim().is_empty() {
            return Ok(Self::default());
        }

        let file: ServerRegistryFile = toml::from_str(&contents)
            .with_context(|| format!("Failed to parse {}", path.display()))?;

        Ok(Self {
            entries: file.server.unwrap_or_default(),
        })
    }

    /// Persist the registry to disk.
    pub fn save(&self) -> Result<()> {
        let path = registry_path()?;
        let file = ServerRegistryFile {
            server: if self.entries.is_empty() {
                None
            } else {
                Some(self.entries.clone())
            },
        };

        let contents =
            toml::to_string_pretty(&file).context("Failed to serialize server registry")?;
        fs::write(&path, contents)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(())
    }

    pub fn entries(&self) -> &[ServerRegistryEntry] {
        &self.entries
    }

    pub fn into_entries(self) -> Vec<ServerRegistryEntry> {
        self.entries
    }

    pub fn iter(&self) -> impl Iterator<Item = &ServerRegistryEntry> {
        self.entries.iter()
    }

    pub fn find_by_id(&self, id: &Uuid) -> Option<&ServerRegistryEntry> {
        self.entries.iter().find(|entry| &entry.id == id)
    }

    pub fn upsert(&mut self, entry: ServerRegistryEntry) {
        if let Some(existing) = self.entries.iter_mut().find(|e| e.id == entry.id) {
            *existing = entry;
        } else {
            self.entries.push(entry);
        }
    }

    pub fn remove(&mut self, id: &Uuid) {
        self.entries.retain(|entry| &entry.id != id);
    }
}

/// Individual server entry stored in `servers.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerRegistryEntry {
    pub id: Uuid,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub ip: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub enrolled_at: Option<String>,
    #[serde(default)]
    pub last_seen: Option<String>,
    #[serde(default)]
    pub client_id: Option<Uuid>,
    #[serde(default)]
    pub cert_fingerprint: Option<String>,
    #[serde(default)]
    pub cert_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ServerRegistryFile {
    #[serde(default)]
    server: Option<Vec<ServerRegistryEntry>>,
}

fn registry_path() -> Result<PathBuf> {
    let mut path = config::config_dir()?;
    path.push(REGISTRY_FILE_NAME);
    Ok(path)
}
