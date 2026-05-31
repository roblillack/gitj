//! A generic shared-ownership adapter.
//!
//! saudade's widget tree takes ownership of each child (`Box<dyn Widget>`),
//! but the app root needs to keep talking to individual widgets after they're
//! in the tree — e.g. to push new items into a list when the selection
//! elsewhere changes. The notepad/picker demos solve this with one bespoke
//! `SharedFoo` struct per widget type; [`Shared`] collapses that into a single
//! generic wrapper that forwards every [`Widget`] method to an
//! `Rc<RefCell<W>>`, so both the tree and the app root can hold a handle.

use std::cell::RefCell;
use std::rc::Rc;

use saudade::{Event, EventCtx, Painter, PopupRequest, Rect, Theme, Widget};

/// Shared, interior-mutable handle to a widget that lives in the tree.
pub struct Shared<W>(pub Rc<RefCell<W>>);

impl<W> Shared<W> {
    pub fn new(inner: Rc<RefCell<W>>) -> Self {
        Self(inner)
    }

    /// Clone the underlying handle so the app root can keep a reference.
    pub fn handle(&self) -> Rc<RefCell<W>> {
        self.0.clone()
    }
}

impl<W: Widget> Widget for Shared<W> {
    fn bounds(&self) -> Rect {
        self.0.borrow().bounds()
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        self.0.borrow_mut().paint(painter, theme);
    }

    fn paint_overlay(&mut self, painter: &mut Painter, theme: &Theme) {
        self.0.borrow_mut().paint_overlay(painter, theme);
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        self.0.borrow_mut().event(event, ctx);
    }

    fn captures_pointer(&self) -> bool {
        self.0.borrow().captures_pointer()
    }

    fn focusable(&self) -> bool {
        self.0.borrow().focusable()
    }

    fn set_focused(&mut self, focused: bool) {
        self.0.borrow_mut().set_focused(focused);
    }

    fn accepts_accelerators(&self) -> bool {
        self.0.borrow().accepts_accelerators()
    }

    fn layout(&mut self, bounds: Rect) {
        self.0.borrow_mut().layout(bounds);
    }

    fn focus_first(&mut self) -> bool {
        self.0.borrow_mut().focus_first()
    }

    fn popup_request(&self) -> Option<PopupRequest> {
        self.0.borrow().popup_request()
    }

    fn wants_ticks(&self) -> bool {
        self.0.borrow().wants_ticks()
    }
}
