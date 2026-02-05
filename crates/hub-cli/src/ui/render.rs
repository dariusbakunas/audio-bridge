use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, Paragraph},
};

use crate::ui::view_model::UiView;

use super::app::App;

pub(crate) fn draw(f: &mut ratatui::Frame, app: &mut App) {
    app.refresh_auto_preview_if_needed();

    let view = UiView::from_app(app);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(16), Constraint::Min(5), Constraint::Length(7)])
        .split(f.area());

    let top_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(6), Constraint::Length(10)])
        .split(chunks[0]);

    let header = Paragraph::new(
        view
            .header_lines
            .iter()
            .cloned()
            .map(Line::from)
            .collect::<Vec<_>>(),
    )
    .block(Block::default().borders(Borders::ALL).title("Target"));
    f.render_widget(header, top_chunks[0]);

    let now_playing = Paragraph::new(
        view
            .now_playing_summary
            .iter()
            .cloned()
            .map(Line::from)
            .collect::<Vec<_>>(),
    )
    .block(Block::default().borders(Borders::ALL).title("Now Playing"));
    f.render_widget(now_playing, top_chunks[1]);

    let items: Vec<ListItem> = view
        .entry_labels
        .iter()
        .cloned()
        .map(ListItem::new)
        .collect();

    let mid_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(chunks[1]);

    let list_block = Block::default().borders(Borders::ALL).title("Entries");
    app.list_view_height = list_block.inner(mid_chunks[0]).height as usize;
    let list = List::new(items)
        .block(list_block)
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol("▶ ");
    f.render_stateful_widget(list, mid_chunks[0], &mut app.list_state);

    let queue_width = mid_chunks[1].width as usize;

    let queued_items: Vec<ListItem> = if view.queue_labels.is_empty() {
        vec![ListItem::new("<empty>")]
    } else {
        view
            .queue_labels
            .iter()
            .map(|label| ListItem::new(truncate_label(label, queue_width.saturating_sub(1))))
            .collect()
    };

    let queue_list = List::new(queued_items)
        .block(Block::default().borders(Borders::ALL).title("Up Next"))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    f.render_widget(queue_list, mid_chunks[1]);

    let footer_block = Block::default().borders(Borders::ALL).title("Status");
    let footer_inner = footer_block.inner(chunks[2]);
    f.render_widget(footer_block, chunks[2]);

    let footer_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(footer_inner);

    f.render_widget(
        Paragraph::new(Line::from(view.status_line.clone())),
        footer_chunks[0],
    );
    f.render_widget(Paragraph::new(Line::from(view.remote_status.clone())), footer_chunks[1]);
    if let Some((ratio, label)) = view.remote_gauge.clone() {
        let gauge_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(10), Constraint::Length(50)])
            .split(footer_chunks[2]);
        let width = gauge_chunks[0].width as usize;
        if width > 0 {
            let played_cells = (ratio * width as f64).round() as usize;
            let buffer_cells = view.buffer_ratio
                .map(|r| (r * width as f64).round() as usize)
                .unwrap_or(0)
                .max(played_cells)
                .min(width);
            let mut spans = Vec::new();
            if played_cells > 0 {
                spans.push(Span::styled(
                    " ".repeat(played_cells),
                    Style::default().bg(Color::White).fg(Color::Black),
                ));
            }
            if buffer_cells > played_cells {
                spans.push(Span::styled(
                    " ".repeat(buffer_cells - played_cells),
                    Style::default().bg(Color::DarkGray).fg(Color::Black),
                ));
            }
            if width > buffer_cells {
                spans.push(Span::styled(
                    " ".repeat(width - buffer_cells),
                    Style::default().bg(Color::Black).fg(Color::Black),
                ));
            }
            f.render_widget(Paragraph::new(Line::from(spans)), gauge_chunks[0]);
        } else {
            let gauge = Gauge::default()
                .ratio(ratio)
                .style(Style::default().fg(Color::Black).bg(Color::White))
                .gauge_style(Style::default().fg(Color::White).bg(Color::Black));
            f.render_widget(gauge, gauge_chunks[0]);
        }
        f.render_widget(
            Paragraph::new(Line::from(label)).alignment(Alignment::Right),
            gauge_chunks[1],
        );
    } else {
        f.render_widget(
            Paragraph::new(Line::from("remote progress: -")),
            footer_chunks[2],
        );
    }
    f.render_widget(
        Paragraph::new(Line::from(
            view.keys_line.clone(),
        )),
        footer_chunks[3],
    );

    if let Some(panel) = &view.now_playing_panel {
        let area = centered_rect(70, 70, f.area());
        f.render_widget(Clear, area);
        let block = Block::default().title(panel.title.as_str()).borders(Borders::ALL);
        f.render_widget(Paragraph::new(panel.body.as_str()).block(block), area);
    }

    if app.help_open {
        let area = centered_rect(70, 70, f.area());
        f.render_widget(Clear, area);
        let help = view.help_lines.join("\n");
        let block = Block::default().title("Help").borders(Borders::ALL);
        f.render_widget(Paragraph::new(help).block(block), area);
    }

    if app.outputs_open {
        let area = centered_rect(60, 60, f.area());
        f.render_widget(Clear, area);
        let mut items = Vec::new();
        if let Some(err) = view.outputs_error.as_ref() {
            items.push(ListItem::new(format!("error: {err}")));
        }
        if view.outputs_empty {
            items.push(ListItem::new("<no outputs>"));
        } else {
            for label in &view.outputs_labels {
                items.push(ListItem::new(label.clone()));
            }
        }
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Select Output (Enter to apply, Esc to close)"))
            .highlight_style(Style::default().add_modifier(Modifier::BOLD))
            .highlight_symbol("▶ ");
        f.render_stateful_widget(list, area, &mut app.outputs_state);
    }

    if app.logs_open {
        let area = centered_rect(90, 80, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Logs (Esc to close, ↑/↓ scroll)");
        let inner = block.inner(area);
        let height = inner.height as usize;
        let total = app.logs.len();
        let end = total.saturating_sub(app.logs_scroll);
        let start = end.saturating_sub(height);
        let mut items = Vec::new();
        for line in app.logs.iter().skip(start).take(end.saturating_sub(start)) {
            items.push(ListItem::new(line.clone()));
        }
        if items.is_empty() {
            items.push(ListItem::new("<no logs>"));
        }
        let list = List::new(items).block(block);
        f.render_widget(list, area);
    }

    if app.confirm_clear_queue {
        let area = centered_rect(40, 25, f.area());
        f.render_widget(Clear, area);
        let body = ["Clear entire queue?", "", "Press y to confirm, n to cancel"]
            .join("\n");
        let block = Block::default().title("Clear Queue").borders(Borders::ALL);
        f.render_widget(Paragraph::new(body).block(block), area);
    }
}

fn truncate_label(label: &str, max: usize) -> String {
    if max == 0 || label.len() <= max {
        return label.to_string();
    }
    if max <= 3 {
        return label[..max].to_string();
    }
    let cut = max - 3;
    format!("{}...", &label[..cut])
}

fn centered_rect(percent_x: u16, percent_y: u16, r: ratatui::layout::Rect) -> ratatui::layout::Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1]);
    horizontal[1]
}
