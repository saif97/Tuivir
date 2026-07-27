use ratatui::{
    Frame, Terminal,
    backend::TestBackend,
    buffer::Cell,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};

use crate::app::{AppState, DetailContent, FocusedPanel, ProviderState, WorkspaceState};
use crate::provider::ResourceState;

pub fn render(state: &AppState, frame: &mut Frame<'_>) {
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

    let Some(provider) = state
        .active_provider
        .and_then(|active_provider| state.providers.get(active_provider))
    else {
        frame.render_widget(
            Paragraph::new("No providers discovered")
                .block(Block::default().title(" Workspace ").borders(Borders::ALL)),
            rows[1],
        );
        return;
    };

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(rows[1]);
    render_workspace_panel(
        provider,
        state.focused_panel == FocusedPanel::Resources,
        state.hints.focus_resources.as_deref(),
        frame,
        columns[0],
    );
    render_details_panel(provider, frame, columns[1]);

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
            confirmation.provider_name, confirmation.resource_name, confirmation.resource_id
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
                running.provider_name, running.command, running.resource_name, running.resource_id
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
    let providers_label = match &state.hints.focus_providers {
        Some(key) => format!("[{key}] Providers"),
        None => "Providers".to_owned(),
    };
    let mut provider_spans = vec![
        Span::styled(
            providers_label,
            panel_title_style(state.focused_panel == FocusedPanel::Providers),
        ),
        Span::raw("  "),
    ];
    for (index, provider) in state.providers.iter().enumerate() {
        if index > 0 {
            provider_spans.push(Span::raw("   "));
        }
        if Some(index) == state.active_provider {
            provider_spans.push(Span::styled(
                format!("[ {} ]", provider.name),
                Style::default().add_modifier(Modifier::BOLD),
            ));
        } else {
            provider_spans.push(Span::raw(provider.name.as_str()));
        }
    }
    frame.render_widget(Paragraph::new(Line::from(provider_spans)), area);
}

fn render_workspace_panel(
    provider: &ProviderState,
    focused: bool,
    resources_hint: Option<&str>,
    frame: &mut Frame<'_>,
    area: Rect,
) {
    let title_style = panel_title_style(focused);
    let workspace_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(area);
    frame.render_widget(
        Paragraph::new(format!("Target: {}", provider.target_environment)).block(
            Block::default()
                .title(format!(" {} ", provider.name))
                .borders(Borders::ALL),
        ),
        workspace_rows[0],
    );

    match &provider.workspace_state {
        WorkspaceState::Loading => frame.render_widget(
            Paragraph::new("Refreshing…").block(
                Block::default()
                    .title(workspace_panel_title(resources_hint, "Resources"))
                    .title_style(title_style)
                    .borders(Borders::ALL),
            ),
            workspace_rows[1],
        ),
        WorkspaceState::Error(error) => frame.render_widget(
            Paragraph::new(vec![
                Line::styled(
                    format!("{} provider is unavailable", provider.name),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Line::from(error.message.as_str()),
            ])
            .wrap(ratatui::widgets::Wrap { trim: true })
            .block(
                Block::default()
                    .title(workspace_panel_title(resources_hint, "Error"))
                    .title_style(title_style)
                    .borders(Borders::ALL),
            ),
            workspace_rows[1],
        ),
        WorkspaceState::Ready(snapshot) => {
            if snapshot.panels.is_empty() {
                frame.render_widget(
                    Paragraph::new("No Resource Panels available").block(
                        Block::default()
                            .title(workspace_panel_title(resources_hint, "Resources"))
                            .title_style(title_style)
                            .borders(Borders::ALL),
                    ),
                    workspace_rows[1],
                );
                return;
            }
            let panel_areas = Layout::default()
                .direction(Direction::Vertical)
                .constraints(vec![
                    Constraint::Ratio(1, snapshot.panels.len() as u32);
                    snapshot.panels.len()
                ])
                .split(workspace_rows[1]);
            for (panel, area) in snapshot.panels.iter().zip(panel_areas.iter().copied()) {
                render_resource_panel(provider, panel, resources_hint, title_style, frame, area);
            }
        }
    }
}

fn render_resource_panel(
    provider: &ProviderState,
    panel: &crate::provider::ResourcePanel,
    resources_hint: Option<&str>,
    title_style: Style,
    frame: &mut Frame<'_>,
    area: Rect,
) {
    let mut items = Vec::new();
    if !panel.columns.is_empty() {
        items.push(ListItem::new(Line::styled(
            format!("  Name  {}", panel.columns.join("  ")),
            Style::default().add_modifier(Modifier::BOLD),
        )));
    }
    items.extend(panel.resources.iter().map(|resource| {
        let marker = if provider.selected_resource.as_ref().is_some_and(|selected| {
            selected.panel_id == panel.id && selected.resource_id == resource.id
        }) {
            ">"
        } else {
            " "
        };
        let values = panel
            .columns
            .iter()
            .map(|column| {
                resource
                    .fields
                    .iter()
                    .find_map(|(label, value)| (label == column).then_some(value.as_str()))
                    .unwrap_or("")
            })
            .collect::<Vec<_>>()
            .join("  ");
        let mut spans = vec![Span::raw(format!("{marker} {}  {values}", resource.name))];
        if let Some(status) = &resource.status {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                status.clone(),
                resource
                    .state
                    .map_or_else(Style::default, resource_state_style),
            ));
        }
        ListItem::new(Line::from(spans))
    }));
    if panel.resources.is_empty() {
        items.push(ListItem::new(format!(
            "No {} {} found",
            provider.name,
            panel.title.to_lowercase()
        )));
    }
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title(workspace_panel_title(resources_hint, &panel.title))
                .title_style(title_style)
                .borders(Borders::ALL),
        ),
        area,
    );
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
fn workspace_panel_title(resources_hint: Option<&str>, label: &str) -> String {
    match resources_hint {
        Some(key) => format!(" [{key}] {label} "),
        None => format!(" {label} "),
    }
}

