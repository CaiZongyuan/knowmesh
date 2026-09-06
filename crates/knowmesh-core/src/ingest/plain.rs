use super::{
    BlockKind, ByteSpan,
    builder::{Builder, Draft},
};
use crate::error::AppResult;

pub(super) fn parse(source: &str, builder: &mut Builder) -> AppResult<()> {
    let mut draft: Option<Draft> = None;
    let mut offset = 0;
    for raw in source.split_inclusive('\n') {
        let line = raw.trim_end_matches(['\r', '\n']);
        if line.trim().is_empty() {
            if let Some(block) = draft.take() {
                builder.push(block)?;
            }
        } else {
            let block = draft.get_or_insert_with(|| {
                let mut block = Draft::new(BlockKind::Paragraph, Some(offset..offset));
                block.preserve_whitespace = true;
                block
            });
            if !block.text.is_empty() {
                block.text.push('\n');
            }
            block.text.push_str(line);
            block.source_bytes = Some(ByteSpan {
                start: block
                    .source_bytes
                    .as_ref()
                    .map_or(offset, |span| span.start),
                end: offset + line.len(),
            });
        }
        offset += raw.len();
    }
    if let Some(block) = draft {
        builder.push(block)?;
    }
    Ok(())
}
