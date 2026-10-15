//! Wrap Settings values without mixing their source text with visual padding.

use std::{borrow::Cow, ops::Range};

use ratatui::{
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::render::{ThemeRole, themed_style};
use crate::application::ConfigurationDraft;

pub(super) struct ConfigurationText {
    pub lines: Vec<Line<'static>>,
    pub values: Vec<ValueRun>,
}

pub(super) struct ValueRun {
    pub line: usize,
    pub column: u16,
    pub field: usize,
    /// A source byte boundary for every terminal cell, including the end boundary.
    pub offsets: Vec<usize>,
}

struct Chunk {
    range: Range<usize>,
    text: String,
    offsets: Vec<usize>,
}

fn visible(grapheme: &str, width: usize) -> Cow<'_, str> {
    let text = if grapheme.chars().any(char::is_control) {
        Cow::Owned(
            grapheme
                .chars()
                .flat_map(char::escape_default)
                .collect::<String>(),
        )
    } else {
        Cow::Borrowed(grapheme)
    };
    if text.width() > width {
        Cow::Borrowed("�")
    } else {
        text
    }
}

fn wrap(text: &str, width: usize) -> Vec<Chunk> {
    if width == 0 {
        return Vec::new();
    }
    let graphemes: Vec<_> = text.grapheme_indices(true).collect();
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < graphemes.len() {
        let mut end = start;
        let mut cells = 0;
        let mut word_boundary = None;
        while end < graphemes.len() && graphemes[end].1 != "\n" && graphemes[end].1 != "\r\n" {
            let size = visible(graphemes[end].1, width).width();
            if cells + size > width && end > start {
                break;
            }
            cells += size;
            end += 1;
            if graphemes[end - 1].1.chars().all(char::is_whitespace)
                || matches!(graphemes[end - 1].1, "." | "/" | "_")
            {
                word_boundary = Some(end);
            }
        }
        let newline = end < graphemes.len() && matches!(graphemes[end].1, "\n" | "\r\n");
        if end < graphemes.len() && !newline {
            end = word_boundary
                .filter(|boundary| *boundary > start)
                .unwrap_or(end);
        }
        let byte_start = graphemes[start].0;
        let byte_end = graphemes.get(end).map_or(text.len(), |(byte, _)| *byte);
        let mut display = String::new();
        let mut offsets = vec![byte_start];
        for (index, &(byte, grapheme)) in graphemes[start..end].iter().enumerate() {
            let glyph = visible(grapheme, width);
            let next = graphemes
                .get(start + index + 1)
                .map_or(text.len(), |(byte, _)| *byte);
            offsets.extend(std::iter::repeat_n(next, glyph.width()));
            display.push_str(&glyph);
            debug_assert!(byte <= next);
        }
        chunks.push(Chunk {
            range: byte_start..byte_end,
            text: display,
            offsets,
        });
        start = if newline { end + 1 } else { end };
    }
    if text.is_empty() || text.ends_with('\n') {
        chunks.push(Chunk {
            range: text.len()..text.len(),
            text: String::new(),
            offsets: vec![text.len()],
        });
    }
    chunks
}

fn value_spans(
    draft: &ConfigurationDraft,
    field: usize,
    value: &str,
    range: Range<usize>,
    text: String,
    width: usize,
) -> Vec<Span<'static>> {
    let Some(selection) = draft
        .selection
        .as_ref()
        .filter(|selection| selection.field == field)
    else {
        return vec![Span::raw(text)];
    };
    let selected = selection.range(value.len());
    let start = selected.start.max(range.start).min(range.end);
    let end = selected.end.min(range.end).max(start);
    [
        (range.start..start, false),
        (start..end, true),
        (end..range.end, false),
    ]
    .into_iter()
    .filter(|(range, _)| !range.is_empty())
    .map(|(range, selected)| {
        let text = value[range]
            .graphemes(true)
            .map(|grapheme| visible(grapheme, width))
            .collect::<String>();
        Span::styled(
            text,
            if selected {
                super::render::selection_style()
            } else {
                ratatui::style::Style::default()
            },
        )
    })
    .collect()
}

