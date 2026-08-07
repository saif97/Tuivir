use ratatui::{
    Frame, Terminal,
    backend::TestBackend,
    buffer::Cell,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph},
};

use crate::application::{AppState, FocusedPane, ProviderWorkspaceState};
use crate::application::{
    DetailContent, ResourceDetailsView, ResourcePanelView, WorkspacePresentation, WorkspaceView,
};
use crate::domain::ResourceState;

/// Blank columns between the Providers Pane label and the first Provider
/// Workspace, and between one Provider Workspace and the next.
const PROVIDER_LABEL_GAP: u16 = 2;
const PROVIDER_WORKSPACE_GAP: u16 = 3;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InteractionGeometry {
    pub provider_tabs: Vec<(Rect, usize)>,
    pub provider_selector: Option<Rect>,
    pub resource_panels: Vec<(Rect, usize)>,
    pub resource_rows: Vec<Vec<(usize, Rect)>>,
    pub detail_view_tabs: Vec<(Rect, usize)>,
    pub resources: Option<Rect>,
    pub details: Option<Rect>,
}

impl InteractionGeometry {
    pub fn hit(&self, column: u16, row: u16) -> Option<InteractionTarget> {
        let point = ratatui::layout::Position::new(column, row);
        if let Some((_, index)) = self
            .provider_tabs
            .iter()
            .find(|(area, _)| area.contains(point))
        {
            return Some(InteractionTarget::Provider(*index));
        }
        for (panel_index, rows) in self.resource_rows.iter().enumerate() {
            if let Some((resource_index, _)) = rows.iter().find(|(_, area)| area.contains(point)) {
                return Some(InteractionTarget::Resource {
                    panel: panel_index,
                    resource: *resource_index,
                });
            }
        }
        if let Some((_, index)) = self
            .resource_panels
            .iter()
            .find(|(area, _)| area.contains(point))
        {
            return Some(InteractionTarget::ResourcePanel(*index));
        }
        if let Some((_, index)) = self
            .detail_view_tabs
            .iter()
            .find(|(area, _)| area.contains(point))
        {
            return Some(InteractionTarget::DetailView(*index));
        }
        if self
            .provider_selector
            .is_some_and(|area| area.contains(point))
        {
            return Some(InteractionTarget::ProviderSelector);
        }
        self.details
            .filter(|area| area.contains(point))
            .map(|_| InteractionTarget::Details)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InteractionTarget {
    Provider(usize),
    ProviderSelector,
    ResourcePanel(usize),
    Resource { panel: usize, resource: usize },
    DetailView(usize),
    Details,
}

pub fn interaction_geometry(state: &AppState, area: Rect) -> InteractionGeometry {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(u16::from(!state.running_commands.is_empty())),
        ])
        .split(area);
    let mut geometry = InteractionGeometry::default();
    let mut x = rows[0].x;
    let selector_width = label_width(&provider_selector_label(state)) + PROVIDER_LABEL_GAP;
    geometry.provider_selector = Some(Rect::new(x, rows[0].y, selector_width, 1));
    x += selector_width;
    for (index, provider) in state.providers.iter().enumerate() {
        if index > 0 {
            x += PROVIDER_WORKSPACE_GAP;
        }
        let width = label_width(&provider_workspace_label(
            provider,
            Some(index) == state.active_provider,
        ));
        geometry
            .provider_tabs
            .push((Rect::new(x, rows[0].y, width, 1), index));
        x = x.saturating_add(width);
    }
    let Some(provider) = state.active_workspace() else {
        return geometry;
    };
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(rows[1]);
    geometry.details = Some(columns[1]);
    geometry.resources = Some(columns[0]);
    if let crate::application::WorkspacePresentation::Ready(view) = provider.presentation() {
        let panel_areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                view.panels()
                    .map(|panel| Constraint::Fill(panel.panel.resources.len().max(1) as u16)),
            )
            .split(columns[0]);
        for (panel, panel_area) in view.panels().zip(panel_areas.iter().copied()) {
            let panel_index = geometry.resource_panels.len();
            geometry.resource_panels.push((panel_area, panel_index));
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
            geometry.resource_rows.push(
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
        let details = columns[1];
        let block = pane_block(pane_title(None, "Details", false), false);
        let inner = block.inner(details);
        let summary_len = view
            .selected_resource
            .map_or(1, |resource| 1 + resource.fields.len()) as u16;
        let detail_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(summary_len),
                Constraint::Length(u16::from(!view.detail_views.is_empty()) * 2),
                Constraint::Min(0),
            ])
            .split(inner);
        if !view.detail_views.is_empty() {
            let mut tab_x = detail_rows[1].x;
            for (index, detail) in view.detail_views.iter().enumerate() {
                if index > 0 {
                    tab_x += 2;
                }
                let width = detail.title.chars().count() as u16
                    + if view
                        .selected_detail_view
                        .is_some_and(|selected| selected.id == detail.id)
                    {
                        4
                    } else {
                        0
                    };
                geometry
                    .detail_view_tabs
                    .push((Rect::new(tab_x, detail_rows[1].y + 1, width, 1), index));
                tab_x = tab_x.saturating_add(width);
            }
        }
    }
    geometry
}

