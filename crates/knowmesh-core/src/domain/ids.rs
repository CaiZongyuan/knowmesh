use std::{borrow::Cow, fmt, str::FromStr};

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::error::{AppError, ErrorType};

macro_rules! typed_id {
    ($name:ident, $prefix:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new() -> Self {
                Self(format!("{}{}", $prefix, Ulid::new()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self { Self::new() }
        }

        impl FromStr for $name {
            type Err = AppError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                if let Some(payload) = value.strip_prefix($prefix)
                    && let Ok(id) = payload.parse::<Ulid>()
                    && id.to_string() == payload
                {
                    return Ok(Self(value.to_owned()));
                }
                Err(AppError::new(
                    ErrorType::Validation,
                    "INVALID_ID",
                    concat!("Expected ", $prefix, " followed by a canonical ULID."),
                ).with_hint("Use the typed ID returned by the corresponding list or search operation."))
            }
        }

        impl TryFrom<String> for $name {
            type Error = AppError;
            fn try_from(value: String) -> Result<Self, Self::Error> { value.parse() }
        }

        impl From<$name> for String {
            fn from(value: $name) -> String { value.0 }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(f) }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> { stringify!($name).into() }
            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!({
                    "type": "string",
                    "pattern": concat!("^", $prefix, "[0-7][0-9A-HJKMNP-TV-Z]{25}$")
                })
            }
        }
    };
}

typed_id!(WorkspaceId, "ws_");
typed_id!(SourceId, "src_");
typed_id!(SourceRevisionId, "rev_");
typed_id!(NodeId, "kn_");
typed_id!(ClaimId, "clm_");
typed_id!(ConflictGroupId, "cfg_");
typed_id!(RelationId, "rel_");
typed_id!(EvidenceId, "evd_");
typed_id!(SynthesisId, "syn_");
typed_id!(ProposalId, "prp_");
typed_id!(ProposalItemId, "pri_");
typed_id!(RunId, "run_");
typed_id!(ChunkId, "chk_");
typed_id!(SourceBlockId, "blk_");

impl SourceBlockId {
    pub(crate) fn from_digest(digest: [u8; 32]) -> Self {
        let mut bytes = [0; 16];
        bytes.copy_from_slice(&digest[..16]);
        Self(format!("blk_{}", Ulid::from_bytes(bytes)))
    }
}

impl ChunkId {
    pub(crate) fn from_digest(digest: [u8; 32]) -> Self {
        let mut bytes = [0; 16];
        bytes.copy_from_slice(&digest[..16]);
        Self(format!("chk_{}", Ulid::from_bytes(bytes)))
    }
}
