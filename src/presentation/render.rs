use ratatui::{
    Frame, Terminal,
    backend::TestBackend,
    buffer::Cell,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph},
};

use crate::application::{AppState, DetailSelection, FocusedPane};
use crate::application::{
    DetailContent, ResourceDetailsView, ResourcePanelView, WorkspacePresentation, WorkspaceView,
};
use crate::domain::{ResourceState, ResourceTarget};

use super::screen_layout::{
    DETAIL_VIEW_GAP, ScreenLayout, active_target_label, command_error_area, confirmation_area,
    detail_view_label, gap, help_overlay_area, provider_selector_label, provider_workspace_label,
};

/// The default presentation palette names colours by their purpose so render
/// sites never choose unrelated terminal colours for the same meaning.
#[derive(Clone, Copy)]
enum ThemeRole {
    Primary,
    Success,
    Muted,
    Warning,
    Error,
    Selection,
    InactiveSelection,
    Terminal,
    RaisedSurface,
}

#[derive(Clone, Copy)]
pub(super) enum PaneChrome {
    Resource,
    Details,
    Full,
}

fn theme_colour(role: ThemeRole) -> Color {
    match role {
        ThemeRole::Primary => Color::Blue,
        ThemeRole::Success => Color::Green,
        ThemeRole::Muted => Color::DarkGray,
        ThemeRole::Warning => Color::Yellow,
        ThemeRole::Error => Color::Red,
        ThemeRole::Selection => Color::Blue,
        ThemeRole::InactiveSelection => Color::DarkGray,
        ThemeRole::Terminal => Color::Reset,
        ThemeRole::RaisedSurface => Color::Black,
    }
}

fn themed_style(role: ThemeRole) -> Style {
    Style::default().fg(theme_colour(role))
}

fn raised_surface_style() -> Style {
    Style::default().bg(theme_colour(ThemeRole::RaisedSurface))
}

fn modal_block(title: impl Into<Line<'static>>, accent: ThemeRole) -> Block<'static> {
    Block::default()
        .title(title)
        .title_style(themed_style(accent))
        .border_style(themed_style(accent))
        .style(raised_surface_style())
        .borders(Borders::ALL)
}

pub fn render(state: &AppState, frame: &mut Frame<'_>) {
    render_with_layout(state, frame, &ScreenLayout::measure(state, frame.area()));
}

/// Draws the screen into the regions already measured for it.
///
/// The host measures once per frame and keeps that layout for mouse routing, so
/// what the user clicks is what they see.
pub fn render_with_layout(state: &AppState, frame: &mut Frame<'_>, layout: &ScreenLayout) {
    render_provider_bar(state, frame, layout);
    render_command_bar(state, frame, layout.status);

    let (Some(provider), Some(panes)) = (state.active_workspace(), layout.panes.as_ref()) else {
        frame.render_widget(
            Paragraph::new("No providers discovered")
                .block(Block::default().title(" Workspace ").borders(Borders::ALL)),
            layout.workspace,
        );
        return;
    };

    let columns = [panes.resources, panes.details];
    let presentation = provider.presentation();
    let workspace_view = match &presentation {
        WorkspacePresentation::Ready(view) => Some(view),
        WorkspacePresentation::Loading { .. } | WorkspacePresentation::Error { .. } => None,
    };
    render_workspace_panel(
        &presentation,
        matches!(&state.focused_pane, FocusedPane::Resources),
        &state.hints.focus_resource_panels,
        &state.running_commands,
        frame,
        columns[0],
    );
    render_details_panel(
        provider.name(),
        workspace_view,
        &state.running_commands,
        state.focused_pane == FocusedPane::Details,
        state.hints.focus_details.as_deref(),
        frame,
        columns[1],
    );

    if let (Some(help), Some(area)) = (&state.help_overlay, help_overlay_area(state, frame.area()))
    {
        let lines = help
            .entries
            .iter()
            .map(|entry| Line::from(format!("{}  {}", entry.key, entry.description)))
            .collect::<Vec<_>>();
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(lines).block(modal_block(
                format!(" Commands for {} ", help.target),
                ThemeRole::Primary,
            )),
            area,
        );
    }

    if let (Some(error), Some(area)) = (
        &state.command_error,
        command_error_area(state, frame.area()),
    ) {
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(error.as_str(), themed_style(ThemeRole::Error)),
                Line::from("Press Esc to dismiss."),
            ])
            .wrap(ratatui::widgets::Wrap { trim: true })
            .block(modal_block(" Command failed ", ThemeRole::Error)),
            area,
        );
    }

    if let (Some(confirmation), Some(area)) =
        (&state.confirmation, confirmation_area(state, frame.area()))
    {
        frame.render_widget(Clear, area);
        let mut lines = vec![Line::from(format!(
            "Delete {} resource {} ({})?",
            confirmation.provider_name, confirmation.resource_name, confirmation.target
        ))];
        // Deleting anything but a stopped Resource stops it first, so say so
        // before the single confirmation that authorises both. The wording
        // stays on the outcome: a paused or restarting Resource is not running,
        // but removing it still stops it.
        match confirmation.state {
            Some(ResourceState::Stopped) => {}
            Some(_) => lines.push(Line::from("It will be stopped and removed.")),
            None => lines.push(Line::from("It will be permanently removed.")),
        }
        lines.push(Line::from("Press y/Enter to confirm or n/Esc to cancel."));
        frame.render_widget(
            Paragraph::new(lines).block(modal_block(" Confirm deletion ", ThemeRole::Warning)),
            area,
        );
    }
}

