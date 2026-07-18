use ratatui::{
    Frame, Terminal,
    backend::TestBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use crate::app::{AppState, ProviderState, WorkspaceState};

pub fn render(state: &AppState, frame: &mut Frame<'_>) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(frame.area());

    render_provider_bar(state, frame, rows[0]);

    let Some(provider) = state
        .active_workspace
        .and_then(|active_workspace| state.providers.get(active_workspace))
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
    render_workspace_panel(provider, frame, columns[0]);
    render_details_panel(provider, frame, columns[1]);
}

fn render_provider_bar(state: &AppState, frame: &mut Frame<'_>, area: Rect) {
    let mut provider_spans = vec![
        Span::styled(
            "Providers",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
    ];
    for (index, provider) in state.providers.iter().enumerate() {
        if index > 0 {
            provider_spans.push(Span::raw("   "));
        }
        if Some(index) == state.active_workspace {
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

fn render_workspace_panel(provider: &ProviderState, frame: &mut Frame<'_>, area: Rect) {
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

    match &provider.workspace {
        WorkspaceState::Loading => frame.render_widget(
            Paragraph::new("Refreshing…")
                .block(Block::default().title(" Resources ").borders(Borders::ALL)),
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
            .block(Block::default().title(" Error ").borders(Borders::ALL)),
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
                        .title(format!(" {title} "))
                        .borders(Borders::ALL),
                ),
                workspace_rows[1],
            );
        }
    }
}

fn render_details_panel(provider: &ProviderState, frame: &mut Frame<'_>, area: Rect) {
    let details = match &provider.workspace {
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
        Paragraph::new(details).block(Block::default().title(" Details ").borders(Borders::ALL)),
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
