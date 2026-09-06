use std::ops::Range;

use super::{BlockKind, ByteSpan, ParseWarning, limit_error};
use crate::error::AppResult;

pub(super) struct Draft {
    pub kind: BlockKind,
    pub text: String,
    pub source_bytes: Option<ByteSpan>,
    pub section_path: Vec<String>,
    pub heading_level: Option<u8>,
    pub language: Option<String>,
    pub caption: Option<String>,
    pub preserve_whitespace: bool,
    pub page: Option<u32>,
}

impl Draft {
    pub fn new(kind: BlockKind, span: Option<Range<usize>>) -> Self {
        Self {
            kind,
            text: String::new(),
            source_bytes: span.map(|range| ByteSpan {
                start: range.start,
                end: range.end,
            }),
            section_path: vec![],
            heading_level: None,
            language: None,
            caption: None,
            preserve_whitespace: matches!(kind, BlockKind::Code | BlockKind::Table),
            page: None,
        }
    }
}

#[derive(Default)]
struct Table {
    rows: Vec<Vec<String>>,
    row: Vec<String>,
    cell: Option<String>,
    caption: String,
    in_caption: bool,
    source_bytes: Option<Range<usize>>,
    depth: usize,
}

impl Table {
    fn finish_cell(&mut self) {
        if let Some(cell) = self.cell.take() {
            self.row.push(collapse(&cell));
        }
    }
    fn finish_row(&mut self) {
        self.finish_cell();
        if !self.row.is_empty() {
            self.rows.push(std::mem::take(&mut self.row));
        }
    }
}

pub(super) struct Builder {
    pub blocks: Vec<Draft>,
    pub warnings: Vec<ParseWarning>,
    current: Option<Draft>,
    sections: Vec<(u8, String)>,
    table: Option<Table>,
    max_blocks: usize,
    pub list_depth: usize,
    pub quote_depth: usize,
}

impl Builder {
    pub fn new(max_blocks: usize) -> Self {
        Self {
            blocks: vec![],
            warnings: vec![],
            current: None,
            sections: vec![],
            table: None,
            max_blocks,
            list_depth: 0,
            quote_depth: 0,
        }
    }

    pub fn default_kind(&self) -> BlockKind {
        if self.list_depth > 0 {
            BlockKind::ListItem
        } else if self.quote_depth > 0 {
            BlockKind::Quote
        } else {
            BlockKind::Paragraph
        }
    }

    pub fn in_table(&self) -> bool {
        self.table.is_some()
    }
    pub fn in_code(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|draft| draft.kind == BlockKind::Code)
    }

    pub fn start(
        &mut self,
        kind: BlockKind,
        span: Option<Range<usize>>,
        heading_level: Option<u8>,
    ) -> AppResult<()> {
        if self.in_table() {
            return Ok(());
        }
        self.finish(span.as_ref().map(|range| range.start))?;
        let mut draft = Draft::new(kind, span);
        draft.heading_level = heading_level;
        self.current = Some(draft);
        Ok(())
    }

    pub fn set_language(&mut self, language: &str) {
        if let Some(draft) = &mut self.current {
            draft.language = language.split_whitespace().next().map(str::to_owned);
        }
    }

    pub fn text(&mut self, text: &str, span: Option<Range<usize>>) -> AppResult<()> {
        if let Some(table) = &mut self.table {
            if table.in_caption {
                table.caption.push_str(text);
            } else if let Some(cell) = &mut table.cell {
                cell.push_str(text);
            }
            return Ok(());
        }
        if self.current.is_none() {
            if text.trim().is_empty() {
                return Ok(());
            }
            self.current = Some(Draft::new(self.default_kind(), span.clone()));
        }
        if let Some(draft) = &mut self.current {
            draft.text.push_str(text);
            if let (Some(existing), Some(span)) = (&mut draft.source_bytes, span) {
                existing.end = existing.end.max(span.end);
            }
        }
        Ok(())
    }

    pub fn finish(&mut self, end: Option<usize>) -> AppResult<()> {
        if self.in_table() {
            return Ok(());
        }
        if let Some(mut draft) = self.current.take() {
            if let (Some(span), Some(end)) = (&mut draft.source_bytes, end)
                && end >= span.start
            {
                span.end = end;
            }
            self.push(draft)?;
        }
        Ok(())
    }

    pub fn push(&mut self, mut draft: Draft) -> AppResult<()> {
        draft.text = if draft.preserve_whitespace {
            draft
                .text
                .replace("\r\n", "\n")
                .trim_matches('\n')
                .to_owned()
        } else {
            draft
                .text
                .lines()
                .map(collapse)
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_owned()
        };
        if draft.text.trim().is_empty() {
            return Ok(());
        }
        if self.blocks.len() >= self.max_blocks {
            return Err(limit_error());
        }
        if let Some(level) = draft.heading_level {
            while self
                .sections
                .last()
                .is_some_and(|(previous, _)| *previous >= level)
            {
                self.sections.pop();
            }
            self.sections.push((level, draft.text.clone()));
        }
        draft.section_path = self
            .sections
            .iter()
            .map(|(_, title)| title.clone())
            .collect();
        if draft.kind == BlockKind::FigureCaption {
            draft.caption = Some(draft.text.clone());
        }
        self.blocks.push(draft);
        Ok(())
    }

    pub fn start_table(&mut self, span: Option<Range<usize>>) -> AppResult<()> {
        if let Some(table) = &mut self.table {
            table.depth += 1;
            self.warnings.push(ParseWarning {
                code: "NESTED_TABLE_FLATTENED".into(),
                hint: "Inspect the original nested table when reviewing extracted assertions."
                    .into(),
            });
            return Ok(());
        }
        self.finish(span.as_ref().map(|range| range.start))?;
        self.table = Some(Table {
            source_bytes: span,
            depth: 1,
            ..Default::default()
        });
        Ok(())
    }

    pub fn row(&mut self) {
        if let Some(table) = &mut self.table {
            table.finish_row();
        }
    }
    pub fn cell(&mut self) {
        if let Some(table) = &mut self.table {
            table.finish_cell();
            table.cell = Some(String::new());
        }
    }
    pub fn end_cell(&mut self) {
        if let Some(table) = &mut self.table {
            table.finish_cell();
        }
    }
    pub fn caption(&mut self, active: bool) {
        if let Some(table) = &mut self.table {
            table.in_caption = active;
        }
    }

    pub fn end_table(&mut self, end: Option<usize>) -> AppResult<()> {
        let Some(mut table) = self.table.take() else {
            return Ok(());
        };
        if table.depth > 1 {
            table.depth -= 1;
            self.table = Some(table);
            return Ok(());
        }
        table.finish_row();
        let mut draft = Draft::new(BlockKind::Table, table.source_bytes);
        if let (Some(span), Some(end)) = (&mut draft.source_bytes, end) {
            span.end = end;
        }
        let caption = collapse(&table.caption);
        if !caption.is_empty() {
            draft.caption = Some(caption.clone());
            draft.text.push_str(&caption);
            draft.text.push('\n');
        }
        draft.text.push_str(
            &table
                .rows
                .iter()
                .map(|row| row.join("\t"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        self.push(draft)
    }
}

pub(super) fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