/// Shows every Resource Command still running, wherever the user has navigated.
///
/// Each entry names the Provider, Resource, and Command it was dispatched for,
/// so the status identifies its target even while another Provider Workspace
/// is active.
fn render_command_bar(state: &AppState, frame: &mut Frame<'_>, area: Rect) {
    let mut spans = state
        .command_bar
        .iter()
        .flat_map(|hint| {
            [
                Span::styled(
                    format!(" {} ", hint.key),
                    Style::default()
                        .fg(theme_colour(ThemeRole::Terminal))
                        .bg(theme_colour(ThemeRole::Primary))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(" {}  ", hint.description)),
            ]
        })
        .collect::<Vec<_>>();
    spans.push(Span::styled(
        " ? ",
        Style::default()
            .fg(theme_colour(ThemeRole::Terminal))
            .bg(theme_colour(ThemeRole::Primary))
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::raw(" all commands"));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_provider_bar(state: &AppState, frame: &mut Frame<'_>, layout: &ScreenLayout) {
    if state.providers.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                provider_selector_label(state),
                panel_title_style(state.focused_pane == FocusedPane::Providers),
            )),
            layout.provider_bar,
        );
    }
    for (index, area) in layout.provider_workspaces.iter().copied().enumerate() {
        let active = Some(index) == state.active_provider;
        let style = if active {
            Style::default()
                .fg(theme_colour(ThemeRole::Terminal))
                .bg(theme_colour(ThemeRole::Primary))
                .add_modifier(Modifier::BOLD)
        } else {
            themed_style(ThemeRole::Muted)
        };
        frame.render_widget(
            Paragraph::new(Span::styled(provider_workspace_label(state, index), style)),
            area,
        );
    }
    if let (Some(target), Some(target_area)) = (active_target_label(state), layout.active_target) {
        frame.render_widget(
            Paragraph::new(Span::styled(target, themed_style(ThemeRole::Muted)))
                .alignment(Alignment::Right),
            target_area,
        );
    }
}

fn render_workspace_panel(
    presentation: &WorkspacePresentation<'_>,
    resource_focus: bool,
    resource_hints: &[Option<String>],
    running_commands: &[crate::application::RunningResourceCommand],
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
                PaneChrome::Full,
            )),
            area,
        ),
        WorkspacePresentation::Error { name, error } => frame.render_widget(
            Paragraph::new(vec![
                Line::styled(
                    format!("{name} provider is unavailable"),
                    themed_style(ThemeRole::Error).add_modifier(Modifier::BOLD),
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
                PaneChrome::Full,
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
                        PaneChrome::Resource,
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
                    &panel,
                    resource_hints.get(index).and_then(Option::as_deref),
                    view.id,
                    running_commands,
                    focused,
                    frame,
                    area,
                );
            }
        }
    }
}

