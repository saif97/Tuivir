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
/// `boundary_grab` is the Pane Boundary column the pointer is holding, which is
/// the one fact a gesture needs beyond the regions on screen: a drag names no
/// region, because the pointer has usually left the one it started in.
///
/// Returns `None` where there is nothing to do: over a border, over blank space,
/// or while an overlay owns the screen.
pub fn resolve(
    layout: &ScreenLayout,
    input: MouseInput,
    boundary_grab: Option<u16>,
) -> Option<Command> {
    match input.action {
        // Letting go is answered first, because it is not a click on whatever is
        // on screen — it ends a gesture that began before it. A modal that opened
        // mid-drag would otherwise swallow the release and leave the boundary
        // held, and the next drag anywhere would carry it off.
        //
        // Every click ends in a release too, and only a held boundary makes one
        // worth reporting.
        MouseAction::Release => boundary_grab.map(|_| Command::ReleasePaneBoundary),
        // A modal owns the screen while it is open, so pointing at anything is
        // not for the widgets drawn beneath it. It is dismissed by its own
        // Commands.
        _ if layout.overlay.is_some() => None,
        MouseAction::Press => press(layout, input),
        MouseAction::Drag => drag(layout, input, boundary_grab?),
        MouseAction::ScrollUp => scroll(layout, input, ScrollDirection::Up),
        MouseAction::ScrollDown => scroll(layout, input, ScrollDirection::Down),
    }
}

/// Carries a held Pane Boundary to the pointer, as the share it now leaves the
/// Resource Panels.
///
/// The share is worked out here because the width it is a share of belongs to
/// the screen. `grab` is the column of the boundary the pointer took hold of,
/// and taking it off the pointer is what stops the boundary jumping.
fn drag(layout: &ScreenLayout, input: MouseInput, grab: u16) -> Option<Command> {
    let workspace = layout.workspace;
    if workspace.width == 0 {
        return None;
    }
    let last_column = input.column.saturating_sub(grab);
    let width = last_column
        .saturating_add(1)
        .saturating_sub(workspace.x)
        .min(workspace.width);
    Some(Command::SetPaneBoundary(
        (u32::from(width) * 100 / u32::from(workspace.width)) as u16,
    ))
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
    // Before the Panes: the boundary's own columns are the last column of the
    // Resource Panels and the first of the Details Pane, so a boundary tested
    // afterwards would never be reached.
    if panes.pane_boundary.contains(point) {
        return Some(Command::GrabPaneBoundary(point.x - panes.pane_boundary.x));
    }
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

        assert_eq!(
            resolve(&layout, press(0, 0), None),
            Some(Command::FocusProviders)
        );
        assert_eq!(resolve(&layout, press(79, 23), None), None);
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
                },
                None
            ),
            None
        );
    }
}
