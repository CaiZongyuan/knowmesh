use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::{AppError, AppResult, ErrorType};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IdentifierNormalization {
    Doi,
    NcbiGene,
    Opaque,
}

impl IdentifierNormalization {
    pub fn normalize(self, value: &str) -> AppResult<String> {
        let value = value.trim();
        if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
            return Err(invalid());
        }
        match self {
            Self::Opaque => Ok(value.into()),
            Self::NcbiGene => {
                if value.len() > 20 || !value.bytes().all(|ch| ch.is_ascii_digit()) {
                    return Err(invalid());
                }
                let value = value.trim_start_matches('0');
                if value.is_empty() {
                    return Err(invalid());
                }
                Ok(value.into())
            }
            Self::Doi => normalize_doi(value),
        }
    }
}

fn normalize_doi(value: &str) -> AppResult<String> {
    let value = if value
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("doi:"))
    {
        value[4..].trim().to_owned()
    } else if value.contains("://") {
        let url = Url::parse(value).map_err(|_| invalid())?;
        if !matches!(url.scheme(), "http" | "https")
            || !matches!(url.host_str(), Some("doi.org" | "dx.doi.org"))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(invalid());
        }
        percent_encoding::percent_decode_str(url.path().strip_prefix('/').ok_or_else(invalid)?)
            .decode_utf8()
            .map_err(|_| invalid())?
            .into_owned()
    } else {
        value.to_owned()
    };
    let (prefix, suffix) = value.split_once('/').ok_or_else(invalid)?;
    let registrant = prefix.strip_prefix("10.").ok_or_else(invalid)?;
    if !(4..=9).contains(&registrant.len())
        || !registrant.bytes().all(|ch| ch.is_ascii_digit())
        || suffix.is_empty()
        || !suffix.bytes().all(|ch| ch.is_ascii_graphic())
    {
        return Err(invalid());
    }
    Ok(value.to_ascii_lowercase())
}

fn invalid() -> AppError {
    AppError::new(
        ErrorType::Validation,
        "INVALID_ENTITY_IDENTIFIER",
        "The entity identifier is invalid for its Schema normalization rule.",
    )
}