pub(super) fn prepare(draft: &ConfigurationDraft, area: Rect) -> ConfigurationText {
    let mut result = ConfigurationText {
        lines: Vec::new(),
        values: Vec::new(),
    };
    let width = usize::from(area.width);
    let Some(actual) = &draft.actual else {
        return result;
    };
    if width == 0 {
        return result;
    }
    for error in [draft.validation_error(), draft.error.clone()]
        .into_iter()
        .flatten()
    {
        result.lines.extend(
            wrap(&error, width)
                .into_iter()
                .map(|chunk| Line::styled(chunk.text, themed_style(ThemeRole::Error))),
        );
    }
    let fields: Vec<_> = actual
        .fields
        .iter()
        .enumerate()
        .filter(|(_, field)| !field.editable())
        .collect();
    if !fields.is_empty() {
        if usize::from(area.height) > result.lines.len() + fields.len() + 3 {
            result.lines.push(Line::default());
        }
        let heading = "── Read-only settings ";
        let rule = "─".repeat(width.saturating_sub(heading.width()));
        for chunk in wrap(&format!("{heading}{rule}"), width) {
            result.lines.push(Line::styled(
                chunk.text,
                themed_style(ThemeRole::Primary).add_modifier(Modifier::BOLD),
            ));
        }
        let stacked = width < 50;
        let label_width = fields
            .iter()
            .map(|(_, field)| field.label.width() + 1)
            .max()
            .unwrap_or(0)
            .min(width / 2);
        let value_column = if stacked {
            2.min(width.saturating_sub(1))
        } else {
            label_width + 2
        };
        let value_width = width.saturating_sub(value_column).max(1);
        for (index, field) in fields {
            let label = format!("{}:", field.label);
            let labels = wrap(&label, if stacked { width } else { label_width.max(1) });
            if stacked {
                result.lines.extend(
                    labels.iter().map(|chunk| {
                        Line::styled(chunk.text.clone(), themed_style(ThemeRole::Muted))
                    }),
                );
            }
            let values = wrap(&field.value, value_width);
            let rows = if stacked {
                values.len()
            } else {
                values.len().max(labels.len())
            };
            let mut values = values.into_iter();
            for row in 0..rows {
                let label = if stacked {
                    ""
                } else {
                    labels.get(row).map_or("", |chunk| chunk.text.as_str())
                };
                let mut spans = vec![
                    Span::styled(label.to_owned(), themed_style(ThemeRole::Muted)),
                    Span::raw(" ".repeat(value_column.saturating_sub(label.width()))),
                ];
                if let Some(chunk) = values.next() {
                    spans.extend(value_spans(
                        draft,
                        index,
                        &field.value,
                        chunk.range,
                        chunk.text,
                        value_width,
                    ));
                    result.values.push(ValueRun {
                        line: result.lines.len(),
                        column: value_column as u16,
                        field: index,
                        offsets: chunk.offsets,
                    });
                }
                result.lines.push(Line::from(spans));
            }
            if stacked || rows > 1 {
                result.lines.push(Line::default());
            }
        }
        result.lines.push(Line::default());
    }
    result.lines.extend(
        wrap(&actual.notice, width)
            .into_iter()
            .map(|chunk| Line::styled(chunk.text, themed_style(ThemeRole::Muted))),
    );
    result
}

/// The same wrapped source geometry drives drawing and mouse selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigurationTextLayout {
    pub area: Rect,
    pub lines: Vec<Line<'static>>,
    pub values: Vec<ConfigurationValueRegion>,
    pub scroll: u16,
    pub scroll_limit: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigurationValueRegion {
    pub area: Rect,
    pub field: usize,
    pub offsets: Vec<usize>,
}

impl ConfigurationValueRegion {
    pub fn source_offset(&self, column: u16) -> usize {
        let cell = usize::from(column.saturating_sub(self.area.x));
        self.offsets
            .get(cell)
            .or_else(|| self.offsets.last())
            .copied()
            .unwrap_or(0)
    }
}

pub(super) fn layout(draft: &ConfigurationDraft, area: Rect) -> ConfigurationTextLayout {
    let text = prepare(draft, area);
    let scroll_limit = text
        .lines
        .len()
        .saturating_sub(usize::from(area.height))
        .min(u16::MAX as usize) as u16;
    let scroll = draft.scroll.min(scroll_limit);
    let values = text
        .values
        .into_iter()
        .filter_map(|value| {
            let row = value.line.checked_sub(usize::from(scroll))?;
            (row < usize::from(area.height)).then_some(ConfigurationValueRegion {
                area: Rect::new(
                    area.x + value.column,
                    area.y + row as u16,
                    area.width.saturating_sub(value.column),
                    1,
                ),
                field: value.field,
                offsets: value.offsets,
            })
        })
        .collect();
    ConfigurationTextLayout {
        area,
        lines: text.lines,
        values,
        scroll,
        scroll_limit,
    }
}