fn render_resource_panel(
    view: &ResourcePanelView<'_>,
    resources_hint: Option<&str>,
    provider_id: &crate::domain::ProviderId,
    running_commands: &[crate::application::RunningResourceCommand],
    focused: bool,
    frame: &mut Frame<'_>,
    area: Rect,
) {
    let panel = view.panel;
    let viewport_height = area.height.saturating_sub(2) as usize;
    let visible =
        visible_resource_range(view.selected_index, viewport_height, panel.resources.len());
    // A row starts with the Resource State symbol, leaving the Resource name
    // easy to scan without repeating Provider-specific status text.
    let mut items = panel
        .resources
        .get(visible.clone())
        .unwrap_or_default()
        .iter()
        .map(|resource| {
            let selected = view.selected_resource == Some(&resource.id);
            let target = ResourceTarget::new(panel.id.clone(), resource.id.clone());
            let running = running_commands
                .iter()
                .find(|running| running.provider_id == *provider_id && running.target == target);
            let state = running.map_or_else(
                || resource.state.map(resource_state_symbol).unwrap_or(" "),
                |_| "*",
            );
            let mut spans = vec![
                Span::styled(
                    state,
                    running.map_or_else(
                        || {
                            resource
                                .state
                                .map_or_else(Style::default, resource_state_style)
                        },
                        |_| themed_style(ThemeRole::Warning),
                    ),
                ),
                Span::raw(" "),
                Span::raw(resource.name.as_str()),
            ];
            if let Some(secondary_text) = &resource.secondary_text {
                spans.push(Span::raw(" · "));
                spans.push(Span::raw(secondary_text.as_str()));
            }
            let style = selected.then(|| {
                Style::default().bg(theme_colour(if focused {
                    ThemeRole::Selection
                } else {
                    ThemeRole::InactiveSelection
                }))
            });
            ListItem::new(Line::from(spans)).style(style.unwrap_or_default())
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        items.push(ListItem::new(Line::styled(
            "No resources",
            themed_style(ThemeRole::Muted),
        )));
    }
    frame.render_widget(
        List::new(items).block(pane_block(
            pane_title(
                resources_hint,
                &format!("{} ({})", panel.title, panel.resources.len()),
                focused,
            ),
            focused,
            PaneChrome::Resource,
        )),
        area,
    );
    render_resource_scrollbar(visible, viewport_height, panel.resources.len(), frame, area);
}

fn render_resource_scrollbar(
    visible: std::ops::Range<usize>,
    viewport_height: usize,
    resource_count: usize,
    frame: &mut Frame<'_>,
    area: Rect,
) {
    if resource_count <= viewport_height || viewport_height == 0 || area.width < 2 {
        return;
    }
    let thumb_height = (viewport_height * viewport_height / resource_count).max(1);
    let thumb_start = visible.start * viewport_height / resource_count;
    let lines = (0..viewport_height).map(|row| {
        Line::styled(
            if (thumb_start..thumb_start + thumb_height).contains(&row) {
                "█"
            } else {
                "░"
            },
            themed_style(ThemeRole::Muted),
        )
    });
    frame.render_widget(
        Paragraph::new(lines.collect::<Vec<_>>()),
        Rect::new(
            area.x + area.width - 2,
            area.y + 1,
            1,
            area.height.saturating_sub(2),
        ),
    );
}

fn resource_state_symbol(state: ResourceState) -> &'static str {
    match state {
        ResourceState::Running => "●",
        ResourceState::Stopped => "○",
        ResourceState::Paused => "‖",
        ResourceState::Transitioning => "↻",
        ResourceState::Broken => "✕",
        ResourceState::Unknown => "?",
    }
}

