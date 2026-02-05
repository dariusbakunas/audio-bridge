use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

pub(crate) fn modal_block(title: &str) -> Block<'_> {
    Block::default().title(title).borders(Borders::ALL)
}

pub(crate) fn draw_modal_text<'a>(title: &'a str, body: &'a str) -> Paragraph<'a> {
    Paragraph::new(body).block(modal_block(title))
}

pub(crate) fn draw_list_panel<'a>(title: &'a str, items: Vec<ListItem<'a>>, highlight: bool) -> List<'a> {
    let mut list = List::new(items).block(modal_block(title));
    if highlight {
        list = list
            .highlight_style(Style::default().add_modifier(Modifier::BOLD))
            .highlight_symbol("▶ ");
    }
    list
}
