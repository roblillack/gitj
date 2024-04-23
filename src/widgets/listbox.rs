use std::borrow::Cow;

use crate::style::{self, DEFAULT_FONT_SIZE, SCROLLBAR_WIDTH};

use iced::{
    advanced::{
        graphics,
        layout::{Limits, Node},
        renderer,
        text::{Paragraph, Text},
        widget::{tree, Tree},
        Clipboard, Layout, Shell, Widget,
    },
    alignment::{Horizontal, Vertical},
    event,
    mouse::{self, Cursor},
    widget::{
        container,
        scrollable::{Alignment, Direction, Properties},
        text::{self, LineHeight},
        Container, Scrollable,
    },
    Border, Color, Element, Event, Font, Length, Padding, Pixels, Rectangle, Shadow, Size, Theme,
};
use iced_aw::{native::List, SelectionListStyles};
use std::{fmt::Display, hash::Hash, marker::PhantomData};

use super::scrollbox;

/// A widget for selecting a single value from a dynamic scrollable list of options.
#[allow(missing_debug_implementations)]
#[allow(clippy::type_repetition_in_bounds)]
pub struct ListBox<'a, T, Message, Renderer = iced::Renderer>
where
    Message: 'a + Clone,
    T: Clone + ToString + Eq + Hash,
    [T]: ToOwned<Owned = Vec<T>>,
    Renderer: renderer::Renderer + iced::advanced::text::Renderer<Font = iced::Font>,
{
    /// Container for Rendering List.
    container: Container<'a, Message, Theme, Renderer>,
    /// List of Elements to Render.
    options: &'a [T],
    /// Label Font
    font: Renderer::Font,
    /// The Containers Width
    width: Length,
    /// The Containers height
    height: Length,
    /// The padding Width
    padding: f32,
    /// The Text Size
    text_size: f32,
}

pub fn listbox<'a, T, Message, Renderer>(
    options: &'a [T],
    on_selected: impl Fn(usize, T) -> Message + 'static,
    selected: Option<usize>,
) -> ListBox<'a, T, Message, Renderer>
where
    Message: 'a + Clone,
    Renderer: 'a + renderer::Renderer + iced::advanced::text::Renderer<Font = iced::Font>,
    T: Clone + Display + Eq + Hash,
    [T]: ToOwned<Owned = Vec<T>>,
{
    ListBox::new_with(
        options,
        on_selected,
        DEFAULT_FONT_SIZE,
        3.0,
        selected,
        Font::DEFAULT,
    )
}

#[allow(clippy::type_repetition_in_bounds)]
impl<'a, T, Message, Renderer> ListBox<'a, T, Message, Renderer>
where
    Message: 'a + Clone,
    Renderer: 'a + renderer::Renderer + iced::advanced::text::Renderer<Font = iced::Font>,
    T: Clone + Display + Eq + Hash,
    [T]: ToOwned<Owned = Vec<T>>,
{
    /// Creates a new [`SelectionList`] with the given list of `options`,
    /// the current selected value, the message to produce when an option is
    /// selected, the `style`, `text_size`, `padding` and `font`.
    pub fn new_with(
        options: &'a [T],
        on_selected: impl Fn(usize, T) -> Message + 'static,
        text_size: f32,
        padding: f32,
        selected: Option<usize>,
        font: Font,
    ) -> Self {
        let container = Container::new(
            Scrollable::new(
                Container::new(List {
                    options,
                    font,
                    text_size,
                    padding,
                    selected,
                    on_selected: Box::new(on_selected),
                    phantomdata: PhantomData,
                    style: SelectionListStyles::Default,
                })
                .padding(Padding {
                    top: 3.0,
                    right: SCROLLBAR_WIDTH as f32 + 5.,
                    bottom: 3.,
                    left: 5.,
                }),
            )
            .style(iced::theme::Scrollable::Custom(Box::new(
                style::MyButtonStyle {},
            )))
            .direction(Direction::Vertical(
                Properties::new()
                    .width(SCROLLBAR_WIDTH)
                    .margin(0)
                    .scroller_width(SCROLLBAR_WIDTH)
                    .alignment(Alignment::Start),
            )),
        );

        Self {
            options,
            font,
            container,
            width: Length::Fill,
            height: Length::Fill,
            padding,
            text_size,
        }
    }

    /// Sets the width of the [`SelectionList`].
    #[must_use]
    pub fn width(mut self, width: Length) -> Self {
        self.width = width;
        self
    }

    /// Sets the height of the [`SelectionList`].
    #[must_use]
    pub fn height(mut self, height: Length) -> Self {
        self.height = height;
        self
    }
}

