use std::ops::Range;

const CHECKPOINT_STRIDE: usize = 1024;

pub(super) struct CharacterIndex<'a> {
    text: &'a str,
    checkpoints: Vec<usize>,
    pub len: usize,
}

impl<'a> CharacterIndex<'a> {
    pub fn new(text: &'a str) -> Self {
        let mut checkpoints = vec![];
        let mut len = 0;
        for (position, (byte, _)) in text.char_indices().enumerate() {
            if position % CHECKPOINT_STRIDE == 0 {
                checkpoints.push(byte);
            }
            len = position + 1;
        }
        if len % CHECKPOINT_STRIDE == 0 {
            checkpoints.push(text.len());
        }
        Self {
            text,
            checkpoints,
            len,
        }
    }

    pub fn slice(&self, span: Range<usize>) -> &'a str {
        &self.text[self.byte_offset(span.start)..self.byte_offset(span.end)]
    }

    fn byte_offset(&self, position: usize) -> usize {
        let base = self.checkpoints[position / CHECKPOINT_STRIDE];
        base + self.text[base..]
            .char_indices()
            .nth(position % CHECKPOINT_STRIDE)
            .map_or(self.text.len() - base, |(byte, _)| byte)
    }
}

pub(super) fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) struct QuotePattern {
    chars: Vec<char>,
    prefix: Vec<usize>,
}

impl QuotePattern {
    pub fn new(quote: &str) -> Self {
        let chars: Vec<_> = quote.chars().collect();
        let mut prefix = vec![0; chars.len()];
        let mut matched = 0;
        for index in 1..chars.len() {
            while matched > 0 && chars[index] != chars[matched] {
                matched = prefix[matched - 1];
            }
            if chars[index] == chars[matched] {
                matched += 1;
            }
            prefix[index] = matched;
        }
        Self { chars, prefix }
    }

    // KMP counts overlapping occurrences and retains the original Unicode spans.
    pub fn find(&self, text: &str, start: usize) -> Vec<Range<usize>> {
        let mut normalized: Vec<(char, Range<usize>)> = vec![];
        for (offset, ch) in text.chars().enumerate() {
            let position = start + offset;
            if ch.is_whitespace() {
                if let Some((' ', span)) = normalized.last_mut() {
                    span.end = position + 1;
                } else {
                    normalized.push((' ', position..position + 1));
                }
            } else {
                normalized.push((ch, position..position + 1));
            }
        }
        let mut found = vec![];
        let mut matched = 0;
        for (index, (ch, span)) in normalized.iter().enumerate() {
            while matched > 0 && *ch != self.chars[matched] {
                matched = self.prefix[matched - 1];
            }
            if *ch == self.chars[matched] {
                matched += 1;
            }
            if matched == self.chars.len() {
                found.push(normalized[index + 1 - matched].1.start..span.end);
                if found.len() == 2 {
                    break;
                }
                matched = self.prefix[matched - 1];
            }
        }
        found
    }
}
