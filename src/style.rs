use iced::{
    border::Radius,
    widget::{button, container, scrollable},
    Border, Color, Font, Shadow, Theme, Vector,
};

#[derive(Default, Debug, Clone)]
pub struct MyButtonStyle {}

pub const DEFAULT_FONT_SIZE: f32 = 16.0;

impl button::StyleSheet for MyButtonStyle {
    type Style = Theme;

    fn active(&self, style: &Self::Style) -> button::Appearance {
        button::Appearance {
            shadow_offset: Vector::default(),
            background: Some(iced::Background::Color(Color::WHITE)),
            text_color: Color::BLACK,
            border: Border {
                color: Color::BLACK,
                width: 1.5,
                radius: 6.into(),
            },
            shadow: Shadow::default(),
        }
    }

    fn pressed(&self, style: &Self::Style) -> button::Appearance {
        button::Appearance {
            shadow_offset: Vector::default(),
            background: Some(iced::Background::Color(Color::BLACK)),
            text_color: Color::WHITE,
            border: Border {
                color: Color::BLACK,
                width: 1.5,
                radius: 6.into(),
            },
            shadow: Shadow::default(),
        }
    }
}

impl scrollable::StyleSheet for MyButtonStyle {
    type Style = Theme;

    fn active(&self, style: &Self::Style) -> scrollable::Appearance {
        scrollable::Appearance {
            container: container::Appearance {
                background: Some(iced::Background::Color(Color::WHITE)),
                border: Border {
                    color: Color::BLACK,
                    width: 1.0,
                    radius: Radius::from(0.0),
                },
                shadow: Shadow::default(),
                text_color: Some(Color::BLACK),
            },
            scrollbar: scrollable::Scrollbar {
                background: Some(iced::Background::Color(Color::from_rgb8(220, 220, 220))),
                border: Border {
                    color: Color::BLACK,
                    width: 1.0,
                    radius: Radius::from(0.0),
                },
                scroller: scrollable::Scroller {
                    color: Color::WHITE,
                    border: Border {
                        color: Color::BLACK,
                        width: 1.0,
                        radius: Radius::from(0.0),
                    },
                },
            },
            gap: None,
        }
    }

    fn hovered(
        &self,
        style: &Self::Style,
        is_mouse_over_scrollbar: bool,
    ) -> scrollable::Appearance {
        scrollable::Appearance {
            container: container::Appearance {
                background: Some(iced::Background::Color(Color::WHITE)),
                border: Border {
                    color: Color::BLACK,
                    width: 1.0,
                    radius: Radius::from(0.0),
                },
                shadow: Shadow::default(),
                text_color: Some(Color::BLACK),
            },
            scrollbar: scrollable::Scrollbar {
                background: Some(iced::Background::Color(Color::from_rgb8(220, 220, 220))),
                border: Border {
                    color: Color::BLACK,
                    width: 1.0,
                    radius: Radius::from(0.0),
                },
                scroller: scrollable::Scroller {
                    color: Color::WHITE,
                    border: Border {
                        color: Color::BLACK,
                        width: 1.0,
                        radius: Radius::from(0.0),
                    },
                },
            },
            gap: None,
        }
    }
}

impl container::StyleSheet for MyButtonStyle {
    type Style = Theme;

    fn appearance(&self, style: &Self::Style) -> container::Appearance {
        container::Appearance {
            background: Some(iced::Background::Color(Color::from_rgb8(240, 240, 240))),
            border: Border::default(),
            shadow: Shadow::default(),
            text_color: Some(Color::BLACK),
        }
    }
}

pub fn bold_font() -> Font {
    return Font {
        family: iced::font::Family::SansSerif,
        weight: iced::font::Weight::Bold,
        stretch: iced::font::Stretch::Normal,
        style: iced::font::Style::Normal,
    };
}
