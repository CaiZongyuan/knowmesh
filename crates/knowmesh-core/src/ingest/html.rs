use scraper::{Html, Node, Selector};

use super::{
    BlockKind, ParsedMetadata,
    builder::{Builder, collapse},
};
use crate::error::AppResult;

pub(super) fn parse(source: &str, builder: &mut Builder) -> AppResult<ParsedMetadata> {
    let document = Html::parse_document(source);
    let title = Selector::parse("head > title").expect("constant title selector");
    let language = Selector::parse("html").expect("constant language selector");
    let metadata = ParsedMetadata {
        title: document
            .select(&title)
            .next()
            .map(|element| collapse(&element.text().collect::<String>()))
            .filter(|title| !title.is_empty()),
        language: document
            .select(&language)
            .next()
            .and_then(|element| element.attr("lang"))
            .map(str::to_owned),
        ..Default::default()
    };
    let mut stack = vec![(document.tree.root(), false)];
    while let Some((node, closing)) = stack.pop() {
        match node.value() {
            Node::Element(element) => {
                let name = element.name();
                if matches!(
                    name,
                    "head" | "script" | "style" | "template" | "noscript" | "svg"
                ) || element.attr("hidden").is_some()
                    || element
                        .attr("aria-hidden")
                        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
                {
                    continue;
                }
                if closing {
                    close(name, builder)?;
                } else {
                    open(name, builder)?;
                    stack.push((node, true));
                    stack.extend(node.children().rev().map(|child| (child, false)));
                }
            }
            Node::Text(text) if !closing => {
                if builder.in_code() {
                    builder.text(&text.text, None)?;
                } else {
                    let text: String = text
                        .text
                        .chars()
                        .map(|ch| if ch.is_whitespace() { ' ' } else { ch })
                        .collect();
                    builder.text(&text, None)?;
                }
            }
            Node::Document | Node::Fragment => {
                stack.extend(node.children().rev().map(|child| (child, false)))
            }
            _ => {}
        }
    }
    builder.finish(None)?;
    Ok(metadata)
}

fn open(name: &str, builder: &mut Builder) -> AppResult<()> {
    match name {
        "table" => return builder.start_table(None),
        "tr" => {
            builder.row();
            return Ok(());
        }
        "th" | "td" => {
            builder.cell();
            return Ok(());
        }
        "caption" => {
            builder.caption(true);
            return Ok(());
        }
        "br" => return builder.text("\n", None),
        _ => {}
    }
    if builder.in_table() || builder.in_code() {
        return Ok(());
    }
    match name {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            builder.start(BlockKind::Heading, None, Some(name.as_bytes()[1] - b'0'))?
        }
        "p" => builder.start(builder.default_kind(), None, None)?,
        "li" => {
            builder.list_depth += 1;
            builder.start(BlockKind::ListItem, None, None)?;
        }
        "blockquote" => {
            builder.finish(None)?;
            builder.quote_depth += 1;
        }
        "pre" => builder.start(BlockKind::Code, None, None)?,
        "figcaption" => builder.start(BlockKind::FigureCaption, None, None)?,
        "div" | "section" | "article" | "main" | "header" | "footer" | "hr" | "ul" | "ol" => {
            builder.finish(None)?
        }
        _ => {}
    }
    Ok(())
}

fn close(name: &str, builder: &mut Builder) -> AppResult<()> {
    match name {
        "table" => return builder.end_table(None),
        "tr" => {
            builder.row();
            return Ok(());
        }
        "th" | "td" => {
            builder.end_cell();
            return Ok(());
        }
        "caption" => {
            builder.caption(false);
            return Ok(());
        }
        "pre" => return builder.finish(None),
        _ => {}
    }
    if builder.in_table() || builder.in_code() {
        return Ok(());
    }
    match name {
        "li" => {
            builder.finish(None)?;
            builder.list_depth = builder.list_depth.saturating_sub(1);
        }
        "blockquote" => {
            builder.finish(None)?;
            builder.quote_depth = builder.quote_depth.saturating_sub(1);
        }
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "figcaption" | "div" | "section"
        | "article" | "main" | "header" | "footer" => builder.finish(None)?,
        _ => {}
    }
    Ok(())
}
