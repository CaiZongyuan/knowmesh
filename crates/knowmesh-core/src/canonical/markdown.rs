use std::{collections::BTreeMap, ops::Range, str::FromStr};

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use serde::{Serialize, de::DeserializeOwned};

use crate::{
    domain::{knowledge_error, unsupported_version},
    error::{AppError, AppResult, ErrorType},
};

#[derive(Debug)]
pub(super) struct MarkdownFile {
    pub original: String,
    pub header: Range<usize>,
    pub body_start: usize,
    pub newline: &'static str,
}

impl MarkdownFile {
    pub fn parse(text: &str) -> AppResult<Self> {
        if text.len() > 8 * 1024 * 1024 {
            return Err(knowledge_error(
                "DOCUMENT_TOO_LARGE",
                "Canonical Markdown files must not exceed 8 MiB.",
            ));
        }
        if text.lines().any(|line| {
            line.starts_with("<<<<<<< ") || line.starts_with(">>>>>>> ") || line == "======="
        }) {
            return Err(AppError::new(
                ErrorType::Conflict,
                "CANONICAL_FILE_CONFLICT",
                "Resolve Git conflict markers before parsing canonical Markdown.",
            ));
        }
        let bom_size = if text.starts_with('\u{feff}') {
            '\u{feff}'.len_utf8()
        } else {
            0
        };
        let mut lines = text[bom_size..].split_inclusive('\n');
        let opening = lines.next().ok_or_else(invalid_frontmatter)?;
        if opening.trim_end_matches(['\r', '\n']) != "---" || !opening.ends_with('\n') {
            return Err(invalid_frontmatter());
        }
        let start = bom_size + opening.len();
        let mut end = start;
        for line in lines {
            if line.trim_end_matches(['\r', '\n']) == "---" {
                return Ok(Self {
                    original: text.to_owned(),
                    header: start..end,
                    body_start: end + line.len(),
                    newline: if opening.ends_with("\r\n") {
                        "\r\n"
                    } else {
                        "\n"
                    },
                });
            }
            end += line.len();
        }
        Err(invalid_frontmatter())
    }

    pub fn metadata<T: DeserializeOwned>(&self) -> AppResult<T> {
        let text = &self.original[self.header.clone()];
        let value: serde_yaml::Value =
            serde_yaml::from_str(text).map_err(|_| invalid_frontmatter())?;
        if value["version"].as_u64() != Some(1) {
            return Err(unsupported_version());
        }
        serde_yaml::from_str(text).map_err(|_| invalid_frontmatter())
    }

    pub fn body(&self) -> &str {
        &self.original[self.body_start..]
    }

    pub fn render_metadata<T: Serialize + DeserializeOwned>(
        &self,
        before: &T,
        after: &T,
    ) -> AppResult<String> {
        let previous: BTreeMap<String, serde_json::Value> = serde_json::from_value(
            serde_json::to_value(before).map_err(|_| invalid_frontmatter())?,
        )
        .map_err(|_| invalid_frontmatter())?;
        let next: BTreeMap<String, serde_json::Value> =
            serde_json::from_value(serde_json::to_value(after).map_err(|_| invalid_frontmatter())?)
                .map_err(|_| invalid_frontmatter())?;
        if previous == next {
            return Ok(self.original[self.header.clone()].to_owned());
        }
        let document = yaml_edit::YamlFile::from_str(&self.original[self.header.clone()])
            .map_err(|_| invalid_frontmatter())?;
        let mapping = document
            .document()
            .and_then(|doc| doc.as_mapping())
            .ok_or_else(invalid_frontmatter)?;
        let generated_yaml = serde_yaml::to_string(after).map_err(|_| invalid_frontmatter())?;
        let generated =
            yaml_edit::Document::from_str(&generated_yaml).map_err(|_| invalid_frontmatter())?;
        let replacement = generated.as_mapping().ok_or_else(invalid_frontmatter)?;
        for key in previous.keys().filter(|key| !next.contains_key(*key)) {
            mapping.remove(key.as_str());
        }
        for (key, value) in &next {
            if previous.get(key) != Some(value) {
                mapping.set(
                    key.as_str(),
                    replacement
                        .get(key.as_str())
                        .ok_or_else(invalid_frontmatter)?,
                );
            }
        }
        let text = document.to_string();
        let text = if self.newline == "\r\n" {
            text.replace("\r\n", "\n").replace('\n', "\r\n")
        } else {
            text
        };
        let actual: T = serde_yaml::from_str(&text).map_err(|_| invalid_frontmatter())?;
        let actual: BTreeMap<String, serde_json::Value> = serde_json::from_value(
            serde_json::to_value(actual).map_err(|_| invalid_frontmatter())?,
        )
        .map_err(|_| invalid_frontmatter())?;
        // The lossless editor must agree with the typed serializer before writing.
        if actual != next {
            return Err(knowledge_error(
                "FRONTMATTER_EDIT_FAILED",
                "The edited YAML does not match the intended metadata.",
            ));
        }
        Ok(text)
    }

