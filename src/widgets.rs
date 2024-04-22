use std::borrow::Cow;

use iced::{
    widget::{
        scrollable::{Alignment, Direction, Properties},
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

pub fn button<'a, Message>(content: impl Into<Cow<'a, str>>) -> Button<'a, Message> {
    return Button::new(
        Text::new(content)
            .size(DEFAULT_FONT_SIZE)
            // .font(Font::DEFAULT)
            .font(style::bold_font())
            .horizontal_alignment(iced::alignment::Horizontal::Center)
            .vertical_alignment(iced::alignment::Vertical::Center),
    )
    .height(30)
    .padding(5)
    .style(iced::theme::Button::Custom(Box::new(
        style::MyButtonStyle {},
    )));
}
