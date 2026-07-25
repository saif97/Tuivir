use ratatui::{
    Frame, Terminal,
    backend::TestBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};

use crate::app::{AppState, FocusedPanel, ProviderState, WorkspaceState};
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
    let mut provider_spans = vec![
        Span::styled(
            "[1] Providers",
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
                    .title(" [2] Resources ")
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
                    .title(" [2] Error ")
                    .title_style(title_style)
                    .borders(Borders::ALL),
            ),
            workspace_rows[1],
        ),
        WorkspaceState::Ready(snapshot) => {
            // Providers currently populate exactly one panel; only the first is
            // shown here. Selection/navigation elsewhere already walks every
            // panel via `WorkspaceSnapshot::resources`, so a future
            // multi-panel provider would need this to do the same.
            let panel = snapshot.panels.first();
            let title = panel.map_or("Resources", |panel| panel.title.as_str());
            let items = panel
                .into_iter()
                .flat_map(|panel| &panel.resources)
                .map(|resource| {
                    let status = resource.status.as_deref().unwrap_or("");
                    let marker = if provider.selected_resource.as_ref() == Some(&resource.id) {
                        ">"
                    } else {
                        " "
                    };
                    ListItem::new(format!("{marker} {}  {status}", resource.name))
                })
                .collect::<Vec<_>>();
            let items = if items.is_empty() {
                vec![ListItem::new(format!(
                    "No {} {} found",
                    provider.name,
                    title.to_lowercase()
                ))]
            } else {
                items
            };
            frame.render_widget(
                List::new(items).block(
                    Block::default()
                        .title(format!(" [2] {title} "))
                        .title_style(title_style)
                        .borders(Borders::ALL),
                ),
                workspace_rows[1],
            );
        }
    }
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

fn render_details_panel(provider: &ProviderState, frame: &mut Frame<'_>, area: Rect) {
    let details = match &provider.workspace_state {
        WorkspaceState::Ready(snapshot) => snapshot
            .resources()
            .find(|resource| provider.selected_resource.as_ref() == Some(&resource.id))
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
    frame.render_widget(
        Paragraph::new(details)
            .wrap(ratatui::widgets::Wrap { trim: true })
            .block(Block::default().title(" Details ").borders(Borders::ALL)),
        area,
    );
}

pub fn render_to_text(state: &AppState, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal.draw(|frame| render(state, frame)).expect("draw");
    let buffer = terminal.backend().buffer();

    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
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
