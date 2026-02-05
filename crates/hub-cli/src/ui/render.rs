use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, ListItem, Paragraph},
};

use crate::ui::layout::centered_rect;
use crate::ui::widgets::{draw_list_panel, draw_modal_text, modal_block};
use crate::ui::view_model::{ModalLayout, UiModal, UiView};

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
    .block(modal_block("Target"));
    f.render_widget(header, top_chunks[0]);

    let now_playing = Paragraph::new(
        view
            .now_playing_summary
            .iter()
            .cloned()
            .map(Line::from)
            .collect::<Vec<_>>(),
    )
    .block(modal_block("Now Playing"));
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

    let list_block = modal_block("Entries");
    app.list_view_height = list_block.inner(mid_chunks[0]).height as usize;
    let list = draw_list_panel("Entries", items, true);
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

    let queue_list = draw_list_panel("Up Next", queued_items, false);

    f.render_widget(queue_list, mid_chunks[1]);

    let footer_block = modal_block("Status");
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

    if let Some(modal) = &view.active_modal {
        match modal {
            UiModal::Help { title, body, layout } => {
                let area = modal_rect(layout, f.area());
                f.render_widget(Clear, area);
                f.render_widget(draw_modal_text(title.as_str(), body.as_str()), area);
            }
            UiModal::Outputs { title, items, empty, error, layout } => {
                let area = modal_rect(layout, f.area());
                f.render_widget(Clear, area);
                let mut list_items = Vec::new();
                if let Some(err) = error.as_ref() {
                    list_items.push(ListItem::new(format!("error: {err}")));
                }
                if *empty {
                    list_items.push(ListItem::new("<no outputs>"));
                } else {
                    for label in items {
                        list_items.push(ListItem::new(label.clone()));
                    }
                }
                let list = draw_list_panel(title.as_str(), list_items, true);
                f.render_stateful_widget(list, area, &mut app.outputs_state);
            }
            UiModal::Logs { title, empty, layout } => {
                let area = modal_rect(layout, f.area());
                f.render_widget(Clear, area);
                let block = modal_block(title.as_str());
                let inner = block.inner(area);
                let height = inner.height as usize;
                let total = app.logs.len();
                let (start, end) = if total <= height {
                    (0, total)
                } else {
                    let max_scroll = total.saturating_sub(height);
                    let scroll = app.logs_scroll.min(max_scroll);
                    let end = total.saturating_sub(scroll);
                    let start = end.saturating_sub(height);
                    (start, end)
                };
                let mut list_items = Vec::new();
                for line in app.logs.iter().skip(start).take(end.saturating_sub(start)) {
                    list_items.push(ListItem::new(line.clone()));
                }
                if *empty || list_items.is_empty() {
                    list_items.push(ListItem::new("<no logs>"));
                }
                let list = draw_list_panel(title.as_str(), list_items, false).block(block);
                f.render_widget(list, area);
            }
            UiModal::ClearQueue { title, body, layout } => {
                let area = modal_rect(layout, f.area());
                f.render_widget(Clear, area);
                f.render_widget(draw_modal_text(title.as_str(), body.as_str()), area);
            }
        }
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

fn modal_rect(layout: &ModalLayout, r: ratatui::layout::Rect) -> ratatui::layout::Rect {
    centered_rect(layout.width_pct, layout.height_pct, r)
}
