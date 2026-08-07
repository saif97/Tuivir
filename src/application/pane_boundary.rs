//! The movable edge between the Resource Panels and the Details Pane.

/// The share of the Active Workspace's width the Resource Panels hold.
///
/// A share rather than a column count: the user chose a proportion of their
/// screen, so a terminal that changes size keeps the split they asked for
/// instead of a width that only fitted the terminal they asked in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneBoundary {
    resources_percent: u16,
}

impl Default for PaneBoundary {
    fn default() -> Self {
        Self::new(48)
    }
}

impl PaneBoundary {
    /// Holds the share inside the range both Panes stay usable in.
    ///
    /// Clamping here rather than at each caller makes the range a property of
    /// the boundary itself, so no way of moving it can leave the range.
    pub fn new(resources_percent: u16) -> Self {
        Self {
            resources_percent: resources_percent.clamp(MINIMUM_PERCENT, MAXIMUM_PERCENT),
        }
    }

    pub fn resources_percent(self) -> u16 {
        self.resources_percent
    }

    pub fn moved_left(self) -> Self {
        Self::new(self.resources_percent.saturating_sub(STEP))
    }

    pub fn moved_right(self) -> Self {
        Self::new(self.resources_percent.saturating_add(STEP))
    }
}

/// What one press of a resize Command moves the share by.
///
/// Twenty presses cross the whole width, which is few enough to reach the size
/// the user wants and many enough to stop where they meant to.
const STEP: u16 = 5;

/// The narrowest share either Pane may be left with.
///
/// A share rather than a column count, so the rule holds without consulting a
/// terminal width that changes. On an eighty-column terminal a quarter is
/// twenty columns, which still holds a Resource name inside its borders.
const MINIMUM_PERCENT: u16 = 25;
const MAXIMUM_PERCENT: u16 = 100 - MINIMUM_PERCENT;
