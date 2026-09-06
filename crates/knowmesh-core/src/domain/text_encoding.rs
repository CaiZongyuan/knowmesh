use std::{borrow::Cow, fmt, str::FromStr};

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult, ErrorType};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TextEncoding(String);

impl Default for TextEncoding {
    fn default() -> Self {
        Self("utf-8".into())
    }
}

impl TextEncoding {
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn is_utf8(&self) -> bool {
        self.0 == "utf-8"
    }
}

impl FromStr for TextEncoding {
    type Err = AppError;
    fn from_str(value: &str) -> AppResult<Self> {
        let encoding = (value.len() <= 128)
            .then(|| encoding_rs::Encoding::for_label(value.as_bytes()))
            .flatten()
            .filter(|encoding| *encoding != encoding_rs::REPLACEMENT)
            .ok_or_else(|| {
                AppError::new(
                    ErrorType::Validation,
                    "UNSUPPORTED_SOURCE_ENCODING",
                    "The source encoding label is unsupported.",
                )
                .with_param("encoding")
                .with_hint(
                    "Use a supported encoding label such as utf-8, windows-1252, or utf-16le.",
                )
            })?;
        Ok(Self(encoding.name().to_ascii_lowercase()))
    }
}

impl TryFrom<String> for TextEncoding {
    type Error = AppError;
    fn try_from(value: String) -> AppResult<Self> {
        value.parse()
    }
}
impl From<TextEncoding> for String {
    fn from(value: TextEncoding) -> Self {
        value.0
    }
}
impl fmt::Display for TextEncoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl JsonSchema for TextEncoding {
    fn schema_name() -> Cow<'static, str> {
        "TextEncoding".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type": "string", "maxLength": 128, "description": "Explicit WHATWG encoding label; serialized using its canonical lowercase name."})
    }
}

pub fn decode_source_text<'a>(
    bytes: &'a [u8],
    encoding: Option<&TextEncoding>,
) -> AppResult<Cow<'a, str>> {
    let codec = encoding.map_or(encoding_rs::UTF_8, |encoding| {
        encoding_rs::Encoding::for_label(encoding.as_str().as_bytes())
            .expect("validated encoding label")
    });
    if let Some((bom, _)) = encoding_rs::Encoding::for_bom(bytes)
        && bom != codec
    {
        return Err(invalid());
    }
    codec
        .decode_without_bom_handling_and_without_replacement(bytes)
        .ok_or_else(invalid)
}

fn invalid() -> AppError {
    AppError::new(
        ErrorType::Validation,
        "INVALID_SOURCE_ENCODING",
        "Source bytes do not decode strictly with the declared encoding.",
    )
    .with_param("encoding")
    .with_hint("Specify the original text encoding explicitly; UTF-8 is the default.")
}