/// Builds the Providers Pane label exactly as the provider bar draws it.
///
/// Measuring and drawing read this one string, so a hit-test region cannot
/// disagree with the text under it.
fn provider_selector_label(state: &AppState) -> String {
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
fn provider_workspace_label(provider: &ProviderWorkspaceState, active: bool) -> String {
    if !active {
        return provider.name().to_owned();
    }
    match provider.target_environment() {
        Some(target) => format!("[ {} · {target} ]", provider.name()),
        None => format!("[ {} ]", provider.name()),
    }
}

fn label_width(label: &str) -> u16 {
    label.chars().count() as u16
}

pub fn render(state: &AppState, frame: &mut Frame<'_>) {
    let geometry = interaction_geometry(state, frame.area());
    render_with_geometry(state, frame, &geometry);
}

pub fn render_with_geometry(
    state: &AppState,
    frame: &mut Frame<'_>,
    geometry: &InteractionGeometry,
) {
    let status_height = u16::from(!state.running_commands.is_empty());
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(status_height),
        ])
        .split(frame.area());

    render_provider_bar(state, frame, rows[0]);
    render_running_command_status(state, frame, rows[2]);

    let Some(provider) = state.active_workspace() else {
        frame.render_widget(
            Paragraph::new("No providers discovered")
                .block(Block::default().title(" Workspace ").borders(Borders::ALL)),
            rows[1],
        );
        return;
    };

    let columns = [
        geometry
            .resources
            .expect("workspace geometry has resources"),
        geometry.details.expect("workspace geometry has details"),
    ];
    let presentation = provider.presentation();
    let workspace_view = match &presentation {
        WorkspacePresentation::Ready(view) => Some(view),
        WorkspacePresentation::Loading { .. } | WorkspacePresentation::Error { .. } => None,
    };
    render_workspace_panel(
        &presentation,
        matches!(&state.focused_pane, FocusedPane::Resources),
        &state.hints.focus_resource_panels,
        frame,
        columns[0],
    );
    render_details_panel(
        provider.name(),
        workspace_view,
        state.focused_pane == FocusedPane::Details,
        state.hints.focus_details.as_deref(),
        frame,
        columns[1],
    );

    if let Some(help) = &state.help_overlay {
        let area = centered_rect(42, (help.entries.len() as u16 + 2).max(4), frame.area());
        let lines = help
            .entries
            .iter()
            .map(|entry| Line::from(format!("{}  {}", entry.key, entry.description)))
            .collect::<Vec<_>>();
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .title(format!(" Commands for {} ", help.target))
                    .borders(Borders::ALL),
            ),
            area,
        );
    }

    if let Some(error) = &state.command_error {
        // Narrow terminals wrap the message instead of clipping it: an error
        // that cannot name its Provider, Resource, and Command is not an
        // identifying one.
        let message_width = error.chars().count() as u16;
        let width = (message_width + 4).min(frame.area().width);
        let wrapped_lines = message_width.div_ceil(width.saturating_sub(2).max(1));
        let area = centered_rect(width, wrapped_lines + 3, frame.area());
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(error.as_str(), Style::default().fg(Color::Red)),
                Line::from("Press Esc to dismiss."),
            ])
            .wrap(ratatui::widgets::Wrap { trim: true })
            .block(
                Block::default()
                    .title(" Command failed ")
                    .borders(Borders::ALL),
            ),
            area,
        );
    }

    if let Some(confirmation) = &state.confirmation {
        let area = centered_rect(64, 5, frame.area());
        frame.render_widget(Clear, area);
        let mut lines = vec![Line::from(format!(
            "Delete {} resource {} ({})?",
            confirmation.provider_name, confirmation.resource_name, confirmation.target
        ))];
        // Deleting anything but a stopped Resource stops it first, so say so
        // before the single confirmation that authorises both. The wording
        // stays on the outcome: a paused or restarting Resource is not running,
        // but removing it still stops it.
        if confirmation.state != ResourceState::Stopped {
            lines.push(Line::from("It will be stopped and removed."));
        }
        lines.push(Line::from("Press y/Enter to confirm or n/Esc to cancel."));
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .title(" Confirm deletion ")
                    .borders(Borders::ALL),
            ),
            area,
        );
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

