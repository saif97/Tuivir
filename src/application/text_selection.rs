//! Selection state shared by plain Details and wrapped Settings values.

use std::ops::Range;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextSelection<P> {
    anchor: P,
    cursor: P,
    click_range: Option<Range<P>>,
}

impl<P: Copy + Ord> TextSelection<P> {
    pub fn begin(anchor: P) -> Self {
        Self {
            anchor,
            cursor: anchor,
            click_range: None,
        }
    }

    pub fn whole_value(anchor: P, range: Range<P>) -> Self {
        Self {
            click_range: Some(range),
            ..Self::begin(anchor)
        }
    }

    pub fn extend(&mut self, cursor: P) {
        self.cursor = cursor;
        self.click_range = None;
    }

    pub fn range(&self) -> Range<P> {
        self.click_range
            .clone()
            .unwrap_or_else(|| self.anchor.min(self.cursor)..self.anchor.max(self.cursor))
    }
}

/// Each view translates a pointer into the coordinates of its source content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DetailSelectionPoint {
    Text { line: u16, column: u16 },
    ConfigurationValue { field: usize, offset: usize },
}