impl<'a, T, Message, Renderer> Widget<Message, Theme, Renderer>
    for ListBox<'a, T, Message, Renderer>
where
    T: 'a + Clone + ToString + Eq + Hash + Display,
    Message: 'a + Clone,
    Renderer: renderer::Renderer + iced::advanced::text::Renderer<Font = iced::Font> + 'a,
{
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.container as &dyn Widget<_, _, _>)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&[&self.container as &dyn Widget<_, _, _>]);
        let state = tree.state.downcast_mut::<State>();

        state.values = self
            .options
            .iter()
            .map(|_| graphics::text::Paragraph::new())
            .collect();
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.width, Length::Shrink)
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::new(self.options))
    }

    fn layout(&self, tree: &mut Tree, renderer: &Renderer, limits: &Limits) -> Node {
        use std::f32;

        let state = tree.state.downcast_mut::<State>();

        let limits = limits.width(self.width).height(self.height);

        let max_width = match self.width {
            Length::Shrink => self
                .options
                .iter()
                .enumerate()
                .map(|(id, val)| {
                    let text = Text {
                        content: &val.to_string(),
                        size: Pixels(self.text_size),
                        line_height: LineHeight::default(),
                        bounds: Size::INFINITY,
                        font: self.font,
                        horizontal_alignment: Horizontal::Left,
                        vertical_alignment: Vertical::Top,
                        shaping: text::Shaping::Advanced,
                    };

                    state.values[id].update(text);
                    state.values[id].min_bounds().width.round() as u32 + self.padding as u32 * 2
                })
                .max()
                .unwrap_or(100),
            _ => limits.max().width as u32,
        };

        let limits = limits.max_width(max_width as f32 + self.padding * 2.0);

        let content = self
            .container
            .layout(&mut tree.children[0], renderer, &limits);
        let size = limits.resolve(self.width, self.height, content.size());
        Node::with_children(size, vec![content])
    }

    fn on_event(
        &mut self,
        state: &mut Tree,
        event: Event,
        layout: Layout<'_>,
        cursor: Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<Message>,
        viewport: &Rectangle,
    ) -> event::Status {
        self.container.on_event(
            &mut state.children[0],
            event,
            layout
                .children()
                .next()
                .expect("Scrollable Child Missing in Selection List"),
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        )
    }

    fn mouse_interaction(
        &self,
        state: &Tree,
        layout: Layout<'_>,
        cursor: Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.container
            .mouse_interaction(&state.children[0], layout, cursor, viewport, renderer)
    }

    fn draw(
        &self,
        state: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: Cursor,
        _viewport: &Rectangle,
    ) {
        renderer.fill_quad(
            renderer::Quad {
                bounds: layout.bounds(),
                border: Border {
                    radius: (0.0).into(),
                    width: 1.5,
                    color: Color::BLACK,
                },
                shadow: Shadow::default(),
            },
            Color::WHITE,
        );

        self.container.draw(
            &state.children[0],
            renderer,
            theme,
            style,
            layout
                .children()
                .next()
                .expect("Scrollable Child Missing in Selection List"),
            cursor,
            &layout.bounds(),
        );
    }
}

impl<'a, T, Message, Renderer> From<ListBox<'a, T, Message, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    T: Clone + ToString + Eq + Hash + Display,
    Message: 'a + Clone,
    Renderer: 'a + renderer::Renderer + iced::advanced::text::Renderer<Font = iced::Font>,
{
    fn from(selection_list: ListBox<'a, T, Message, Renderer>) -> Self {
        Element::new(selection_list)
    }
}

/// A Paragraph cache to enhance speed of layouting.
#[derive(Debug, Default, Clone)]
pub struct State {
    values: Vec<graphics::text::Paragraph>,
}

impl State {
    /// Creates a new [`State`], representing an unfocused [`TextInput`].
    pub fn new<T>(options: &[T]) -> Self
    where
        T: Clone + Display + Eq + Hash,
        [T]: ToOwned<Owned = Vec<T>>,
    {
        Self {
            values: options
                .iter()
                .map(|_| graphics::text::Paragraph::new())
                .collect(),
        }
    }
}
