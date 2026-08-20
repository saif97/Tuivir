//! Where everything sits on screen, worked out once.
//!
//! Drawing and mouse routing both read this, so a region can never disagree
//! with the text drawn inside it. Nothing here knows about the mouse.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::application::{AppState, FocusedPane, WorkspacePresentation};

use super::render::{PaneChrome, pane_block, pane_title, visible_resource_range};

/// Blank columns between the Providers Pane label and the first Provider
/// Workspace, and between one Provider Workspace and the next.
pub const PROVIDER_LABEL_GAP: u16 = 2;
pub const PROVIDER_WORKSPACE_GAP: u16 = 3;
/// Blank columns between one Detail View Tab label and the next.
pub const DETAIL_VIEW_GAP: u16 = 2;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenLayout {
    /// The three horizontal bands: the provider bar, the Active Workspace, and
    /// the running-command status line.
    pub provider_bar: Rect,
    pub workspace: Rect,
    pub status: Rect,
    /// The Providers Pane label inside the provider bar.
    pub provider_selector: Rect,
    /// The Active Workspace's Target Environment, kept apart from navigation.
    pub active_target: Option<Rect>,
    /// One region per Provider Workspace, in the order they are drawn.
    pub provider_workspaces: Vec<Rect>,
    /// Absent until a Provider Workspace is active.
    pub panes: Option<WorkspacePanes>,
    /// The region the topmost modal owns, when one is open.
    pub overlay: Option<Rect>,
}

/// The Panes of the Active Workspace, and the rows and Detail View Tabs inside them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspacePanes {
    pub resources: Rect,
    /// One region per Resource Panel, in the order they are drawn.
    pub resource_panels: Vec<Rect>,
    /// Per Resource Panel, the visible rows paired with the Resource each one
    /// shows. Scrolling means a row's position is not its Resource index.
    pub resource_rows: Vec<Vec<(usize, Rect)>>,
    pub details: Rect,
    /// The scrollable text area of the active Detail View Tab, excluding its
    /// summary, view strip, and border.
    pub detail_content: Rect,
    /// One region per Detail View Tab label, in the order they are drawn.
    pub detail_views: Vec<Rect>,
    /// The Pane Boundary the user drags: the two border columns that touch,
    /// the right of the Resource Panels and the left of the Details Pane.
    pub pane_boundary: Rect,
}

impl ScreenLayout {
    pub fn measure(state: &AppState, area: Rect) -> Self {
        let bands = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(area);
        let (provider_bar, workspace, status) = (bands[0], bands[1], bands[2]);

        let mut x = provider_bar.x;
        let selector_width = u16::from(state.providers.is_empty())
            * (label_width(&provider_selector_label(state)) + PROVIDER_LABEL_GAP);
        let provider_selector = Rect::new(x, provider_bar.y, selector_width, 1);
        x += selector_width;

        let active_target = active_target_label(state).map(|label| {
            let width = label_width(&label).min(provider_bar.width);
            Rect::new(
                provider_bar.right().saturating_sub(width),
                provider_bar.y,
                width,
                1,
            )
        });
        let navigation_end = active_target.map_or(provider_bar.right(), |area| area.x);

        let mut provider_workspaces = Vec::with_capacity(state.providers.len());
        for index in 0..state.providers.len() {
            if index > 0 {
                x += PROVIDER_WORKSPACE_GAP;
            }
            let width = label_width(&provider_workspace_label(state, index))
                .min(navigation_end.saturating_sub(x));
            provider_workspaces.push(Rect::new(x, provider_bar.y, width, 1));
            x = x.saturating_add(width);
        }

        Self {
            provider_bar,
            workspace,
            status,
            provider_selector,
            active_target,
            provider_workspaces,
            panes: measure_panes(state, workspace),
            overlay: overlay_area(state, area),
        }
    }
}

/// The region of whichever modal is drawn last, and so sits on top.
pub fn overlay_area(state: &AppState, area: Rect) -> Option<Rect> {
    confirmation_area(state, area)
        .or_else(|| command_error_area(state, area))
        .or_else(|| help_overlay_area(state, area))
}

pub fn help_overlay_area(state: &AppState, area: Rect) -> Option<Rect> {
    let help = state.help_overlay.as_ref()?;
    Some(centered_rect(
        42,
        (help.entries.len() as u16 + 2).max(4),
        area,
    ))
}

pub fn command_error_area(state: &AppState, area: Rect) -> Option<Rect> {
    let error = state.command_error.as_deref()?;
    // Narrow terminals wrap the message instead of clipping it: an error that
    // cannot name its Provider, Resource, and Command is not an identifying one.
    let message_width = error.chars().count() as u16;
    let width = (message_width + 4).min(area.width);
    let wrapped_lines = message_width.div_ceil(width.saturating_sub(2).max(1));
    Some(centered_rect(width, wrapped_lines + 3, area))
}

pub fn confirmation_area(state: &AppState, area: Rect) -> Option<Rect> {
    state
        .confirmation
        .as_ref()
        .map(|_| centered_rect(64, 5, area))
}

pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

fn measure_panes(state: &AppState, workspace: Rect) -> Option<WorkspacePanes> {
    let provider = state.active_workspace()?;
    let resources_width = state.pane_boundary.resources_width(workspace.width);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(resources_width), Constraint::Min(0)])
        .split(workspace);
    let (resources, details) = (columns[0], columns[1]);
    let pane_boundary = Rect::new(
        resources.right().saturating_sub(1),
        resources.y,
        2.min(resources.width),
        resources.height,
    );

    let WorkspacePresentation::Ready(view) = provider.presentation() else {
        // A Workspace that is still loading or has failed draws no Resource
        // Panels, so it offers no rows to point at.
        return Some(WorkspacePanes {
            resources,
            resource_panels: Vec::new(),
            resource_rows: Vec::new(),
            details,
            detail_content: Rect::default(),
            detail_views: Vec::new(),
            pane_boundary,
        });
    };

    let panel_areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            view.panels()
                .map(|panel| Constraint::Fill(panel.panel.resources.len().max(1) as u16)),
        )
        .split(resources);

    let mut resource_panels = Vec::new();
    let mut resource_rows = Vec::new();
    for (panel, panel_area) in view.panels().zip(panel_areas.iter().copied()) {
        resource_panels.push(panel_area);
        let viewport_height = panel_area.height.saturating_sub(2) as usize;
        let visible = visible_resource_range(
            panel.selected_index,
            viewport_height,
            panel.panel.resources.len(),
        );
        let inner = Rect::new(
            panel_area.x + 1,
            panel_area.y + 1,
            panel_area.width.saturating_sub(2),
            panel_area.height.saturating_sub(2),
        );
        resource_rows.push(
            (visible.start..visible.end)
                .map(|index| {
                    (
                        index,
                        Rect::new(
                            inner.x,
                            inner.y.saturating_add((index - visible.start) as u16),
                            inner.width,
                            1,
                        ),
                    )
                })
                .collect(),
        );
    }

    let inner = pane_block(
        pane_title(None, "Details", false),
        false,
        PaneChrome::Details,
    )
    .inner(details);
    let has_detail_tabs = view.selected_resource.is_some();
    let detail_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(u16::from(has_detail_tabs)),
            Constraint::Min(0),
        ])
        .split(inner);

    let mut detail_views = Vec::new();
    let mut tab_x = detail_rows[0].x;
    if has_detail_tabs {
        let overview_width = label_width(&detail_view_label("Overview", view.overview_selected));
        detail_views.push(Rect::new(tab_x, detail_rows[0].y, overview_width, 1));
        tab_x = tab_x.saturating_add(overview_width + DETAIL_VIEW_GAP);
    }
    for detail in view.detail_views {
        let selected = view
            .selected_detail_view
            .is_some_and(|selected| selected.id == detail.id);
        let width = label_width(&detail_view_label(&detail.title, selected));
        detail_views.push(Rect::new(tab_x, detail_rows[0].y, width, 1));
        tab_x = tab_x.saturating_add(width + DETAIL_VIEW_GAP);
    }

    Some(WorkspacePanes {
        resources,
        resource_panels,
        resource_rows,
        details,
        detail_content: detail_rows[1],
        detail_views,
        pane_boundary,
    })
}

/// Builds the Providers Pane label exactly as the provider bar draws it.
///
/// Measuring and drawing read this one string, so a region cannot disagree
/// with the text under it. The same holds for the two labels below.
pub fn provider_selector_label(state: &AppState) -> String {
    let label = match &state.hints.focus_providers {
        Some(key) => format!("[{key}] Providers"),
        None => "Providers".to_owned(),
    };
    if state.focused_pane == FocusedPane::Providers {
        format!("▶ {label}")
    } else {
        label
    }
}

/// Builds one Provider Workspace label exactly as the provider bar draws it.
pub fn provider_workspace_label(state: &AppState, index: usize) -> String {
    let provider = &state.providers[index];
    let label = match state.hints.focus_providers.as_deref() {
        Some(key) => format!("[{key}] {}", provider.name()),
        None => provider.name().to_owned(),
    };
    if index == 0 && state.focused_pane == FocusedPane::Providers {
        format!("▶ {label}")
    } else {
        label
    }
}

/// Builds the Active Workspace's Target Environment label.
pub fn active_target_label(state: &AppState) -> Option<String> {
    state
        .active_workspace()
        .and_then(|workspace| workspace.target_environment())
        .map(|target| format!("Target: {target}"))
}

/// Builds one Detail View Tab label exactly as the Details Pane draws it.
pub fn detail_view_label(title: &str, selected: bool) -> String {
    if selected {
        format!("[ {title} ]")
    } else {
        title.to_owned()
    }
}

pub fn label_width(label: &str) -> u16 {
    label.chars().count() as u16
}

/// The blank columns drawn between labels, as a string of that width.
pub fn gap(width: u16) -> String {
    " ".repeat(width as usize)
}
