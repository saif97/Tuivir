//! Wrap Settings values without mixing their source text with visual padding.

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
}

struct Chunk {
    text: String,
}

fn visible(grapheme: &str) -> String {
    if grapheme.chars().any(char::is_control) {
        grapheme.chars().flat_map(char::escape_default).collect()
    } else {
        grapheme.to_owned()
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
            let size = visible(graphemes[end].1).width().min(width);
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
        let mut display = String::new();
        for &(_, grapheme) in &graphemes[start..end] {
            let mut glyph = visible(grapheme);
            if glyph.width() > width {
                glyph = "�".into();
            }
            display.push_str(&glyph);
        }
        chunks.push(Chunk { text: display });
        start = if newline { end + 1 } else { end };
    }
    if text.is_empty() || text.ends_with('\n') {
        chunks.push(Chunk {
            text: String::new(),
        });
    }
    chunks
}

pub(super) fn prepare(draft: &ConfigurationDraft, area: Rect) -> ConfigurationText {
    let mut result = ConfigurationText { lines: Vec::new() };
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
        for (_, field) in fields {
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
                if let Some(chunk) = values.get(row) {
                    spans.push(Span::raw(chunk.text.clone()));
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
