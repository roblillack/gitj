use iced::theme::Button::Custom;
use iced::widget::{
    button, checkbox, column, container, horizontal_rule, pick_list, progress_bar, row, scrollable,
    slider, text, text_input, toggler, vertical_rule, vertical_space, Button, Scrollable,
};
use iced::{font, Alignment, Element, Font, Length, Sandbox, Settings, Theme};

mod style;
mod widgets;

pub fn main() -> iced::Result {
    Styling::run(Settings::default())
}

#[derive(Default)]
struct Styling {
    theme: Theme,
    input_value: String,
    slider_value: f32,
    checkbox_value: bool,
    toggler_value: bool,
}

#[derive(Debug, Clone)]
enum Message {
    ThemeChanged(Theme),
    InputChanged(String),
    ButtonPressed,
    SliderChanged(f32),
    CheckboxToggled(bool),
    TogglerToggled(bool),
}

impl Sandbox for Styling {
    type Message = Message;

    fn new() -> Self {
        Styling::default()
    }

    fn title(&self) -> String {
        String::from("Journey: amend.repo")
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::ThemeChanged(theme) => {
                self.theme = theme;
            }
            Message::InputChanged(value) => self.input_value = value,
            Message::ButtonPressed => {}
            Message::SliderChanged(value) => self.slider_value = value,
            Message::CheckboxToggled(value) => self.checkbox_value = value,
            Message::TogglerToggled(value) => self.toggler_value = value,
        }
    }

    fn view(&self) -> Element<Message> {
        let file_selector = widgets::listbox(column![
            "[All files]",
            "amend.π.r",
            "amend.h",
            "browser.c",
            "browser.h",
            vertical_space().height(200),
        ])
        .width(160)
        .height(100);

        let commit_selector = widgets::listbox(column![
            text("repo: Don't use D_IGNOREBLANKS for diffreg"),
            text("browser+committer: Improve Edit menu ops, add Cut+Paste in i"),
            text("browser: Only enable repo menu actions when committer is note"),
            text("committer: Move diff_line and diff_chunk into committer struc"),
            text("xxxxxx x x x xxx xxxxxxx xx xx x xxxxxxx"),
            vertical_space().height(800),
        ])
        .width(550)
        .height(144);

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
            widgets::listbox(column![
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
