use super::{
    Capability, CapabilityDefinition, CapabilityKind, CapabilityMetadata, CapabilityParameter,
    SessionMode, ShellInteractiveCapability, ShellScriptCapability,
};
use crate::config::parser::{CapabilityConfig, Config};
use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::sync::Arc;

pub struct CapabilityHandle {
    capability: Arc<dyn Capability>,
}

impl CapabilityHandle {
    pub fn capability(&self) -> Arc<dyn Capability> {
        Arc::clone(&self.capability)
    }
}

#[derive(Default)]
pub struct CapabilityRegistry {
    capabilities: HashMap<String, Arc<dyn Capability>>,
}

impl CapabilityRegistry {
    pub fn from_config(config: &Config) -> Result<Self> {
        let mut registry = CapabilityRegistry::default();
        for capability in &config.capabilities {
            registry.register(capability.clone())?;
        }
        Ok(registry)
    }

    pub fn register(&mut self, config: CapabilityConfig) -> Result<()> {
        if self.capabilities.contains_key(&config.id) {
            return Err(anyhow!(
                "Duplicate capability '{}' while building registry",
                config.id
            ));
        }

        let metadata = build_metadata(&config);

        let capability: Arc<dyn Capability> = match &config.definition {
            CapabilityDefinition::ShellScript(_) => {
                Arc::new(ShellScriptCapability::new(config, metadata))
            }
            CapabilityDefinition::ShellInteractive(_) => {
                Arc::new(ShellInteractiveCapability::new(config, metadata))
            }
        };

        self.capabilities
            .insert(capability.metadata().id.clone(), capability);

        Ok(())
    }

    pub fn list_metadata(&self) -> Vec<CapabilityMetadata> {
        self.capabilities
            .values()
            .map(|cap| cap.metadata().clone())
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<CapabilityHandle> {
        self.capabilities.get(id).map(|cap| CapabilityHandle {
            capability: Arc::clone(cap),
        })
    }
}

fn build_metadata(config: &CapabilityConfig) -> CapabilityMetadata {
    CapabilityMetadata {
        id: config.id.clone(),
        name: config.name.clone(),
        description: config.description.clone(),
        tags: config.tags.clone(),
        kind: CapabilityKind::from(&config.definition),
        session_mode: SessionMode::from(config.definition.session_mode()),
        requires_confirmation: config.requires_confirmation,
        privileged: config.privileged,
        parameters: config
            .parameters
            .iter()
            .map(CapabilityParameter::from)
            .collect(),
    }
}
