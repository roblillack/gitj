use backend::{Backend, BackendError};
use git2::Error;
use iced::theme::Button::Custom;
use iced::widget::{
    button, checkbox, column, container, horizontal_rule, pick_list, progress_bar, row, scrollable,
    slider, text, text_input, toggler, vertical_rule, vertical_space, Button, Column, Scrollable,
};
use iced::{font, Alignment, Element, Font, Length, Sandbox, Settings, Theme};
use iced_aw::{SelectionList, SelectionListStyles};

mod backend;
mod style;
mod widgets;

pub fn main() -> iced::Result {
    State::run(Settings::default())
}

#[derive(Default)]
struct State {
    theme: Theme,
    input_value: String,
    slider_value: f32,
    checkbox_value: bool,
    toggler_value: bool,
    repo: Option<Backend>,
    selected_commit: Option<usize>,
    empty_message: Vec<String>,
}

#[derive(Debug, Clone)]
enum Message {
    OpenRepo(String),
    ThemeChanged(Theme),
    InputChanged(String),
    ButtonPressed,
    SliderChanged(f32),
    CheckboxToggled(bool),
    TogglerToggled(bool),
    CommitSelected(usize, String),
}

impl State {
    fn open_repo(&self) -> Message {
        Message::OpenRepo(String::from("/Users/rob/dev/journey"))
    }
}

impl Sandbox for State {
    type Message = Message;

    fn new() -> Self {
        let mut x = State::default();
        // TODO: Uuuuuuugly
        x.empty_message = vec!["No commits".to_string()];
        x.update(x.open_repo());
        x
    }

    fn title(&self) -> String {
        String::from("Journey: amend.repo")
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::OpenRepo(path) => self.repo = Backend::new(path).ok(),
            Message::ThemeChanged(theme) => {
                self.theme = theme;
            }
            Message::CommitSelected(idx, _) => self.selected_commit = Some(idx),
            Message::InputChanged(value) => self.input_value = value,
            Message::ButtonPressed => {}
            Message::SliderChanged(value) => self.slider_value = value,
            Message::CheckboxToggled(value) => self.checkbox_value = value,
            Message::TogglerToggled(value) => self.toggler_value = value,
        }
    }

    fn view(&self) -> Element<Message> {
        let file_selector = widgets::scrollbox(column![
            "[All files]",
            "amend.π.r",
            "amend.h",
            "browser.c",
            "browser.h",
            vertical_space().height(200),
        ])
        .width(160)
        .height(100);

        let commit_selector = SelectionList::new_with(
            if let Some(b) = &self.repo {
                &b.messages
            } else {
                &self.empty_message
            },
            Message::CommitSelected,
            12.0,
            5.0,
            SelectionListStyles::Default,
            self.selected_commit,
            Font::default(),
        )
        .width(iced::Length::Fixed(550.))
        .height(iced::Length::Fixed(144.));

        let top = row![
            column![
                file_selector,
                widgets::button("Generate Diff")
                    .width(160)
                    .height(30)
                    .on_press(Message::ButtonPressed)
            ]
            .spacing(14),
            commit_selector
        ]
        .spacing(14);

        let content = column![
            top,
            widgets::scrollbox(column![
                "Scroll me!",
                vertical_space().height(800),
                "You did it!"
            ])
            .width(Length::Fill)
            .height(255)
        ]
        .spacing(14)
        .padding(14);
        // .max_width(600);

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            // .center_x()
            // .center_y()
            .style(iced::theme::Container::Custom(Box::new(
                style::MyButtonStyle {},
            )))
            .into()
    }

    fn theme(&self) -> Theme {
        self.theme.clone()
    }
}
