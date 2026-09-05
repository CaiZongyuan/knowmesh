mod ids;
mod timestamp;

pub use ids::*;
pub use timestamp::Timestamp;

use sha2::{Digest, Sha256};

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