fn render_details_panel(provider: &ProviderState, frame: &mut Frame<'_>, area: Rect) {
    let summary = match &provider.workspace_state {
        WorkspaceState::Ready(snapshot) => provider
            .selected_resource
            .as_ref()
            .and_then(|selected| snapshot.resource(&selected.panel_id, &selected.resource_id))
            .map(|resource| {
                let mut lines = vec![Line::styled(
                    resource.name.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                )];
                lines.extend(
                    resource
                        .fields
                        .iter()
                        .map(|(label, value)| Line::from(format!("{label}: {value}"))),
                );
                lines
            })
            .unwrap_or_else(|| vec![Line::from("Select a resource")]),
        _ => vec![Line::from("No details available")],
    };

    let block = Block::default().title(" Details ").borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let views = provider.detail_views();
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
    render_detail_content(provider, frame, rows[2]);
    let mut spans = Vec::new();
    for view in views {
        if !spans.is_empty() {
            spans.push(Span::raw("  "));
        }
        if provider.selected_detail_view.as_ref() == Some(&view.id) {
            spans.push(Span::styled(
                format!("[ {} ]", view.title),
                Style::default().add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::raw(view.title.as_str()));
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
fn render_detail_content(provider: &ProviderState, frame: &mut Frame<'_>, area: Rect) {
    let Some(details) = &provider.details else {
        return;
    };
    let lines = match &details.content {
        DetailContent::Loading => vec![Line::from(format!("Loading {}…", details.title))],
        DetailContent::Ready(loaded) if loaded.is_empty() => vec![Line::styled(
            format!(
                "{} returned no {} for {}",
                provider.name, details.title, details.resource_name
            ),
            Style::default().fg(Color::DarkGray),
        )],
        DetailContent::Ready(loaded) => loaded
            .lines
            .iter()
            .map(|line| Line::from(line.as_str()))
            .collect(),
        DetailContent::Error(error) => vec![
            Line::styled(
                format!(
                    "{} {} failed for {}:",
                    provider.name, details.title, details.resource_name
                ),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Line::from(error.message.as_str()),
        ],
    };
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(ratatui::widgets::Wrap { trim: false })
            .scroll((details.scroll, 0)),
        area,
    );
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
    fn focused_panel_titles_are_visually_distinct() {
        assert_eq!(
            panel_title_style(true),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        );
        assert_eq!(panel_title_style(false), Style::default());
    }
}
