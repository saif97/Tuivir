//! Pointer input and hit-testing, tested through the public interface.
//!
//! What the mouse does to the Pane Boundary lives in `pane_boundary.rs`; this
//! is everything else the pointer reaches.

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use virtui::{
    application::{AppState, Command, FocusedPane, ProviderWorkspaceState},
    domain::{Provider, ProviderId},
    presentation::{
        MouseAction, MouseInput, ScreenLayout, mouse_from_event, render_to_text, resolve_mouse,
    },
};

fn press(column: u16, row: u16) -> MouseInput {
    MouseInput {
        action: MouseAction::Press,
        column,
        row,
    }
}

#[test]
fn normalizes_click_and_wheel_but_ignores_motion_and_release() {
    let click = mouse_from_event(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 4,
        row: 8,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(
        click,
        Some(MouseInput {
            action: MouseAction::Press,
            column: 4,
            row: 8,
        })
    );
    assert_eq!(
        mouse_from_event(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 4,
            row: 8,
            modifiers: KeyModifiers::NONE,
        })
        .map(|input| input.action),
        Some(MouseAction::ScrollDown)
    );
    assert_eq!(
        mouse_from_event(MouseEvent {
            kind: MouseEventKind::Moved,
            column: 4,
            row: 8,
            modifiers: KeyModifiers::NONE,
        }),
        None
    );
}

/// Startup draws a Providers Pane and nothing else, so every other point is
/// blank space rather than a region to guess at.
#[test]
fn an_empty_screen_routes_only_the_providers_pane() {
    let layout = ScreenLayout::measure(&AppState::default(), Rect::new(0, 0, 80, 24));

    assert_eq!(
        resolve_mouse(&layout, press(0, 0), None),
        Some(Command::FocusProviders)
    );
    assert_eq!(resolve_mouse(&layout, press(79, 23), None), None);
}

#[test]
fn the_wheel_over_blank_space_resolves_to_nothing() {
    let layout = ScreenLayout::measure(&AppState::default(), Rect::new(0, 0, 80, 24));

    assert_eq!(
        resolve_mouse(
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

/// Hit-testing is only honest if it agrees with what the user sees, so this
/// compares measured regions against the drawn screen rather than against a
/// second copy of the same arithmetic.
#[test]
fn measured_provider_workspaces_sit_where_they_are_drawn() {
    let state = AppState {
        providers: vec![
            ProviderWorkspaceState::new(
                Provider::new(ProviderId::new("docker"), "Docker", None, None),
                None,
            ),
            ProviderWorkspaceState::new(
                Provider::new(ProviderId::new("incus"), "Incus", None, None),
                None,
            ),
        ],
        active_provider: Some(0),
        ..AppState::default()
    };
    // The Providers Pane is unfocused by default, so no caret is drawn.
    assert_eq!(state.focused_pane, FocusedPane::Resources);

    let layout = ScreenLayout::measure(&state, Rect::new(0, 0, 80, 24));
    let screen = render_to_text(&state, 80, 24);
    let bar = screen.lines().next().expect("a provider bar is drawn");

    let drawn = bar.find("Incus").expect("the Provider Workspace is drawn") as u16;

    assert_eq!(
        layout.provider_workspaces[1].x, drawn,
        "a click lands where the Provider Workspace is drawn, not where a second calculation guessed"
    );
}
