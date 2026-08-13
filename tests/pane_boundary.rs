//! The Pane Boundary between the Resource Panels and the Details Pane, tested
//! through the public interface.
//!
//! Measuring, hit-testing, and pointer normalization all meet here because they
//! are one behaviour to the user: the line they drag. The application-side
//! Commands live in `application.rs`, and their Keybindings in
//! `command_registry.rs`.

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use tuivir::{
    application::{AppState, Command, PaneBoundary, ProviderWorkspaceState},
    domain::{Provider, ProviderId},
    presentation::resolve_mouse,
    presentation::{MouseAction, MouseInput, ScreenLayout, mouse_from_event, render_to_text},
};

/// A Provider Workspace with no snapshot yet. It draws both Panes and the Pane
/// Boundary between them, which is all these tests need to point at.
fn workspace_at(boundary: PaneBoundary) -> AppState {
    AppState {
        providers: vec![ProviderWorkspaceState::new(
            Provider::new(ProviderId::new("docker"), "Docker", None, None),
            None,
        )],
        active_provider: Some(0),
        pane_boundary: boundary,
        ..AppState::default()
    }
}

fn active_workspace() -> AppState {
    workspace_at(PaneBoundary::default())
}

fn press(column: u16, row: u16) -> MouseInput {
    MouseInput {
        action: MouseAction::Press,
        column,
        row,
    }
}

/// The share the user chose is what the Panes are measured from, so one
/// terminal shows two different splits for two different shares.
#[test]
fn the_pane_boundary_decides_where_the_resource_panels_end() {
    let area = Rect::new(0, 0, 80, 24);

    let narrow = ScreenLayout::measure(&workspace_at(PaneBoundary::new(25)), area)
        .panes
        .expect("a Provider Workspace is active");
    let wide = ScreenLayout::measure(&workspace_at(PaneBoundary::new(75)), area)
        .panes
        .expect("a Provider Workspace is active");

    assert_eq!(narrow.resources.width, 20, "a quarter of 80 columns");
    assert_eq!(wide.resources.width, 60, "three quarters of 80 columns");
    assert_eq!(
        narrow.details.x, 20,
        "the Details Pane starts where the Resource Panels end"
    );
    assert_eq!(wide.details.x, 60);
}

/// A share, not a column count. A terminal that changes size keeps the split
/// the user chose, instead of a width that only suited the terminal they chose
/// it in.
#[test]
fn a_terminal_that_changes_size_keeps_the_share_the_user_chose() {
    let state = workspace_at(PaneBoundary::new(50));

    let small = ScreenLayout::measure(&state, Rect::new(0, 0, 80, 24))
        .panes
        .expect("a Provider Workspace is active");
    let large = ScreenLayout::measure(&state, Rect::new(0, 0, 120, 40))
        .panes
        .expect("a Provider Workspace is active");

    assert_eq!(small.resources.width, 40, "half of 80 columns");
    assert_eq!(large.resources.width, 60, "half of 120 columns");
    assert_eq!(
        small.pane_boundary.x, 39,
        "the boundary follows the Panes it separates"
    );
    assert_eq!(large.pane_boundary.x, 59);
}

/// The two border columns the user sees between the Panes are the two the mouse
/// can grab. A Pane Boundary measured anywhere else would be a line the user
/// cannot see and cannot aim at.
#[test]
fn the_measured_pane_boundary_sits_on_the_borders_that_are_drawn() {
    let state = workspace_at(PaneBoundary::new(40));

    let layout = ScreenLayout::measure(&state, Rect::new(0, 0, 80, 24));
    let screen = render_to_text(&state, 80, 24);
    let boundary = layout
        .panes
        .as_ref()
        .expect("a Provider Workspace is active")
        .pane_boundary;

    // A row below both Pane titles, so only the side borders sit on it.
    let row = screen.lines().nth(4).expect("a row inside the Panes");
    let drawn = row
        .chars()
        .enumerate()
        .filter(|(_, character)| *character == '\u{2502}' || *character == '\u{2503}')
        .map(|(column, _)| column as u16)
        .collect::<Vec<_>>();

    assert_eq!(
        drawn,
        vec![0, boundary.x, boundary.x + 1],
        "drawn row: {row}"
    );
    assert_eq!(boundary.width, 2, "both drawn columns can be grabbed");
}

/// The Pane Boundary is tested before the Panes it separates. Its right column
/// is the Details Pane's first column, so a boundary tested second could never
/// be grabbed there at all.
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
        resolve_mouse(&layout, press(boundary.x, row), None),
        Some(Command::GrabPaneBoundary(0)),
        "the grab remembers which of the two columns was pressed"
    );
    assert_eq!(
        resolve_mouse(&layout, press(boundary.x + 1, row), None),
        Some(Command::GrabPaneBoundary(1)),
        "the Details Pane starts on this column, and the boundary wins it"
    );
    assert_eq!(
        resolve_mouse(&layout, press(boundary.x + 2, row), None),
        Some(Command::FocusDetails),
        "one column further in is the Details Pane again"
    );
}

