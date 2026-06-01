//! A generic flat-focus container — the engine behind both journey screens.
//!
//! saudade's focus model is flat *per container*: a focusable widget nested
//! inside another container collapses to a single Tab stop. So journey keeps
//! every pane a direct child of one `Shell`, which gives correct Tab cycling
//! across all panes, lets the menu bar's Alt-accelerators reach it regardless
//! of which pane has focus, and floats a modal dialog overlay over the whole
//! window. The event / focus / capture / accelerator / overlay handling
//! mirrors saudade's `Column`; only the per-child placement differs, which is
//! why each child carries its own layout closure. The gitk browse screen and
//! the git-gui commit screen are then just two different sets of placements
//! (see [`crate::widgets::layout`]).

use saudade::{Color, Event, EventCtx, Painter, PopupRequest, Rect, Theme, Widget};

/// Computes a child's rectangle from the container's bounds.
type Place = Box<dyn Fn(Rect) -> Rect>;

struct Child {
    widget: Box<dyn Widget>,
    place: Place,
}

pub struct Shell {
    bounds: Rect,
    background: Color,
    children: Vec<Child>,
    overlays: Vec<Box<dyn Widget>>,
    captured: Option<usize>,
    focused: Option<usize>,
}

impl Shell {
    pub fn new() -> Self {
        Self {
            bounds: Rect::new(0, 0, 0, 0),
            background: Color::LIGHT_GRAY,
            children: Vec::new(),
            overlays: Vec::new(),
            captured: None,
            focused: None,
        }
    }

    /// Add a child positioned by `place`. Call order also sets the keyboard
    /// focus order — Tab visits focusable children in the order they're added.
    pub fn add(
        mut self,
        widget: impl Widget + 'static,
        place: impl Fn(Rect) -> Rect + 'static,
    ) -> Self {
        self.children.push(Child {
            widget: Box::new(widget),
            place: Box::new(place),
        });
        self
    }

    /// Add a floating overlay (e.g. a modal dialog) over the whole shell.
    pub fn add_overlay(mut self, widget: impl Widget + 'static) -> Self {
        self.overlays.push(Box::new(widget));
        self
    }

    /// Focus the child at `index` (a direct-child index, not a focusable-only
    /// index), if it is focusable. Returns whether focus moved there.
    pub fn focus_child(&mut self, index: usize) -> bool {
        let Some(child) = self.children.get(index) else {
            return false;
        };
        if !child.widget.focusable() {
            return false;
        }
        if let Some(old) = self.focused
            && old != index
            && let Some(c) = self.children.get_mut(old)
        {
            c.widget.set_focused(false);
        }
        let focused = self.children[index].widget.focus_first();
        if focused {
            self.focused = Some(index);
        }
        focused
    }

    fn active_overlay(&self) -> Option<usize> {
        self.overlays.iter().position(|o| o.captures_pointer())
    }

    fn choose_target(&self, event: &Event) -> Option<usize> {
        if event.is_keyboard() {
            return self.focused;
        }
        if let Some(idx) = self.captured {
            return Some(idx);
        }
        let pos = event.position()?;
        (0..self.children.len())
            .rev()
            .find(|&i| self.children[i].widget.bounds().contains(pos))
    }

    fn change_focus(&mut self, new_focus: Option<usize>, ctx: &mut EventCtx) {
        if new_focus == self.focused {
            return;
        }
        if let Some(old) = self.focused
            && let Some(c) = self.children.get_mut(old)
        {
            c.widget.set_focused(false);
        }
        if let Some(new) = new_focus
            && let Some(c) = self.children.get_mut(new)
        {
            c.widget.focus_first();
        }
        self.focused = new_focus;
        ctx.request_paint();
    }

    fn focusable_count(&self) -> usize {
        self.children
            .iter()
            .filter(|c| c.widget.focusable())
            .count()
    }

    fn cycle_focus(&mut self, dir: i32, ctx: &mut EventCtx) -> bool {
        let candidates: Vec<usize> = (0..self.children.len())
            .filter(|&i| self.children[i].widget.focusable())
            .collect();
        if candidates.is_empty() {
            return false;
        }
        let cur_pos = self
            .focused
            .and_then(|c| candidates.iter().position(|&i| i == c));
        let n = candidates.len() as i32;
        let next = match cur_pos {
            None => {
                if dir > 0 {
                    candidates[0]
                } else {
                    candidates[(n - 1) as usize]
                }
            }
            Some(p) => candidates[((p as i32 + dir).rem_euclid(n)) as usize],
        };
        if Some(next) == self.focused {
            return false;
        }
        self.change_focus(Some(next), ctx);
        true
    }
}

