mod io;
mod keys;
mod stages;

pub use keys::{CacheStage, ModelIdentity, StageKey};
pub use stages::{chunk_cached, parse_cached};

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    canonical::workspace::{Workspace, runtime_path},
    domain::valid_sha256,
    error::{AppError, AppResult, ErrorType},
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtifactReference {
    pub version: u32,
    pub stage: CacheStage,
    pub key_sha256: String,
    pub path: String,
    pub sha256: String,
    pub byte_size: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    key: StageKey,
    key_sha256: String,
    artifact: ArtifactReference,
}

pub struct CacheHit<T> {
    pub value: T,
    pub reference: ArtifactReference,
    pub cache_hit: bool,
}

pub struct FileStageCache {
    root: PathBuf,
    max_artifact_bytes: u64,
}

impl FileStageCache {
    pub fn new(workspace: &Workspace, max_artifact_bytes: u64) -> AppResult<Self> {
        if max_artifact_bytes == 0 || max_artifact_bytes > u64::MAX - 1 {
            return Err(AppError::new(
                ErrorType::Validation,
                "INVALID_CACHE_LIMIT",
                "The cache size limit must be positive and bounded.",
            ));
        }
        Ok(Self {
            root: workspace.root.clone(),
            max_artifact_bytes,
        })
    }

    pub fn manifest_path(&self, key: &StageKey) -> AppResult<PathBuf> {
        self.path(&format!(
            "cache/{}/entries/{}.json",
            key.stage().name(),
            key.fingerprint()?
        ))
    }

    pub fn load<T: DeserializeOwned>(
        &self,
        key: &StageKey,
        validate: impl FnOnce(&T) -> AppResult<()>,
    ) -> AppResult<Option<CacheHit<T>>> {
        let Some(manifest) =
            io::read_artifact::<Manifest>(&self.manifest_path(key)?, None, 64 * 1024)?
        else {
            return Ok(None);
        };
        let hash = key.fingerprint()?;
        if manifest.version != 1
            || manifest.key_sha256 != hash
            || manifest.key.fingerprint().ok().as_deref() != Some(&hash)
            || manifest.artifact.key_sha256 != hash
            || manifest.artifact.stage != key.stage()
        {
            return Ok(None);
        }
        Ok(self
            .read_reference(&manifest.artifact, validate)?
            .map(|value| CacheHit {
                value,
                reference: manifest.artifact,
                cache_hit: true,
            }))
    }

    pub fn read_reference<T: DeserializeOwned>(
        &self,
        reference: &ArtifactReference,
        validate: impl FnOnce(&T) -> AppResult<()>,
    ) -> AppResult<Option<T>> {
        if reference.version != 1
            || !valid_sha256(&reference.sha256)
            || !valid_sha256(&reference.key_sha256)
            || reference.path != object_path(reference.stage, &reference.sha256)
        {
            return Ok(None);
        }
        let Some(value) = io::read_artifact(
            &self.path(&reference.path)?,
            Some((&reference.sha256, reference.byte_size)),
            self.max_artifact_bytes,
        )?
        else {
            return Ok(None);
        };
        if validate(&value).is_err() {
            return Ok(None);
        }
        Ok(Some(value))
    }

    pub fn store<T: Serialize>(&self, key: &StageKey, value: &T) -> AppResult<ArtifactReference> {
        let key_sha256 = key.fingerprint()?;
        let stage = key.stage();
        let objects = format!("cache/{}/objects", stage.name());
        let entries = format!("cache/{}/entries", stage.name());
        io::ensure_directory(&self.path(&objects)?)?;
        io::ensure_directory(&self.path(&entries)?)?;
        let directory = self.path(&objects)?;
        let (temporary, sha256, byte_size) =
            io::write_artifact(&directory, value, self.max_artifact_bytes)?;
        let reference = ArtifactReference {
            version: 1,
            stage,
            key_sha256: key_sha256.clone(),
            path: object_path(stage, &sha256),
            sha256,
            byte_size,
        };
        io::publish(temporary, &self.path(&reference.path)?)?;
        let manifest = Manifest {
            version: 1,
            key: key.clone(),
            key_sha256,
            artifact: reference.clone(),
        };
        let (temporary, _, _) = io::write_artifact(&self.path(&entries)?, &manifest, 64 * 1024)?;
        io::publish(temporary, &self.manifest_path(key)?)?;
        Ok(reference)
    }

    fn path(&self, relative: &str) -> AppResult<PathBuf> {
        runtime_path(&self.root, Path::new(relative))
    }
}

fn object_path(stage: CacheStage, hash: &str) -> String {
    format!("cache/{}/objects/{hash}.json", stage.name())
}
