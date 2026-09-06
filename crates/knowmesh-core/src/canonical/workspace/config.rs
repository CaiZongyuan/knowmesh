use std::collections::BTreeMap;

use schemars::JsonSchema;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::{
    domain::WorkspaceId,
    error::{AppError, AppResult, ErrorType},
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceConfig {
    pub version: u32,
    pub workspace: WorkspaceSettings,
    pub schema: SchemaSettings,
    #[serde(default)]
    pub sources: SourceSettings,
    #[serde(default)]
    pub compiler: CompilerSettings,
    #[serde(default)]
    pub embedding: EmbeddingSettings,
    #[serde(default)]
    pub search: SearchSettings,
    #[serde(default)]
    pub server: ServerSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSettings {
    pub id: WorkspaceId,
    pub name: String,
    pub default_language: String,
    pub template: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SchemaSettings {
    pub packs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SourceSettings {
    pub default_storage: String,
    pub max_file_mib: u64,
    pub allow_remote_urls: bool,
    pub connect_timeout_seconds: u64,
    pub fetch_timeout_seconds: u64,
}

impl Default for SourceSettings {
    fn default() -> Self {
        Self {
            default_storage: "managed".into(),
            max_file_mib: 100,
            allow_remote_urls: true,
            connect_timeout_seconds: 10,
            fetch_timeout_seconds: 60,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct CompilerSettings {
    pub enabled: bool,
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub api_key: String,
    pub max_concurrency: usize,
    pub prompt_version: String,
    pub response_format: crate::model::ResponseFormat,
    pub max_tokens_parameter: crate::model::CompletionTokenParameter,
}

impl Default for CompilerSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: "openai-compatible".into(),
            model: "${KNOWMESH_COMPILER_MODEL}".into(),
            base_url: "${KNOWMESH_LLM_BASE_URL}".into(),
            api_key: "${KNOWMESH_LLM_API_KEY}".into(),
            max_concurrency: 4,
            prompt_version: "compiler-v1".into(),
            response_format: Default::default(),
            max_tokens_parameter: Default::default(),
        }
    }
}

#[derive(Debug)]
pub struct ResolvedCompilerSettings {
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub api_key: SecretString,
    pub response_format: crate::model::ResponseFormat,
    pub max_tokens_parameter: crate::model::CompletionTokenParameter,
}

impl CompilerSettings {
    pub fn resolve(&self, env: &BTreeMap<String, String>) -> AppResult<ResolvedCompilerSettings> {
        if !self.enabled {
            return Err(config_error(
                "MODEL_DISABLED",
                "Compiler model calls are disabled.",
            ));
        }
        if !self.api_key.starts_with("${") {
            return Err(config_error(
                "INVALID_SECRET_REFERENCE",
                "API keys must reference an environment variable.",
            ));
        }
        Ok(ResolvedCompilerSettings {
            provider: self.provider.clone(),
            model: resolve_value(&self.model, env)?,
            base_url: resolve_value(&self.base_url, env)?,
            api_key: resolve_value(&self.api_key, env)?.into(),
            response_format: self.response_format,
            max_tokens_parameter: self.max_tokens_parameter,
        })
    }
}

pub fn resolve_value(value: &str, env: &BTreeMap<String, String>) -> AppResult<String> {
    if let Some(name) = value.strip_prefix("${").and_then(|v| v.strip_suffix('}')) {
        if name.is_empty()
            || !name
                .bytes()
                .enumerate()
                .all(|(i, c)| c == b'_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
        {
            return Err(config_error(
                "INVALID_ENV_REFERENCE",
                "Expected a valid environment variable name.",
            ));
        }
        env.get(name)
            .filter(|v| !v.trim().is_empty())
            .cloned()
            .ok_or_else(|| {
                config_error(
                    "MODEL_NOT_CONFIGURED",
                    "A required model environment variable is missing.",
                )
                .with_param(name)
            })
    } else if value.contains("${") {
        Err(config_error(
            "INVALID_ENV_REFERENCE",
            "Environment references must occupy the whole value.",
        ))
    } else if value.trim().is_empty() {
        Err(config_error(
            "MODEL_NOT_CONFIGURED",
            "A required model configuration value is empty.",
        ))
    } else {
        Ok(value.to_owned())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct EmbeddingSettings {
    pub enabled: bool,
    pub provider: String,
    pub model: String,
    pub dimensions: usize,
    pub api_key: String,
    pub base_url: String,
}

impl Default for EmbeddingSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "openai-compatible".into(),
            model: "${KNOWMESH_EMBEDDING_MODEL}".into(),
            dimensions: 1024,
            api_key: "${KNOWMESH_EMBEDDING_API_KEY}".into(),
            base_url: "${KNOWMESH_EMBEDDING_BASE_URL}".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SearchSettings {
    pub default_limit: usize,
    pub rrf_k: usize,
    pub word_weight: f64,
    pub trigram_weight: f64,
    pub vector_weight: f64,
    pub boosts_enabled: bool,
    pub candidate_limit: u32,
    pub lexical_timeout_ms: u64,
    pub graph_expansion_depth: usize,
}

impl Default for SearchSettings {
    fn default() -> Self {
        Self {
            default_limit: 20,
            rrf_k: 60,
            word_weight: 1.0,
            trigram_weight: 0.8,
            vector_weight: 1.0,
            boosts_enabled: true,
            candidate_limit: 100,
            lexical_timeout_ms: 200,
            graph_expansion_depth: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ServerSettings {
    pub host: String,
    pub port: u16,
}

impl Default for ServerSettings {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 7331,
        }
    }
}

impl WorkspaceConfig {
    pub fn research(name: String, template: String) -> Self {
        let packs = if ["research", "clinical"].contains(&template.as_str()) {
            vec![
                "./schemas/base.yaml".into(),
                format!("./schemas/{template}.yaml"),
            ]
        } else {
            vec!["./schemas/base.yaml".into()]
        };
        Self {
            version: 1,
            workspace: WorkspaceSettings {
                id: WorkspaceId::new(),
                name,
                default_language: "zh-CN".into(),
                purpose: (template == "research").then(|| "./purpose.md".into()),
                template,
            },
            schema: SchemaSettings { packs },
            sources: SourceSettings::default(),
            compiler: CompilerSettings::default(),
            embedding: EmbeddingSettings::default(),
            search: SearchSettings::default(),
            server: ServerSettings::default(),
        }
    }

    pub fn parse(bytes: &[u8]) -> AppResult<Self> {
        let value: serde_yaml::Value = serde_yaml::from_slice(bytes).map_err(|_| {
            config_error(
                "INVALID_CONFIGURATION",
                "Workspace configuration is not valid YAML.",
            )
        })?;
        if value["version"].as_u64() != Some(1) {
            return Err(config_error(
                "UNSUPPORTED_CONFIG_VERSION",
                "Only workspace configuration version 1 is supported.",
            ));
        }
        let config: Self = serde_yaml::from_value(value).map_err(|_| {
            config_error(
                "INVALID_CONFIGURATION",
                "Workspace configuration has invalid or unknown fields.",
            )
        })?;
        if config.workspace.name.trim().is_empty()
            || config.schema.packs.is_empty()
            || config.sources.max_file_mib == 0
            || config.sources.max_file_mib > u64::MAX / (1024 * 1024)
            || !(1..=300).contains(&config.sources.connect_timeout_seconds)
            || !(1..=3600).contains(&config.sources.fetch_timeout_seconds)
            || !(1..=32).contains(&config.compiler.max_concurrency)
            || !(1..=65536).contains(&config.embedding.dimensions)
            || !(1..=100).contains(&config.search.default_limit)
            || config.search.rrf_k == 0
            || [
                config.search.word_weight,
                config.search.trigram_weight,
                config.search.vector_weight,
            ]
            .iter()
            .any(|weight| !weight.is_finite() || *weight <= 0.0)
            || !(1..=500).contains(&config.search.candidate_limit)
            || !(1..=5000).contains(&config.search.lexical_timeout_ms)
            || !(1..=3).contains(&config.search.graph_expansion_depth)
            || !["managed", "referenced", "snapshot-url"]
                .contains(&config.sources.default_storage.as_str())
        {
            return Err(config_error(
                "INVALID_CONFIGURATION",
                "Workspace configuration contains values outside the supported range.",
            ));
        }
        Ok(config)
    }
}

pub(super) fn config_error(code: &str, message: &str) -> AppError {
    AppError::new(ErrorType::Configuration, code, message)
        .with_hint("Check knowmesh.yaml and the referenced environment variables.")
}
