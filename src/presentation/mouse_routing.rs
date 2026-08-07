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

    /// A Provider Workspace with no snapshot yet. It draws both Panes and the
    /// Pane Boundary between them, which is all this needs to point at.
    fn active_workspace() -> AppState {
        use crate::application::ProviderWorkspaceState;
        use crate::domain::{Provider, ProviderId};

        AppState {
            providers: vec![ProviderWorkspaceState::new(
                Provider::new(ProviderId::new("docker"), "Docker", None, None),
                None,
            )],
            active_provider: Some(0),
            ..AppState::default()
        }
    }

    /// The Pane Boundary is tested before the Panes it separates. Its right
    /// column is the Details Pane's first column, so a boundary tested second
    /// could never be grabbed there at all.
    #[test]
    fn a_press_on_the_pane_boundary_grabs_it_rather_than_the_pane_behind_it() {
        let state = active_workspace();
        let layout = ScreenLayout::measure(&state, Rect::new(0, 0, 80, 24));
        let boundary = layout
            .panes
            .as_ref()
            .expect("a Provider Workspace is active")
            .pane_boundary;
        let row = boundary.y + 2;

        assert_eq!(
            resolve(&layout, press(boundary.x, row), None),
            Some(Command::GrabPaneBoundary(0)),
            "the grab remembers which of the two columns was pressed"
        );
        assert_eq!(
            resolve(&layout, press(boundary.x + 1, row), None),
            Some(Command::GrabPaneBoundary(1)),
            "the Details Pane starts on this column, and the boundary wins it"
        );
        assert_eq!(
            resolve(&layout, press(boundary.x + 2, row), None),
            Some(Command::FocusDetails),
            "one column further in is the Details Pane again"
        );
    }

    /// Hit-testing follows the resize. The screen is measured afresh each frame,
    /// so the boundary the user aims at is the one now drawn, and the column it
    /// used to occupy belongs to the Pane that has taken it.
    #[test]
    fn hit_testing_follows_the_pane_boundary_after_a_resize() {
        use crate::application::PaneBoundary;

        let mut state = active_workspace();
        let area = Rect::new(0, 0, 80, 24);
        let before = ScreenLayout::measure(&state, area);
        let was = before
            .panes
            .as_ref()
            .expect("a Provider Workspace is active")
            .pane_boundary;

        state.pane_boundary = PaneBoundary::new(70);
        let after = ScreenLayout::measure(&state, area);
        let now = after
            .panes
            .as_ref()
            .expect("a Provider Workspace is active")
            .pane_boundary;
        let row = now.y + 2;

        assert_ne!(was.x, now.x, "the resize moved the boundary");
        assert_eq!(
            resolve(&after, press(now.x, row), None),
            Some(Command::GrabPaneBoundary(0)),
            "the boundary is grabbed where it is drawn now"
        );
        assert_eq!(
            resolve(&after, press(was.x, row), None),
            None,
            "the column it left is the Resource Panels' again"
        );
    }

    /// A modal can open on its own while the pointer is mid-drag — a Resource
    /// Command that failed in the background. It must not strand the Pane
    /// Boundary in the user's hand: a release the modal swallowed would leave
    /// the boundary held, and the next drag anywhere on screen would move it.
    ///
    /// The boundary still holds still underneath, because the modal owns the
    /// screen until it is dismissed.
    #[test]
    fn a_modal_that_opens_mid_drag_does_not_strand_the_pane_boundary() {
        let mut state = active_workspace();
        state.command_error = Some("Docker stop failed for api".to_owned());
        let layout = ScreenLayout::measure(&state, Rect::new(0, 0, 80, 24));
        assert!(layout.overlay.is_some(), "the failure modal is open");
        let at = |action| MouseInput {
            action,
            column: 39,
            row: layout.workspace.y + 2,
        };

        assert_eq!(
            resolve(&layout, at(MouseAction::Release), Some(0)),
            Some(Command::ReleasePaneBoundary),
            "letting go is not a click on the modal, and is always reported"
        );
        assert_eq!(
            resolve(&layout, at(MouseAction::Drag), Some(0)),
            None,
            "the modal owns the screen, so the boundary holds still under it"
        );
    }

    /// Only the boundary the pointer took hold of follows it. A drag that began
    /// anywhere else is the user selecting text, and leaves the Panes alone.
    #[test]
    fn only_a_held_pane_boundary_follows_the_pointer() {
        let state = active_workspace();
        let layout = ScreenLayout::measure(&state, Rect::new(0, 0, 80, 24));
        let row = layout.workspace.y + 2;
        let drag = |column| MouseInput {
            action: MouseAction::Drag,
            column,
            row,
        };

        assert_eq!(
            resolve(&layout, drag(39), None),
            None,
            "a drag that grabbed nothing moves nothing"
        );
        assert_eq!(
            resolve(&layout, drag(39), Some(0)),
            Some(Command::SetPaneBoundary(50)),
            "column 39 ends a Resources column 40 of 80 wide"
        );
        assert_eq!(
            resolve(&layout, drag(39), Some(1)),
            Some(Command::SetPaneBoundary(48)),
            "grabbing the right column keeps the boundary one column back"
        );
    }

    /// Letting go is only news when something was held. Every other click ends
    /// in a release too, and none of those concern the Pane Boundary.
    #[test]
    fn releasing_reports_only_a_boundary_that_was_held() {
        let layout = ScreenLayout::measure(&active_workspace(), Rect::new(0, 0, 80, 24));
        let release = MouseInput {
            action: MouseAction::Release,
            column: 39,
            row: layout.workspace.y + 2,
        };

        assert_eq!(resolve(&layout, release, None), None);
        assert_eq!(
            resolve(&layout, release, Some(0)),
            Some(Command::ReleasePaneBoundary)
        );
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
