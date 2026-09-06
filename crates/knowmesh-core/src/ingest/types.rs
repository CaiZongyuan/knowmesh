use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::{SourceBlockId, SourceRevisionId, TextEncoding};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    Heading,
    Paragraph,
    ListItem,
    Quote,
    Code,
    Table,
    FigureCaption,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ByteSpan {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SourceBlock {
    pub id: SourceBlockId,
    pub kind: BlockKind,
    pub text: String,
    pub page: Option<u32>,
    pub section_path: Vec<String>,
    pub paragraph: Option<u32>,
    pub char_start: usize,
    pub char_end: usize,
    pub source_bytes: Option<ByteSpan>,
    pub heading_level: Option<u8>,
    pub language: Option<String>,
    pub caption: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ParsedMetadata {
    pub title: Option<String>,
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub attributes: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ParseWarning {
    pub code: String,
    pub hint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExtractionQuality {
    pub usable_for_compile: bool,
    pub visible_characters: usize,
    pub replacement_characters: usize,
    pub replacement_ratio: f64,
    pub page_count: Option<u32>,
    pub text_pages: Option<u32>,
    pub page_map_reliable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ParsedSource {
    pub version: u32,
    pub source_revision_id: SourceRevisionId,
    pub source_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_encoding: Option<TextEncoding>,
    pub normalized_text: String,
    pub text_sha256: String,
    pub metadata: ParsedMetadata,
    pub blocks: Vec<SourceBlock>,
    pub warnings: Vec<ParseWarning>,
    pub quality: ExtractionQuality,
    pub parser_name: String,
    pub parser_version: String,
    pub parser_config_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ParserDescriptor {
    pub name: String,
    pub version: String,
    pub config_sha256: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct ParseLimits {
    pub max_bytes: usize,
    pub max_blocks: usize,
}

impl Default for ParseLimits {
    fn default() -> Self {
        Self {
            max_bytes: 100 * 1024 * 1024,
            max_blocks: 100_000,
        }
    }
}