pub(super) fn visible_resource_range(
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
/// does not recognise must not borrow the colour of a state Tuivir understands.
fn resource_state_style(state: ResourceState) -> Style {
    let colour = match state {
        ResourceState::Running => theme_colour(ThemeRole::Success),
        ResourceState::Stopped => theme_colour(ThemeRole::Muted),
        ResourceState::Paused => theme_colour(ThemeRole::Warning),
        ResourceState::Transitioning => theme_colour(ThemeRole::Primary),
        ResourceState::Broken => theme_colour(ThemeRole::Error),
        ResourceState::Unknown => theme_colour(ThemeRole::Terminal),
    };
    Style::default().fg(colour)
}

fn panel_title_style(focused: bool) -> Style {
    if focused {
        themed_style(ThemeRole::Primary).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    }
}

/// Builds a workspace panel title that prefixes the focus key, or shows only the
/// label when the focus Command is unbound.
pub(super) fn pane_title(hint: Option<&str>, label: &str, focused: bool) -> String {
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

pub(super) fn pane_block(title: String, focused: bool, chrome: PaneChrome) -> Block<'static> {
    let borders = match chrome {
        PaneChrome::Resource => Borders::TOP | Borders::BOTTOM | Borders::RIGHT,
        PaneChrome::Details => Borders::TOP | Borders::BOTTOM | Borders::LEFT,
        PaneChrome::Full => Borders::ALL,
    };
    Block::default()
        .title(title)
        .title_style(panel_title_style(focused))
        .border_style(panel_title_style(focused))
        .border_type(BorderType::Plain)
        .borders(borders)
}

fn render_details_panel(
    provider_name: &str,
    view: Option<&WorkspaceView<'_>>,
    running_commands: &[crate::application::RunningResourceCommand],
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

    let block = pane_block(
        pane_title(details_hint, "Details", focused),
        focused,
        PaneChrome::Details,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let views = view.map_or(&[][..], |view| view.detail_views);
    let has_detail_tabs = view.is_some_and(|view| view.selected_resource.is_some());
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(u16::from(has_detail_tabs)),
            Constraint::Min(0),
        ])
        .split(inner);

    if !has_detail_tabs {
        frame.render_widget(
            Paragraph::new(summary).wrap(ratatui::widgets::Wrap { trim: true }),
            rows[1],
        );
        return;
    }
    let running = view.and_then(|view| {
        let target = ResourceTarget::new(
            view.focused_resource_panel?.clone(),
            view.selected_resource?.id.clone(),
        );
        running_commands
            .iter()
            .find(|running| running.provider_id == *view.id && running.target == target)
    });
    if let Some(running) = running {
        frame.render_widget(
            Paragraph::new(format!(
                "Running {} for {}…",
                running.command, running.resource_name
            ))
            .alignment(Alignment::Center)
            .style(themed_style(ThemeRole::Warning).add_modifier(Modifier::BOLD)),
            rows[1],
        );
    } else if let Some(details) = view.and_then(|view| view.details) {
        render_detail_content(provider_name, details, frame, rows[1]);
    }
    let overview_selected = view.is_some_and(|view| view.overview_selected);
    let mut spans = vec![detail_view_tab("Overview", overview_selected)];
    for detail_view in views {
        spans.push(Span::raw(gap(DETAIL_VIEW_GAP)));
        let selected = view
            .and_then(|workspace| workspace.selected_detail_view)
            .is_some_and(|selected| selected.id == detail_view.id);
        spans.push(detail_view_tab(&detail_view.title, selected));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), rows[0]);
}

fn detail_view_tab(title: &str, selected: bool) -> Span<'static> {
    let label = detail_view_label(title, selected);
    if selected {
        Span::styled(
            label,
            themed_style(ThemeRole::Primary).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(label, themed_style(ThemeRole::Muted))
    }
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
            themed_style(ThemeRole::Muted),
        )],
        DetailContent::Ready(loaded) => loaded
            .lines
            .iter()
            .enumerate()
            .skip(details.scroll as usize)
            .take(area.height as usize)
            .map(|(line_index, line)| detail_line(line, line_index as u16, details.selection))
            .collect(),
        DetailContent::Error(error) => vec![
            Line::styled(
                format!(
                    "{} {} failed for {}:",
                    provider_name, details.title, details.resource_name
                ),
                themed_style(ThemeRole::Error).add_modifier(Modifier::BOLD),
            ),
            Line::from(error.message.as_str()),
        ],
    };
    frame.render_widget(Paragraph::new(lines), area);
}

fn detail_line(text: &str, line: u16, selection: Option<&DetailSelection>) -> Line<'static> {
    let Some(selection) = selection else {
        return Line::from(text.to_owned());
    };
    let (start, end) = if selection.start <= selection.end {
        (selection.start, selection.end)
    } else {
        (selection.end, selection.start)
    };
    if start == end || line < start.line || line > end.line {
        return Line::from(text.to_owned());
    }
    let first = if line == start.line {
        start.column as usize
    } else {
        0
    };
    let last = if line == end.line {
        end.column as usize
    } else {
        text.chars().count()
    };
    let before = text.chars().take(first).collect::<String>();
    let selected = text
        .chars()
        .skip(first)
        .take(last.saturating_sub(first))
        .collect::<String>();
    let after = text.chars().skip(last).collect::<String>();
    Line::from(vec![
        Span::raw(before),
        Span::styled(selected, Style::default().fg(Color::White).bg(Color::Blue)),
        Span::raw(after),
    ])
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

/// The background counterpart of [`render_to_text`], used to verify raised
/// surfaces and interactive selection highlights.
pub fn render_background_colours(state: &AppState, width: u16, height: u16) -> Vec<Vec<Color>> {
    render_to_buffer(state, width, height, |cell| cell.bg)
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
                .fg(theme_colour(ThemeRole::Primary))
                .add_modifier(Modifier::BOLD)
        );
        assert_eq!(panel_title_style(false), Style::default());
    }
}
