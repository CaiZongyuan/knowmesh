use std::collections::BTreeMap;

use super::{ParsedMetadata, builder::collapse};
use crate::error::{AppError, AppResult, ErrorType};

pub(super) const MAX_BYTES: usize = 64 * 1024;

pub(super) fn parse(text: &str) -> AppResult<ParsedMetadata> {
    if text.len() > MAX_BYTES {
        return Err(super::limit_error());
    }
    if yaml_edit::lex(text)
        .iter()
        .any(|(kind, _)| *kind == yaml_edit::SyntaxKind::REFERENCE)
    {
        return Err(AppError::new(
            ErrorType::Validation,
            "SOURCE_FRONTMATTER_ALIAS_UNSUPPORTED",
            "Source frontmatter cannot expand YAML aliases.",
        )
        .with_hint("Expand aliases into bounded literal metadata before importing the source."));
    }
    let value: serde_yaml::Value = serde_yaml::from_str(text).map_err(|_| invalid())?;
    let attributes: BTreeMap<String, serde_json::Value> = if value.is_null() {
        BTreeMap::new()
    } else {
        serde_json::from_value(serde_json::to_value(value).map_err(|_| invalid())?)
            .map_err(|_| invalid())?
    };
    let title = attributes
        .get("title")
        .and_then(serde_json::Value::as_str)
        .map(collapse)
        .filter(|title| !title.is_empty());
    let language = attributes
        .get("language")
        .or_else(|| attributes.get("lang"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|language| !language.is_empty())
        .map(str::to_owned);
    Ok(ParsedMetadata {
        title,
        language,
        attributes,
    })
}

fn invalid() -> AppError {
    AppError::new(
        ErrorType::Validation,
        "INVALID_SOURCE_FRONTMATTER",
        "Source frontmatter must be a JSON-compatible YAML mapping.",
    )
    .with_hint("Correct the source frontmatter while retaining the original imported revision.")
}