impl Default for Shell {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Shell {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn layout(&mut self, bounds: Rect) {
        self.bounds = bounds;
        for child in &mut self.children {
            let rect = (child.place)(bounds);
            child.widget.layout(rect);
        }
        for overlay in &mut self.overlays {
            overlay.layout(bounds);
        }
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        painter.fill_rect(self.bounds, self.background);
        for child in &mut self.children {
            child.widget.paint(painter, theme);
        }
        for child in &mut self.children {
            child.widget.paint_overlay(painter, theme);
        }
        for overlay in &mut self.overlays {
            overlay.paint(painter, theme);
            overlay.paint_overlay(painter, theme);
        }
    }

    fn paint_overlay(&mut self, painter: &mut Painter, theme: &Theme) {
        for child in &mut self.children {
            child.widget.paint_overlay(painter, theme);
        }
        for overlay in &mut self.overlays {
            overlay.paint_overlay(painter, theme);
        }
    }

    fn event(&mut self, event: &Event, ctx: &mut EventCtx) {
        if let Some(idx) = self.active_overlay() {
            self.overlays[idx].event(event, ctx);
            return;
        }

        if !event.is_keyboard() && event.position().is_none() && self.captured.is_none() {
            for child in &mut self.children {
                child.widget.event(event, ctx);
            }
            return;
        }

        if event.is_keyboard() {
            let mut accelerator_blocking = false;
            for (idx, child) in self.children.iter_mut().enumerate() {
                if child.widget.accepts_accelerators() && Some(idx) != self.focused {
                    child.widget.event(event, ctx);
                    if ctx.is_consumed() {
                        return;
                    }
                    if child.widget.captures_pointer() {
                        accelerator_blocking = true;
                    }
                }
            }
            if accelerator_blocking {
                return;
            }

            match tab_action(event) {
                Some(TabKind::Cycle(dir)) => {
                    if self.cycle_focus(dir, ctx) {
                        return;
                    }
                }
                Some(TabKind::Swallow) if self.focusable_count() >= 2 => return,
                _ => {}
            }
        }

        let Some(idx) = self.choose_target(event) else {
            return;
        };

        let captured_was_set = self.captured == Some(idx);
        {
            let child = &mut self.children[idx];
            child.widget.event(event, ctx);
            if !event.is_keyboard() {
                if child.widget.captures_pointer() {
                    self.captured = Some(idx);
                } else if captured_was_set {
                    self.captured = None;
                }
            }
        }

        if ctx.is_focus_requested() {
            ctx.clear_focus_flags();
            self.change_focus(Some(idx), ctx);
        } else if ctx.is_focus_released() {
            ctx.clear_focus_flags();
            if self.focused == Some(idx) {
                self.change_focus(None, ctx);
            }
        }
    }

    fn captures_pointer(&self) -> bool {
        self.captured.is_some() || self.active_overlay().is_some()
    }

    fn focusable(&self) -> bool {
        self.children.iter().any(|c| c.widget.focusable())
    }

    fn focus_first(&mut self) -> bool {
        for (idx, child) in self.children.iter_mut().enumerate() {
            if child.widget.focus_first() {
                self.focused = Some(idx);
                return true;
            }
        }
        false
    }

    fn popup_request(&self) -> Option<PopupRequest> {
        for overlay in &self.overlays {
            if let Some(req) = overlay.popup_request() {
                return Some(req);
            }
        }
        for child in &self.children {
            if let Some(req) = child.widget.popup_request() {
                return Some(req);
            }
        }
        None
    }

    fn wants_ticks(&self) -> bool {
        self.children.iter().any(|c| c.widget.wants_ticks())
            || self.overlays.iter().any(|o| o.wants_ticks())
    }
}

// Tab handling mirrors saudade's internal `tab_action`, which isn't public.
enum TabKind {
    Cycle(i32),
    Swallow,
}

fn tab_action(event: &Event) -> Option<TabKind> {
    use saudade::{Key, NamedKey};
    match event {
        Event::KeyDown {
            key: Key::Named(NamedKey::Tab),
            modifiers,
        } if !modifiers.control && !modifiers.alt && !modifiers.logo => {
            Some(TabKind::Cycle(if modifiers.shift { -1 } else { 1 }))
        }
        Event::Char {
            ch: '\t',
            modifiers,
        } if !modifiers.control && !modifiers.alt && !modifiers.logo => Some(TabKind::Swallow),
        _ => None,
    }
}
