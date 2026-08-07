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
    pub fn new(resources_percent: u16) -> Self {
        Self { resources_percent }
    }

    pub fn resources_percent(self) -> u16 {
        self.resources_percent
    }
}
