pub mod parameters;
pub mod registry;
pub mod shell_interactive;
pub mod shell_script;

pub use registry::{CapabilityHandle, CapabilityRegistry};
pub use shell_interactive::ShellInteractiveCapability;
pub use shell_script::ShellScriptCapability;

use crate::config::parser::{
    CapabilityDefinition, CapabilityParameterConfig, CapabilitySessionMode as ConfigSessionMode,
};
use async_trait::async_trait;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionMode {
    OneShot,
    Realtime,
    Upload,
    Download,
}

impl From<ConfigSessionMode> for SessionMode {
    fn from(value: ConfigSessionMode) -> Self {
        match value {
            ConfigSessionMode::OneShot => SessionMode::OneShot,
            ConfigSessionMode::Realtime => SessionMode::Realtime,
            ConfigSessionMode::Upload => SessionMode::Upload,
            ConfigSessionMode::Download => SessionMode::Download,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityKind {
    ShellScript,
    ShellInteractive,
    FileTransfer,
}

impl From<&CapabilityDefinition> for CapabilityKind {
    fn from(definition: &CapabilityDefinition) -> Self {
        match definition {
            CapabilityDefinition::ShellScript(_) => CapabilityKind::ShellScript,
            CapabilityDefinition::ShellInteractive(_) => CapabilityKind::ShellInteractive,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CapabilityParameter {
    pub name: String,
    pub param_type: String,
    pub description: Option<String>,
    pub min: Option<i32>,
    pub max: Option<i32>,
    pub default_value: Option<String>,
    pub options: Vec<String>,
    pub validation: Option<String>,
    pub label_on: Option<String>,
    pub label_off: Option<String>,
    pub step: Option<i32>,
    pub default_value_command: Option<String>,
    pub default_value_pattern: Option<String>,
}

impl From<&CapabilityParameterConfig> for CapabilityParameter {
    fn from(param: &CapabilityParameterConfig) -> Self {
        Self {
            name: param.name.clone(),
            param_type: param.param_type.clone(),
            description: param.description.clone(),
            min: param.min,
            max: param.max,
            default_value: param.default.clone(),
            options: param.options.clone(),
            validation: param.validation.clone(),
            label_on: param.label_on.clone(),
            label_off: param.label_off.clone(),
            step: param.step,
            default_value_command: param.default_value_command.clone(),
            default_value_pattern: param.default_value_pattern.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CapabilityMetadata {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub kind: CapabilityKind,
    pub session_mode: SessionMode,
    pub requires_confirmation: bool,
    pub privileged: bool,
    pub parameters: Vec<CapabilityParameter>,
}

pub struct CapabilityOpenContext {
    pub metadata: CapabilityMetadata,
    pub parameters: std::collections::HashMap<String, String>,
    pub client_fingerprint: Option<String>,
}

#[async_trait]
pub trait Capability: Send + Sync {
    fn metadata(&self) -> &CapabilityMetadata;
    fn definition(&self) -> &CapabilityDefinition;

    async fn open_session(
        &self,
        ctx: CapabilityOpenContext,
        endpoints: crate::sessions::SessionEndpoints,
    ) -> anyhow::Result<()>;
}