/// Shows every Resource Command still running, wherever the user has navigated.
///
/// Each entry names the Provider, Resource, and Command it was dispatched for,
/// so the status identifies its target even while another Provider Workspace
/// is active.
fn render_running_command_status(state: &AppState, frame: &mut Frame<'_>, area: Rect) {
    if state.running_commands.is_empty() {
        return;
    }
    let status = state
        .running_commands
        .iter()
        .map(|running| {
            format!(
                "Running {} {} for {} ({})…",
                running.provider_name, running.command, running.resource_name, running.target
            )
        })
        .collect::<Vec<_>>()
        .join("   ");
    frame.render_widget(
        Paragraph::new(Line::styled(
            status,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        area,
    );
}

fn render_provider_bar(state: &AppState, frame: &mut Frame<'_>, area: Rect) {
    let mut provider_spans = vec![
        Span::styled(
            provider_selector_label(state),
            panel_title_style(state.focused_pane == FocusedPane::Providers),
        ),
        Span::raw(" ".repeat(PROVIDER_LABEL_GAP as usize)),
    ];
    for (index, provider) in state.providers.iter().enumerate() {
        if index > 0 {
            provider_spans.push(Span::raw(" ".repeat(PROVIDER_WORKSPACE_GAP as usize)));
        }
        let active = Some(index) == state.active_provider;
        let label = provider_workspace_label(provider, active);
        provider_spans.push(if active {
            Span::styled(label, Style::default().add_modifier(Modifier::BOLD))
        } else {
            Span::raw(label)
        });
    }
    frame.render_widget(Paragraph::new(Line::from(provider_spans)), area);
}

fn render_workspace_panel(
    presentation: &WorkspacePresentation<'_>,
    resource_focus: bool,
    resource_hints: &[Option<String>],
    frame: &mut Frame<'_>,
    area: Rect,
) {
    match presentation {
        WorkspacePresentation::Loading { .. } => frame.render_widget(
            Paragraph::new("Refreshing…").block(pane_block(
                pane_title(
                    resource_hints.first().and_then(Option::as_deref),
                    "Resources",
                    resource_focus,
                ),
                resource_focus,
            )),
            area,
        ),
        WorkspacePresentation::Error { name, error } => frame.render_widget(
            Paragraph::new(vec![
                Line::styled(
                    format!("{name} provider is unavailable"),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Line::from(error.message.as_str()),
            ])
            .wrap(ratatui::widgets::Wrap { trim: true })
            .block(pane_block(
                pane_title(
                    resource_hints.first().and_then(Option::as_deref),
                    "Error",
                    resource_focus,
                ),
                resource_focus,
            )),
            area,
        ),
        WorkspacePresentation::Ready(view) => {
            if view.panels().len() == 0 {
                frame.render_widget(
                    Paragraph::new("No Resource Panels available").block(pane_block(
                        pane_title(
                            resource_hints.first().and_then(Option::as_deref),
                            "Resources",
                            resource_focus,
                        ),
                        resource_focus,
                    )),
                    area,
                );
                return;
            }
            // Panels share the height in proportion to how much they have to
            // show, so a provider's one busy panel is not squeezed to make room
            // for an empty one. Every panel keeps its border and a first row,
            // so none can be crowded out entirely.
            let panel_areas = Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    view.panels()
                        .map(|panel| Constraint::Fill(panel.panel.resources.len().max(1) as u16)),
                )
                .split(area);
            for (index, (panel, area)) in view.panels().zip(panel_areas.iter().copied()).enumerate()
            {
                let focused =
                    resource_focus && view.focused_resource_panel == Some(&panel.panel.id);
                render_resource_panel(
                    view.name,
                    &panel,
                    resource_hints.get(index).and_then(Option::as_deref),
                    focused,
                    frame,
                    area,
                );
            }
        }
    }
}

fn render_resource_panel(
    provider_name: &str,
    view: &ResourcePanelView<'_>,
    resources_hint: Option<&str>,
    focused: bool,
    frame: &mut Frame<'_>,
    area: Rect,
) {
    let panel = view.panel;
    let viewport_height = area.height.saturating_sub(2) as usize;
    let visible =
        visible_resource_range(view.selected_index, viewport_height, panel.resources.len());
    // A row names its Resource and, where the Resource has one, says what it is
    // doing. Everything else a Provider reported is in the Details pane, so a
    // row never has to compete for width with it.
    let mut items = panel
        .resources
        .get(visible)
        .unwrap_or_default()
        .iter()
        .map(|resource| {
            let marker = if view.selected_resource == Some(&resource.id) {
                ">"
            } else {
                " "
            };
            let mut spans = vec![
                Span::raw(marker),
                Span::raw(" "),
                Span::raw(resource.name.as_str()),
            ];
            if let Some(status) = &resource.status {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    status.as_str(),
                    resource
                        .state
                        .map_or_else(Style::default, resource_state_style),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        items.push(ListItem::new(format!(
            "No {} {} found",
            provider_name,
            panel.title.to_lowercase()
        )));
    }
    frame.render_widget(
        List::new(items).block(pane_block(
            pane_title(resources_hint, &panel.title, focused),
            focused,
        )),
        area,
    );
}

fn visible_resource_range(
    selected_index: usize,
    viewport_height: usize,
    resource_count: usize,
) -> std::ops::Range<usize> {
    let visible_count = viewport_height.min(resource_count);
    if visible_count == 0 {
        return 0..0;
    }
    let selected_index = selected_index.min(resource_count - 1);
    let start = selected_index
        .saturating_add(1)
        .saturating_sub(visible_count)
        .min(resource_count - visible_count);
    start..start + visible_count
}

/// Colours a Resource's status by its Resource State, so a paused or broken
/// Resource is distinguishable without reading the text.
///
/// `Unknown` is deliberately left neutral: a status this Provider Workspace
/// does not recognise must not borrow the colour of a state Virtui understands.
fn resource_state_style(state: ResourceState) -> Style {
    let colour = match state {
        ResourceState::Running => Color::Green,
        ResourceState::Stopped => Color::DarkGray,
        ResourceState::Paused => Color::Yellow,
        ResourceState::Transitioning => Color::Blue,
        ResourceState::Broken => Color::Red,
        ResourceState::Unknown => Color::Reset,
    };
    Style::default().fg(colour)
}

fn panel_title_style(focused: bool) -> Style {
    if focused {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    }
}

/// Builds a workspace panel title that prefixes the focus key, or shows only the
/// label when the focus Command is unbound.
fn pane_title(hint: Option<&str>, label: &str, focused: bool) -> String {
    let title = match hint {
        Some(key) => format!(" [{key}] {label} "),
        None => format!(" {label} "),
    };
    if focused {
        format!(" ▶{title}")
    } else {
        title
    }
}

fn pane_block(title: String, focused: bool) -> Block<'static> {
    Block::default()
        .title(title)
        .title_style(panel_title_style(focused))
        .border_style(panel_title_style(focused))
        .border_type(if focused {
            BorderType::Thick
        } else {
            BorderType::Plain
        })
        .borders(Borders::ALL)
}

fn render_details_panel(
    provider_name: &str,
    view: Option<&WorkspaceView<'_>>,
    focused: bool,
    details_hint: Option<&str>,
    frame: &mut Frame<'_>,
    area: Rect,
) {
    let summary = view
        .and_then(|view| view.selected_resource)
        .map(|resource| {
            let mut lines = vec![Line::styled(
                resource.name.as_str(),
                Style::default().add_modifier(Modifier::BOLD),
            )];
            lines.extend(resource.fields.iter().map(|(label, value)| {
                Line::from(vec![
                    Span::raw(*label),
                    Span::raw(": "),
                    Span::raw(value.as_str()),
                ])
            }));
            lines
        })
        .unwrap_or_else(|| {
            if view.is_some() {
                vec![Line::from("Select a resource")]
            } else {
                vec![Line::from("No details available")]
            }
        });

    let block = pane_block(pane_title(details_hint, "Details", focused), focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let views = view.map_or(&[][..], |view| view.detail_views);
    // The view strip is a control, not another summary field, so it gets a
    // blank line to sit behind rather than running straight into the fields.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(summary.len() as u16),
            Constraint::Length(u16::from(!views.is_empty()) * 2),
            Constraint::Min(0),
        ])
        .split(inner);
    frame.render_widget(
        Paragraph::new(summary).wrap(ratatui::widgets::Wrap { trim: true }),
        rows[0],
    );

    if views.is_empty() {
        return;
    }
    if let Some(details) = view.and_then(|view| view.details) {
        render_detail_content(provider_name, details, frame, rows[2]);
    }
    let mut spans = Vec::new();
    for detail_view in views {
        if !spans.is_empty() {
            spans.push(Span::raw("  "));
        }
        if view
            .and_then(|workspace| workspace.selected_detail_view)
            .is_some_and(|selected| selected.id == detail_view.id)
        {
            spans.push(Span::styled(
                format!("[ {} ]", detail_view.title),
                Style::default().add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::raw(detail_view.title.as_str()));
        }
    }
    frame.render_widget(
        Paragraph::new(vec![Line::default(), Line::from(spans)]),
        rows[1],
    );
}

/// Draws the loaded detail view, or what is happening to it.
///
/// An empty view and a failed one are told apart deliberately: a container that
/// has logged nothing is not a broken one, and neither must read as the other.
fn render_detail_content(
    provider_name: &str,
    details: ResourceDetailsView<'_>,
    frame: &mut Frame<'_>,
    area: Rect,
) {
    let lines = match details.content {
        DetailContent::Loading => vec![Line::from(format!("Loading {}…", details.title))],
        DetailContent::Ready(loaded) if loaded.is_empty() => vec![Line::styled(
            format!(
                "{} returned no {} for {}",
                provider_name, details.title, details.resource_name
            ),
            Style::default().fg(Color::DarkGray),
        )],
        DetailContent::Ready(loaded) => loaded
            .lines
            .iter()
            .skip(details.scroll as usize)
            .take(area.height as usize)
            .map(|line| Line::from(line.as_str()))
            .collect(),
        DetailContent::Error(error) => vec![
            Line::styled(
                format!(
                    "{} {} failed for {}:",
                    provider_name, details.title, details.resource_name
                ),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Line::from(error.message.as_str()),
        ],
    };
    frame.render_widget(Paragraph::new(lines), area);
}

pub fn render_to_text(state: &AppState, width: u16, height: u16) -> String {
    render_to_buffer(state, width, height, |cell| cell.symbol().to_owned())
        .into_iter()
        .map(|row| row.concat())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Renders one cell per screen position, reporting only its foreground colour.
///
/// This is the colour counterpart of [`render_to_text`]: the two share a
/// coordinate system, so a test can locate text in one and read its colour from
/// the other.
pub fn render_foreground_colours(state: &AppState, width: u16, height: u16) -> Vec<Vec<Color>> {
    render_to_buffer(state, width, height, |cell| cell.fg)
}

fn render_to_buffer<T>(
    state: &AppState,
    width: u16,
    height: u16,
    read: impl Fn(&Cell) -> T,
) -> Vec<Vec<T>> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal.draw(|frame| render(state, frame)).expect("draw");
    let buffer = terminal.backend().buffer();

    (0..height)
        .map(|y| (0..width).map(|x| read(&buffer[(x, y)])).collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_viewport_contains_only_rows_ending_at_the_selection() {
        assert_eq!(visible_resource_range(8, 3, 12), 6..9);
        assert_eq!(visible_resource_range(1, 3, 12), 0..3);
        assert_eq!(visible_resource_range(11, 3, 12), 9..12);
    }

    #[test]
    fn focused_panel_titles_are_visually_distinct() {
        assert_eq!(
            panel_title_style(true),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        );
        assert_eq!(panel_title_style(false), Style::default());
    }

    /// Hit-testing is only honest if it agrees with what the user sees, so this
    /// compares measured regions against the drawn screen rather than against a
    /// second copy of the same arithmetic.
    #[test]
    fn measured_provider_workspaces_sit_where_they_are_drawn() {
        use crate::application::ProviderWorkspaceState;
        use crate::domain::{Provider, ProviderId};

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

        let geometry = interaction_geometry(&state, Rect::new(0, 0, 80, 24));
        let screen = render_to_text(&state, 80, 24);
        let bar = screen.lines().next().expect("a provider bar is drawn");

        let drawn = bar.find("Incus").expect("the Provider Workspace is drawn") as u16;
        let (measured, _) = geometry.provider_tabs[1];

        assert_eq!(
            measured.x, drawn,
            "a click lands where the Provider Workspace is drawn, not where a second calculation guessed"
        );
    }

    #[test]
    fn geometry_handles_empty_workspace_without_panics() {
        let geometry = interaction_geometry(&AppState::default(), Rect::new(0, 0, 80, 24));
        assert_eq!(
            geometry.hit(0, 0),
            Some(InteractionTarget::ProviderSelector)
        );
        assert_eq!(geometry.hit(79, 23), None);
    }
}
