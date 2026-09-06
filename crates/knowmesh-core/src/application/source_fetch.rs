use std::net::IpAddr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use url::{Host, Url};

use crate::{
    canonical::{
        snapshot::CanonicalSnapshot,
        source::{ImportInput, ImportedContent, SourceLibrary},
        workspace::Workspace,
    },
    domain::{StorageMode, validate_source_url},
    error::{AppError, AppResult, ErrorType},
    ports::SourceFetcher,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddInput {
    #[serde(flatten)]
    pub import: ImportInput,
    #[serde(default)]
    pub allow_private_network: bool,
}

pub struct FetchRequest {
    pub url: String,
    pub max_bytes: u64,
    pub connect_timeout_seconds: u64,
    pub fetch_timeout_seconds: u64,
    pub allow_private_network: bool,
}

pub fn fetch(
    workspace: &Workspace,
    input: &ImportInput,
    allow_private_network: bool,
    fetcher: &dyn SourceFetcher,
) -> AppResult<Option<ImportedContent>> {
    let Some(value) = input.path.to_str().filter(|value| value.contains("://")) else {
        return Ok(None);
    };
    if !workspace.config.sources.allow_remote_urls {
        return Err(AppError::new(
            ErrorType::Policy,
            "REMOTE_URL_DISABLED",
            "Remote URL ingestion is disabled for this workspace.",
        ));
    }
    let url = validate_url(value, allow_private_network)?;
    if input
        .storage
        .is_some_and(|storage| storage != StorageMode::SnapshotUrl)
    {
        return Err(AppError::new(
            ErrorType::Validation,
            "SOURCE_STORAGE_MISMATCH",
            "URL inputs require snapshot-url storage.",
        ));
    }
    if let Some(id) = &input.source_id {
        let source = SourceLibrary::new(workspace).get(id)?;
        if source.manifest.removed_at.is_some() {
            return Err(AppError::new(
                ErrorType::Conflict,
                "SOURCE_REMOVED",
                "Removed sources cannot receive new revisions.",
            ));
        }
        if source.manifest.storage != StorageMode::SnapshotUrl {
            return Err(AppError::new(
                ErrorType::Validation,
                "SOURCE_STORAGE_MISMATCH",
                "Appending a revision must preserve the source storage mode.",
            ));
        }
    }
    CanonicalSnapshot::scan(workspace)?;
    let settings = &workspace.config.sources;
    let result = fetcher.fetch(&FetchRequest {
        url: url.into(),
        max_bytes: settings.max_file_mib * 1024 * 1024,
        connect_timeout_seconds: settings.connect_timeout_seconds,
        fetch_timeout_seconds: settings.fetch_timeout_seconds,
        allow_private_network,
    })?;
    validate_url(&result.final_url, allow_private_network)?;
    if result.bytes.len() as u64 > settings.max_file_mib * 1024 * 1024 {
        return Err(too_large());
    }
    crate::canonical::source::validate_content(&result.mime_type, &result.bytes)?;
    Ok(Some(result))
}

pub fn validate_url(value: &str, allow_private_network: bool) -> AppResult<Url> {
    let mut url = validate_source_url(value)?;
    match url.host() {
        Some(Host::Ipv4(ip)) => validate_address(ip.into(), allow_private_network)?,
        Some(Host::Ipv6(ip)) => validate_address(ip.into(), allow_private_network)?,
        _ => {}
    }
    url.set_fragment(None);
    Ok(url)
}

pub fn validate_address(ip: IpAddr, allow_private_network: bool) -> AppResult<()> {
    if allow_private_network {
        return Ok(());
    }
    let public = match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_documentation()
                && a != 0
                && a < 224
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 198 && (18..=19).contains(&b))
                && !(a == 192 && (b == 0 && c == 0 || b == 88 && c == 99))
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return validate_address(mapped.into(), false);
            }
            let s = ip.segments();
            // Only ordinary global unicast: exclude translation/tunnel and documentation ranges.
            s[0] & 0xe000 == 0x2000
                && s[0] != 0x2002
                && !(s[0] == 0x2001 && (s[1] < 0x0200 || s[1] == 0x0db8))
                && !(s[0] == 0x3fff && s[1] < 0x1000)
        }
    };
    if !public {
        return Err(AppError::new(ErrorType::Policy, "PRIVATE_NETWORK_BLOCKED", "The source target is not on an allowed public network.")
            .with_hint("For a trusted local resource, explicitly pass --allow-private-network to the local CLI."));
    }
    Ok(())
}

pub fn too_large() -> AppError {
    AppError::new(
        ErrorType::Validation,
        "SOURCE_TOO_LARGE",
        "Source exceeds the workspace file size limit.",
    )
}
