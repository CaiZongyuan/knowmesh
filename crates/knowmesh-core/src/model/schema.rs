use serde_json::Value;

use crate::error::{AppError, AppResult, ErrorType};

struct LocalReferencesOnly;
impl jsonschema::Retrieve for LocalReferencesOnly {
    fn retrieve(
        &self,
        _: &jsonschema::Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err("External JSON Schema retrieval is disabled.".into())
    }
}

pub(super) fn compile(schema: &Value) -> AppResult<jsonschema::Validator> {
    jsonschema::options()
        .with_retriever(LocalReferencesOnly)
        .build(schema)
        .map_err(|_| {
            AppError::new(
                ErrorType::Validation,
                "MODEL_SCHEMA_INVALID",
                "The model schema is invalid or references an external document.",
            )
        })
}
