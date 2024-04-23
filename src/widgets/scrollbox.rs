use std::borrow::Cow;

use iced::{
    widget::{
        scrollable::{Alignment, Direction, Properties, StyleSheet},
        Button, Container, Scrollable, Text,
    },
    Element, Font, Padding,
};

use crate::style::{self, DEFAULT_FONT_SIZE, SCROLLBAR_WIDTH};

pub fn scrollbox<'a, Message>(content: impl Into<Element<'a, Message>>) -> Scrollable<'a, Message>
where
    Message: 'a + Clone,
{
    return Scrollable::new(Container::new(content).padding(Padding {
        top: 3.0,
        right: SCROLLBAR_WIDTH as f32 + 5.,
        bottom: 3.,
        left: 5.,
    }))
    .style(iced::theme::Scrollable::Custom(Box::new(
        style::MyButtonStyle {},
    )))
    .direction(Direction::Vertical(
        Properties::new()
            .width(SCROLLBAR_WIDTH)
            .margin(0)
            .scroller_width(SCROLLBAR_WIDTH)
            .alignment(Alignment::Start),
    ));
}