/// Hit-testing follows the resize. The screen is measured afresh each frame, so
/// the boundary the user aims at is the one now drawn, and the column it used to
/// occupy belongs to the Pane that has taken it.
#[test]
fn hit_testing_follows_the_pane_boundary_after_a_resize() {
    let area = Rect::new(0, 0, 80, 24);
    let before = ScreenLayout::measure(&active_workspace(), area);
    let was = before
        .panes
        .as_ref()
        .expect("a Provider Workspace is active")
        .pane_boundary;

    let after = ScreenLayout::measure(&workspace_at(PaneBoundary::new(70)), area);
    let now = after
        .panes
        .as_ref()
        .expect("a Provider Workspace is active")
        .pane_boundary;
    let row = now.y + 2;

    assert_ne!(was.x, now.x, "the resize moved the boundary");
    assert_eq!(
        resolve_mouse(&after, press(now.x, row), None),
        Some(Command::GrabPaneBoundary(0)),
        "the boundary is grabbed where it is drawn now"
    );
    assert_eq!(
        resolve_mouse(&after, press(was.x, row), None),
        None,
        "the column it left is the Resource Panels' again"
    );
}

/// A modal can open on its own while the pointer is mid-drag — a Resource
/// Command that failed in the background. It must not strand the Pane Boundary
/// in the user's hand: a release the modal swallowed would leave the boundary
/// held, and the next drag anywhere on screen would move it.
///
/// The boundary still holds still underneath, because the modal owns the screen
/// until it is dismissed.
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
        resolve_mouse(&layout, at(MouseAction::Release), Some(0)),
        Some(Command::ReleasePaneBoundary),
        "letting go is not a click on the modal, and is always reported"
    );
    assert_eq!(
        resolve_mouse(&layout, at(MouseAction::Drag), Some(0)),
        None,
        "the modal owns the screen, so the boundary holds still under it"
    );
}

/// Only the boundary the pointer took hold of follows it. A drag that began
/// anywhere else is the user selecting text, and leaves the Panes alone.
#[test]
fn only_a_held_pane_boundary_follows_the_pointer() {
    let layout = ScreenLayout::measure(&active_workspace(), Rect::new(0, 0, 80, 24));
    let row = layout.workspace.y + 2;
    let drag = |column| MouseInput {
        action: MouseAction::Drag,
        column,
        row,
    };

    assert_eq!(
        resolve_mouse(&layout, drag(39), None),
        None,
        "a drag that grabbed nothing moves nothing"
    );
    assert_eq!(
        resolve_mouse(&layout, drag(39), Some(0)),
        Some(Command::SetPaneBoundary(50)),
        "column 39 ends a Resources column 40 of 80 wide"
    );
    assert_eq!(
        resolve_mouse(&layout, drag(39), Some(1)),
        Some(Command::SetPaneBoundary(48)),
        "grabbing the right column keeps the boundary one column back"
    );
}

/// Letting go is only news when something was held. Every other click ends in a
/// release too, and none of those concern the Pane Boundary.
#[test]
fn releasing_reports_only_a_boundary_that_was_held() {
    let layout = ScreenLayout::measure(&active_workspace(), Rect::new(0, 0, 80, 24));
    let release = MouseInput {
        action: MouseAction::Release,
        column: 39,
        row: layout.workspace.y + 2,
    };

    assert_eq!(resolve_mouse(&layout, release, None), None);
    assert_eq!(
        resolve_mouse(&layout, release, Some(0)),
        Some(Command::ReleasePaneBoundary)
    );
}

/// Dragging needs both ends of the gesture: the movement that carries the Pane
/// Boundary, and the release that lets go of it. Movement with no button held is
/// still nothing, so pointing at a Pane never disturbs it.
#[test]
fn normalizes_drag_and_release_but_still_ignores_bare_movement() {
    for (kind, expected) in [
        (
            MouseEventKind::Drag(MouseButton::Left),
            Some(MouseAction::Drag),
        ),
        (
            MouseEventKind::Up(MouseButton::Left),
            Some(MouseAction::Release),
        ),
        (MouseEventKind::Drag(MouseButton::Right), None),
        (MouseEventKind::Moved, None),
    ] {
        assert_eq!(
            mouse_from_event(MouseEvent {
                kind,
                column: 4,
                row: 8,
                modifiers: KeyModifiers::NONE,
            })
            .map(|input| input.action),
            expected,
            "{kind:?} should normalize to {expected:?}"
        );
    }
}
