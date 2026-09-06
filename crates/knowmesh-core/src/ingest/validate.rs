use std::collections::BTreeSet;

use super::{BlockKind, ParseStatus, ParsedSource};
use crate::{
    domain::{SourceRevision, sha256, valid_sha256},
    error::{AppError, AppResult, ErrorType},
};

impl ParsedSource {
    pub fn validate(&self, revision: &SourceRevision) -> AppResult<()> {
        if self.version != 1
            || self.source_revision_id != revision.id
            || self.source_sha256 != revision.sha256
            || self.source_encoding != revision.encoding
            || !valid_sha256(&self.source_sha256)
            || self.text_sha256 != sha256(self.normalized_text.as_bytes())
            || self.parser_name.is_empty()
            || self.parser_version.is_empty()
            || !valid_sha256(&self.parser_config_sha256)
        {
            return Err(invalid());
        }
        let mut ids = BTreeSet::new();
        let mut char_position: usize = 0;
        let mut byte_position: usize = 0;
        for (index, block) in self.blocks.iter().enumerate() {
            if index > 0 {
                let end = byte_position.checked_add(2).ok_or_else(invalid)?;
                if self.normalized_text.get(byte_position..end) != Some("\n\n") {
                    return Err(invalid());
                }
                byte_position = end;
                char_position = char_position.checked_add(2).ok_or_else(invalid)?;
            }
            let char_end = char_position
                .checked_add(block.text.chars().count())
                .ok_or_else(invalid)?;
            let byte_end = byte_position
                .checked_add(block.text.len())
                .ok_or_else(invalid)?;
            if block.text.trim().is_empty()
                || !ids.insert(&block.id)
                || block.char_start != char_position
                || block.char_end != char_end
                || self.normalized_text.get(byte_position..byte_end) != Some(&block.text)
                || block.page == Some(0)
                || block.paragraph == Some(0)
                || block.section_path.iter().any(|name| name.trim().is_empty())
                || block.source_bytes.as_ref().is_some_and(|span| {
                    span.start >= span.end || span.end as u64 > revision.byte_size
                })
                || (block.kind == BlockKind::Heading) != block.heading_level.is_some()
                || block
                    .heading_level
                    .is_some_and(|level| !(1..=6).contains(&level))
                || block
                    .page
                    .zip(self.quality.page_count)
                    .is_some_and(|(page, count)| page > count)
            {
                return Err(invalid());
            }
            char_position = char_end;
            byte_position = byte_end;
        }
        let visible = self
            .normalized_text
            .chars()
            .filter(|ch| !ch.is_whitespace() && !ch.is_control())
            .count();
        let replacements = self
            .normalized_text
            .chars()
            .filter(|ch| *ch == '\u{fffd}')
            .count();
        if byte_position != self.normalized_text.len()
            || self.quality.visible_characters != visible
            || self.quality.replacement_characters != replacements
            || self.quality.suspicious_characters
                != self
                    .normalized_text
                    .chars()
                    .filter(|ch| super::suspicious(*ch))
                    .count()
            || !self.quality.suspicious_ratio.is_finite()
            || !(0.0..=1.0).contains(&self.quality.suspicious_ratio)
            || self.quality.usable_for_compile != (self.status == ParseStatus::Ready)
            || !self.quality.replacement_ratio.is_finite()
            || !(0.0..=1.0).contains(&self.quality.replacement_ratio)
            || (visible == 0 && self.quality.usable_for_compile)
            || self.quality.page_count == Some(0)
            || self
                .quality
                .text_pages
                .zip(self.quality.page_count)
                .is_some_and(|(pages, count)| pages > count)
            || (self.quality.page_map_reliable
                && (self.quality.page_count.is_none()
                    || self.blocks.iter().any(|block| block.page.is_none())))
        {
            return Err(invalid());
        }
        Ok(())
    }
}

fn invalid() -> AppError {
    AppError::new(
        ErrorType::Validation,
        "INVALID_PARSED_SOURCE",
        "The parsed source is inconsistent with its revision, text, or block spans.",
    )
}
