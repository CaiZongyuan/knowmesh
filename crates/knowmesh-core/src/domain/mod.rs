pub mod freshness;
mod ids;
mod knowledge;
mod source;
mod text_encoding;
mod timestamp;

pub use ids::*;
pub use knowledge::*;
pub use source::*;
pub use text_encoding::*;
pub use timestamp::Timestamp;

use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

pub fn normalize_name(value: &str) -> String {
    value
        .nfkc()
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
