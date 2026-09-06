use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use super::{BlockKind, ParsedMetadata, builder::Builder, frontmatter};
use crate::error::AppResult;

pub(super) fn parse(source: &str, builder: &mut Builder) -> AppResult<ParsedMetadata> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS;
    let mut html_block: Option<String> = None;
    let mut metadata_block: Option<String> = None;
    let mut metadata = ParsedMetadata::default();
    for (event, span) in Parser::new_ext(source, options).into_offset_iter() {
        if let (Some(buffer), Event::Text(text)) = (&mut metadata_block, &event) {
            if buffer.len().saturating_add(text.len()) > frontmatter::MAX_BYTES {
                return Err(super::limit_error());
            }
            buffer.push_str(text);
            continue;
        }
        match event {
            Event::Start(Tag::MetadataBlock(_)) => metadata_block = Some(String::new()),
            Event::End(TagEnd::MetadataBlock(_)) => {
                metadata = frontmatter::parse(&metadata_block.take().unwrap_or_default())?
            }
            Event::Start(Tag::HtmlBlock) => {
                builder.finish(Some(span.start))?;
                html_block = Some(String::new());
            }
            Event::End(TagEnd::HtmlBlock) => {
                if let Some(html) = html_block.take() {
                    super::html::parse(&html, builder)?;
                }
            }
            Event::Start(Tag::Heading { level, .. }) => {
                builder.start(BlockKind::Heading, Some(span), Some(level as u8))?
            }
            Event::Start(Tag::Paragraph) => {
                builder.start(builder.default_kind(), Some(span), None)?
            }
            Event::Start(Tag::Item) => {
                builder.list_depth += 1;
                builder.start(BlockKind::ListItem, Some(span), None)?;
            }
            Event::End(TagEnd::Item) => {
                builder.finish(Some(span.end))?;
                builder.list_depth = builder.list_depth.saturating_sub(1);
            }
            Event::Start(Tag::BlockQuote(_)) => {
                builder.finish(Some(span.start))?;
                builder.quote_depth += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                builder.finish(Some(span.end))?;
                builder.quote_depth = builder.quote_depth.saturating_sub(1);
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                builder.start(BlockKind::Code, Some(span), None)?;
                if let CodeBlockKind::Fenced(language) = kind {
                    builder.set_language(&language);
                }
            }
            Event::Start(Tag::Table(_)) => builder.start_table(Some(span))?,
            Event::Start(Tag::TableHead | Tag::TableRow)
            | Event::End(TagEnd::TableHead | TagEnd::TableRow) => builder.row(),
            Event::Start(Tag::TableCell) => builder.cell(),
            Event::End(TagEnd::TableCell) => builder.end_cell(),
            Event::End(TagEnd::Table) => builder.end_table(Some(span.end))?,
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::CodeBlock) => {
                builder.finish(Some(span.end))?
            }
            Event::Text(text)
            | Event::Code(text)
            | Event::InlineMath(text)
            | Event::DisplayMath(text) => builder.text(&text, Some(span))?,
            Event::SoftBreak => builder.text(" ", Some(span))?,
            Event::HardBreak => builder.text("\n", Some(span))?,
            Event::Rule => builder.finish(Some(span.end))?,
            Event::Html(html) => {
                if let Some(block) = &mut html_block {
                    block.push_str(&html);
                } else {
                    builder.finish(Some(span.start))?;
                    super::html::parse(&html, builder)?;
                }
            }
            Event::FootnoteReference(label) => builder.text(&format!("[^{label}]"), Some(span))?,
            Event::TaskListMarker(checked) => {
                builder.text(if checked { "[x] " } else { "[ ] " }, Some(span))?
            }
            _ => {}
        }
    }
    builder.finish(Some(source.len()))?;
    Ok(metadata)
}
