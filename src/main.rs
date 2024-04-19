use iced::theme::Button::Custom;
use iced::widget::{
    button, checkbox, column, container, horizontal_rule, pick_list, progress_bar, row, scrollable,
    slider, text, text_input, toggler, vertical_rule, vertical_space, Button, Scrollable,
};
use iced::{font, Alignment, Element, Length, Sandbox, Settings, Theme};

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
            text("repo: Don't use D_IGHOREBLANKS for diffreg"),
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

        let choose_theme = column![
            text("Theme:"),
            pick_list(Theme::ALL, Some(&self.theme), Message::ThemeChanged).width(Length::Fill),
        ]
        .spacing(10);

        let text_input = text_input("Type something...", &self.input_value)
            .on_input(Message::InputChanged)
            .padding(10)
            .size(10);

        let button = Button::new("Submit")
            .padding(10)
            .on_press(Message::ButtonPressed);

        let slider = slider(0.0..=100.0, self.slider_value, Message::SliderChanged);

        let progress_bar = progress_bar(0.0..=100.0, self.slider_value);

        let scrollable123 = scrollable(column![
            "Scroll me!",
            vertical_space().height(800),
            "You did it!"
        ])
        .width(Length::Fill)
        .height(95);

        let checkbox =
            checkbox("Check me!", self.checkbox_value).on_toggle(Message::CheckboxToggled);

        let toggler = toggler(
            String::from("Toggle me!"),
            self.toggler_value,
            Message::TogglerToggled,
        )
        .width(Length::Shrink)
        .spacing(10);

        let content = column![
            top,
            choose_theme,
            horizontal_rule(38),
            row![text_input, button]
                .spacing(10)
                .align_items(Alignment::Center),
            slider,
            progress_bar,
            row![
                scrollable123,
                vertical_rule(38),
                column![checkbox, toggler].spacing(20)
            ]
            .spacing(14)
            .height(100)
            .align_items(Alignment::Center),
        ]
        .spacing(20)
        .padding(14)
        .max_width(600);

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x()
            .center_y()
            .style(iced::theme::Container::Custom(Box::new(
                style::MyButtonStyle {},
            )))
            .into()
    }

    fn theme(&self) -> Theme {
        self.theme.clone()
    }
}
