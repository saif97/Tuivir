//! Turns a mouse event into the Command it means, using the drawn layout.
//!
//! A click resolving to a Command is the same shape as a Keybinding resolving
//! to one, so the mouse joins the keyboard's single path through `App::invoke`
//! instead of reaching into application state on its own.

use super::input::{MouseAction, MouseInput};
use super::screen_layout::ScreenLayout;
use crate::application::Command;

/// Resolves one mouse event against the layout that drew the screen.
///
/// Returns `None` where there is nothing to do: over a border, over blank space,
/// or while an overlay owns the screen.
pub fn resolve(layout: &ScreenLayout, input: MouseInput) -> Option<Command> {
    // A modal owns the screen while it is open, so a click anywhere is not for
    // the widgets drawn beneath it. It is dismissed by its own Commands.
    if layout.overlay.is_some() {
        return None;
    }
    match input.action {
        MouseAction::Press => press(layout, input),
        MouseAction::ScrollUp => scroll(layout, input, ScrollDirection::Up),
        MouseAction::ScrollDown => scroll(layout, input, ScrollDirection::Down),
    }
}

enum ScrollDirection {
    Up,
    Down,
}

fn press(layout: &ScreenLayout, input: MouseInput) -> Option<Command> {
    let point = point(input);

    if let Some(index) = index_containing(&layout.provider_workspaces, point) {
        return Some(Command::ActivateProviderWorkspace(index));
    }
    if layout.provider_selector.contains(point) {
        return Some(Command::FocusProviders);
    }

    let panes = layout.panes.as_ref()?;
    for (panel, rows) in panes.resource_rows.iter().enumerate() {
        if let Some((resource, _)) = rows.iter().find(|(_, area)| area.contains(point)) {
            return Some(Command::SelectResource {
                panel,
                resource: *resource,
            });
        }
    }
    if let Some(index) = index_containing(&panes.detail_views, point) {
        return Some(Command::ActivateDetailView(index));
    }
    if let Some(panel) = index_containing(&panes.resource_panels, point) {
        return Some(Command::FocusResourcePanel(panel));
    }
    if panes.details.contains(point) {
        return Some(Command::FocusDetails);
    }
    None
}

/// The wheel scrolls what is under the pointer and never moves focus, so it
/// reads the layout the same way but resolves to a scrolling Command only.
fn scroll(layout: &ScreenLayout, input: MouseInput, direction: ScrollDirection) -> Option<Command> {
    let point = point(input);
    let panes = layout.panes.as_ref()?;

    if panes.details.contains(point) || index_containing(&panes.detail_views, point).is_some() {
        return Some(match direction {
            ScrollDirection::Up => Command::ScrollDetailsUp,
            ScrollDirection::Down => Command::ScrollDetailsDown,
        });
    }

    let panel = panes
        .resource_rows
        .iter()
        .position(|rows| rows.iter().any(|(_, area)| area.contains(point)))
        .or_else(|| index_containing(&panes.resource_panels, point))?;
    Some(match direction {
        ScrollDirection::Up => Command::ScrollResourcePanelUp(panel),
        ScrollDirection::Down => Command::ScrollResourcePanelDown(panel),
    })
}

fn point(input: MouseInput) -> ratatui::layout::Position {
    ratatui::layout::Position::new(input.column, input.row)
}

fn index_containing(
    areas: &[ratatui::layout::Rect],
    point: ratatui::layout::Position,
) -> Option<usize> {
    areas.iter().position(|area| area.contains(point))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::AppState;
    use ratatui::layout::Rect;

    fn press(column: u16, row: u16) -> MouseInput {
        MouseInput {
            action: MouseAction::Press,
            column,
            row,
        }
    }

    /// Startup draws a Providers Pane and nothing else, so every other point is
    /// blank space rather than a region to guess at.
    #[test]
    fn an_empty_screen_routes_only_the_providers_pane() {
        let layout = ScreenLayout::measure(&AppState::default(), Rect::new(0, 0, 80, 24));

        assert_eq!(resolve(&layout, press(0, 0)), Some(Command::FocusProviders));
        assert_eq!(resolve(&layout, press(79, 23)), None);
    }

    #[test]
    fn the_wheel_over_blank_space_resolves_to_nothing() {
        let layout = ScreenLayout::measure(&AppState::default(), Rect::new(0, 0, 80, 24));

        assert_eq!(
            resolve(
                &layout,
                MouseInput {
                    action: MouseAction::ScrollDown,
                    column: 79,
                    row: 23,
                }
            ),
            None
        );
    }
}
