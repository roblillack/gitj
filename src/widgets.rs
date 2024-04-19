use iced::{
    widget::{
        scrollable::{Alignment, Direction, Properties},
        Button, Container, Scrollable,
    },
    Element, Padding,
};

use crate::style;

pub fn listbox<'a, Message>(content: impl Into<Element<'a, Message>>) -> Scrollable<'a, Message>
where
    Message: 'a + Clone,
{
    return Scrollable::new(Container::new(content).padding(Padding {
        top: 3.0,
        right: 20. + 5.,
        bottom: 3.,
        left: 5.,
    }))
    .style(iced::theme::Scrollable::Custom(Box::new(
        style::MyButtonStyle {},
    )))
    .direction(Direction::Vertical(
        Properties::new()
            .width(20)
            .margin(0)
            .scroller_width(20)
            .alignment(Alignment::Start),
    ));
}

pub fn button<'a, Message>(content: impl Into<Element<'a, Message>>) -> Button<'a, Message> {
    return Button::new(content)
        .height(30)
        .style(iced::theme::Button::Custom(Box::new(
            style::MyButtonStyle {},
        )));
}
