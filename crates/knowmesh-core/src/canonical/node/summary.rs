use std::{borrow::Cow, ops::Range};

use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

use super::{NodeDocument, markdown_options};
use crate::{domain::knowledge_error, error::AppResult};

struct Heading {
    start: usize,
    end: usize,
    level: HeadingLevel,
    title: String,
}
struct Sections {
    summaries: Vec<Range<usize>>,
    first_managed: Option<usize>,
    definitions: Vec<Range<usize>>,
}

impl NodeDocument {
    pub fn set_summary(&mut self, value: &str) -> AppResult<bool> {
        validate(value)?;
        let sections = sections(self.file.body());
        if sections.summaries.len() > 1 {
            return Err(knowledge_error(
                "AMBIGUOUS_NODE_SUMMARY",
                "The Node has more than one top-level Summary section.",
            ));
        }
        let normalized = normalize(value);
        let newline = self.file.newline;
        let content = normalized.replace('\n', newline);
        let change = if let Some(range) = sections.summaries.first() {
            if normalize(&self.file.body()[range.clone()]) == normalized {
                None
            } else {
                let mut replacement = if content.is_empty() {
                    newline.into()
                } else {
                    format!("{newline}{content}{newline}{newline}")
                };
                for definition in &sections.definitions {
                    if definition.start >= range.start && definition.end <= range.end {
                        replacement.push_str(&self.file.body()[definition.clone()]);
                        if !replacement.ends_with(newline) {
                            replacement.push_str(newline);
                        }
                        replacement.push_str(newline);
                    }
                }
                Some((
                    range.start + self.file.body_start..range.end + self.file.body_start,
                    replacement,
                ))
            }
        } else if content.is_empty() {
            None
        } else {
            let position =
                sections.first_managed.unwrap_or(self.file.body().len()) + self.file.body_start;
            let separator =
                if self.file.original[..position].ends_with(&format!("{newline}{newline}")) {
                    ""
                } else {
                    newline
                };
            Some((
                position..position,
                format!("{separator}## Summary{newline}{newline}{content}{newline}{newline}"),
            ))
        };
        self.summary_edit =
            change.filter(|(range, replacement)| &self.file.original[range.clone()] != replacement);
        Ok(self.summary_edit.is_some())
    }

    pub fn summary(&self) -> String {
        let body = if let Some((range, replacement)) = &self.summary_edit {
            let original = self.file.body();
            Cow::Owned(format!(
                "{}{}{}",
                &original[..range.start - self.file.body_start],
                replacement,
                &original[range.end - self.file.body_start..]
            ))
        } else {
            Cow::Borrowed(self.file.body())
        };
        sections(&body)
            .summaries
            .first()
            .map_or_else(String::new, |range| plain_text(&body, range))
    }
}

fn sections(body: &str) -> Sections {
    let parser = Parser::new_ext(body, markdown_options());
    let mut definitions: Vec<_> = parser
        .reference_definitions()
        .iter()
        .map(|(_, definition)| definition.span.clone())
        .collect();
    definitions.sort_by_key(|range| range.start);
    let mut depth = 0usize;
    let mut headings = vec![];
    let mut heading: Option<Heading> = None;
    let mut html_boundaries = vec![];
    let mut first_managed = None;
    for (event, span) in parser.into_offset_iter() {
        match event {
            Event::Start(tag) => {
                if depth == 0
                    && let Tag::Heading { level, .. } = &tag
                    && matches!(level, HeadingLevel::H1 | HeadingLevel::H2)
                {
                    heading = Some(Heading {
                        start: span.start,
                        end: span.end,
                        level: *level,
                        title: String::new(),
                    });
                }
                depth += 1;
            }
            Event::End(tag) => {
                depth = depth.saturating_sub(1);
                if depth == 0
                    && matches!(tag, TagEnd::Heading(_))
                    && let Some(heading) = heading.take()
                {
                    headings.push(heading);
                }
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some(heading) = &mut heading {
                    heading.title.push_str(&text);
                }
            }
            Event::Html(html) => {
                html_boundaries.push(span.start);
                if matches!(
                    html.trim(),
                    "<!-- knowmesh:claims:begin -->" | "<!-- knowmesh:relations:begin -->"
                ) && first_managed.is_none()
                {
                    first_managed = Some(span.start);
                }
            }
            _ => {}
        }
    }
    let mut summaries = vec![];
    let prefer_h2 = headings.iter().any(|heading| {
        heading.level == HeadingLevel::H2 && heading.title.eq_ignore_ascii_case("Summary")
    });
    for (index, heading) in headings.iter().enumerate() {
        if heading.title.eq_ignore_ascii_case("Summary")
            && (!prefer_h2 || heading.level == HeadingLevel::H2)
        {
            let next = headings
                .get(index + 1)
                .map_or(body.len(), |heading| heading.start);
            let end = html_boundaries
                .iter()
                .copied()
                .find(|start| *start >= heading.end)
                .unwrap_or(body.len())
                .min(next);
            summaries.push(heading.end..end);
        }
    }
    Sections {
        summaries,
        first_managed,
        definitions,
    }
}

fn validate(value: &str) -> AppResult<()> {
    if value.len() > 64 * 1024
        || value.contains('\0')
        || value.lines().any(|line| {
            line.starts_with("<<<<<<< ") || line.starts_with(">>>>>>> ") || line == "======="
        })
    {
        return Err(invalid());
    }
    let parser = Parser::new_ext(value, markdown_options());
    if parser.reference_definitions().iter().next().is_some() {
        return Err(invalid());
    }
    let mut depth = 0usize;
    for event in parser {
        match event {
            Event::Start(tag) => {
                if depth == 0
                    && matches!(
                        tag,
                        Tag::Heading {
                            level: HeadingLevel::H1 | HeadingLevel::H2,
                            ..
                        }
                    )
                {
                    return Err(invalid());
                }
                depth += 1;
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            Event::Html(_) | Event::InlineHtml(_) => return Err(invalid()),
            _ => {}
        }
    }
    Ok(())
}

fn normalize(value: &str) -> String {
    if value.trim().is_empty() {
        String::new()
    } else {
        value.replace("\r\n", "\n").trim_matches('\n').to_owned()
    }
}

fn plain_text(value: &str, range: &Range<usize>) -> String {
    let mut output = String::new();
    for (event, span) in Parser::new_ext(value, markdown_options()).into_offset_iter() {
        if span.start < range.start || span.end > range.end {
            continue;
        }
        match event {
            Event::Text(text) | Event::Code(text) => output.push_str(&text),
            Event::SoftBreak
            | Event::HardBreak
            | Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::CodeBlock) => {
                output.push('\n')
            }
            _ => {}
        }
    }
    output.trim().to_owned()
}

fn invalid() -> crate::error::AppError {
    knowledge_error(
        "INVALID_NODE_SUMMARY",
        "Summary Markdown must be bounded and cannot inject top-level sections, HTML, reference definitions, or conflict markers.",
    )
}