    pub fn render(&self, mut replacements: Vec<(Range<usize>, String)>) -> AppResult<String> {
        replacements.sort_by_key(|a| a.0.start);
        let mut output = String::new();
        let mut cursor = 0;
        for (range, text) in replacements {
            if range.start < cursor || range.end > self.original.len() {
                return Err(invalid_managed());
            }
            output.push_str(&self.original[cursor..range.start]);
            output.push_str(&text);
            cursor = range.end;
        }
        output.push_str(&self.original[cursor..]);
        Ok(output)
    }
}

pub(super) fn markdown_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_WIKILINKS
}

pub(super) fn managed_ranges(
    file: &MarkdownFile,
) -> AppResult<BTreeMap<&'static str, Range<usize>>> {
    let markers = [
        ("<!-- knowmesh:claims:begin -->", "claims_begin"),
        ("<!-- knowmesh:claims:end -->", "claims_end"),
        ("<!-- knowmesh:relations:begin -->", "relations_begin"),
        ("<!-- knowmesh:relations:end -->", "relations_end"),
    ];
    let mut positions = BTreeMap::new();
    for (event, span) in Parser::new_ext(file.body(), markdown_options()).into_offset_iter() {
        if let Event::Html(html) = event {
            for (marker, name) in markers {
                if html.trim() == marker
                    && positions
                        .insert(
                            name,
                            span.start + file.body_start..span.end + file.body_start,
                        )
                        .is_some()
                {
                    return Err(invalid_managed());
                }
            }
        }
    }
    let mut ranges = BTreeMap::new();
    for (name, start, end) in [
        ("claims", "claims_begin", "claims_end"),
        ("relations", "relations_begin", "relations_end"),
    ] {
        let start = positions.get(start).ok_or_else(invalid_managed)?;
        let end = positions.get(end).ok_or_else(invalid_managed)?;
        if start.end > end.start {
            return Err(invalid_managed());
        }
        ranges.insert(name, start.end..end.start);
    }
    let claims = &ranges["claims"];
    let relations = &ranges["relations"];
    if claims.start < relations.end && relations.start < claims.end {
        return Err(invalid_managed());
    }
    Ok(ranges)
}

pub(super) fn managed_yaml<T: DeserializeOwned>(text: &str) -> AppResult<Vec<T>> {
    let mut code_blocks = 0;
    let mut inside = false;
    let mut yaml = String::new();
    for event in Parser::new_ext(text, markdown_options()) {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(language)))
                if language.as_ref() == "yaml" && !inside =>
            {
                inside = true;
                code_blocks += 1;
            }
            Event::Text(text) if inside => yaml.push_str(&text),
            Event::End(TagEnd::CodeBlock) if inside => inside = false,
            _ => return Err(invalid_managed()),
        }
    }
    if code_blocks != 1 || inside {
        return Err(invalid_managed());
    }
    let value: serde_yaml::Value = serde_yaml::from_str(&yaml).map_err(|_| invalid_managed())?;
    if value["version"].as_u64() != Some(1) {
        return Err(unsupported_version());
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Block<T> {
        version: u32,
        items: Vec<T>,
    }
    let block: Block<T> = serde_yaml::from_str(&yaml).map_err(|_| invalid_managed())?;
    if block.version != 1 || block.items.len() > 10_000 {
        return Err(invalid_managed());
    }
    Ok(block.items)
}

pub(super) fn render_managed<T: Serialize>(items: &[T], newline: &str) -> AppResult<String> {
    #[derive(Serialize)]
    struct Block<'a, T> {
        version: u32,
        items: &'a [T],
    }
    let yaml =
        serde_yaml::to_string(&Block { version: 1, items }).map_err(|_| invalid_managed())?;
    let longest_run = yaml.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(3.max(longest_run + 1));
    Ok(format!("{fence}yaml\n{yaml}{fence}\n").replace('\n', newline))
}

pub(super) fn code_ranges(text: &str) -> Vec<Range<usize>> {
    Parser::new_ext(text, markdown_options())
        .into_offset_iter()
        .filter_map(|(event, span)| match event {
            Event::Start(Tag::CodeBlock(_)) | Event::Code(_) | Event::Html(_) => Some(span),
            _ => None,
        })
        .collect()
}

pub(super) fn invalid_managed() -> AppError {
    knowledge_error(
        "INVALID_MANAGED_BLOCK",
        "Expected one complete claims block and one complete relations block, each containing versioned fenced YAML.",
    )
}
fn invalid_frontmatter() -> AppError {
    knowledge_error(
        "INVALID_FRONTMATTER",
        "Expected valid versioned YAML frontmatter at the beginning of the document.",
    )
}
