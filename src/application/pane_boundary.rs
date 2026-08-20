//! The movable edge between the Resource Panels and the Details Pane.

/// The share of the Active Workspace's width the Resource Panels hold.
///
/// A share rather than a column count: the user chose a proportion of their
/// screen, so a terminal that changes size keeps the split they asked for
/// instead of a width that only fitted the terminal they asked in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneBoundary {
    resources_percent: u16,
    /// Which of the boundary's two columns the pointer took hold of, while it
    /// is held. `None` means the pointer is not dragging it.
    grab: Option<u16>,
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
            resources_percent: clamped(resources_percent),
            grab: None,
        }
    }

    pub fn resources_percent(self) -> u16 {
        self.resources_percent
    }

    /// The Resource Pane width that fits this terminal while leaving both
    /// Panes usable. This is a presentation-time clamp: it never changes the
    /// preference the user chose or the value persisted for a larger terminal.
    pub fn resources_width(self, terminal_width: u16) -> u16 {
        let preferred = terminal_width.saturating_mul(self.resources_percent) / 100;
        let minimum = MINIMUM_PANE_WIDTH.min(terminal_width / 2);
        preferred.clamp(minimum, terminal_width.saturating_sub(minimum))
    }

    pub fn is_valid_percent(resources_percent: u16) -> bool {
        (MINIMUM_PERCENT..=MAXIMUM_PERCENT).contains(&resources_percent)
    }

    /// Which of the boundary's two columns the pointer is holding.
    pub fn grab(self) -> Option<u16> {
        self.grab
    }

    pub fn moved_left(self) -> Self {
        self.with_share(self.resources_percent.saturating_sub(STEP))
    }

    pub fn moved_right(self) -> Self {
        self.with_share(self.resources_percent.saturating_add(STEP))
    }

    pub fn grabbed_at(self, column: u16) -> Self {
        Self {
            grab: Some(column),
            ..self
        }
    }

    /// Follows the pointer, but only while the pointer is holding on.
    ///
    /// A drag that started somewhere else — the user selecting Details text —
    /// reaches here as well, and an unheld boundary is what refuses it.
    pub fn dragged_to(self, resources_percent: u16) -> Self {
        match self.grab {
            Some(_) => self.with_share(resources_percent),
            None => self,
        }
    }

    pub fn released(self) -> Self {
        Self { grab: None, ..self }
    }

    fn with_share(self, resources_percent: u16) -> Self {
        Self {
            resources_percent: clamped(resources_percent),
            ..self
        }
    }
}

fn clamped(resources_percent: u16) -> u16 {
    resources_percent.clamp(MINIMUM_PERCENT, MAXIMUM_PERCENT)
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
/// The smallest useful Pane on a normal terminal. Smaller terminals split
/// evenly rather than allowing either Pane to disappear.
const MINIMUM_PANE_WIDTH: u16 = 20;
