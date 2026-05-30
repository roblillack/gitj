//! The application shell layout — a single flat container that arranges
//! journey's panes the way gitk does and owns one focus scope.
//!
//! Why a bespoke container instead of nesting retrogui `Column`/`Row`:
//! retrogui's focus model is flat *per container*, so a focusable `Row`
//! nested in a `Column` becomes a single Tab stop — its inner panes can't be
//! reached with the keyboard. By keeping every pane a direct child of one
//! `Panes`, Tab cycles across all of them, the menu bar's Alt-accelerators
//! reach it regardless of which pane has focus, and a modal dialog overlay
//! floats over the whole window. The event / focus / capture / accelerator /
//! overlay handling mirrors retrogui's `Column`; only `layout` is custom.

use retrogui::{Color, Event, EventCtx, Painter, PopupRequest, Rect, Theme, Widget};

/// Where a child sits in the gitk arrangement.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    /// Full-width menu bar pinned to the top.
    Menu,
    /// Full-width toolbar (search) below the menu.
    Toolbar,
    /// Full-width commit history, upper content area.
    History,
    /// Main diff view, lower-left content area.
    Diff,
    /// Changed-files list, lower-right content area.
    Files,
}

struct Child {
    widget: Box<dyn Widget>,
    pane: Pane,
}

pub struct Panes {
    bounds: Rect,
    background: Color,
    children: Vec<Child>,
    overlays: Vec<Box<dyn Widget>>,
    captured: Option<usize>,
    focused: Option<usize>,
    menu_h: i32,
    toolbar_h: i32,
    files_w: i32,
    /// Fraction (0..1) of the content area height given to the history pane.
    history_frac: f32,
}

impl Panes {
    pub fn new() -> Self {
        Self {
            bounds: Rect::new(0, 0, 0, 0),
            background: Color::LIGHT_GRAY,
            children: Vec::new(),
            overlays: Vec::new(),
            captured: None,
            focused: None,
            menu_h: 0,
            toolbar_h: 0,
            files_w: 300,
            history_frac: 0.46,
        }
    }

    pub fn menu_height(mut self, h: i32) -> Self {
        self.menu_h = h;
        self
    }

    pub fn toolbar_height(mut self, h: i32) -> Self {
        self.toolbar_h = h;
        self
    }

    pub fn files_width(mut self, w: i32) -> Self {
        self.files_w = w;
        self
    }

    pub fn history_fraction(mut self, frac: f32) -> Self {
        self.history_frac = frac.clamp(0.1, 0.9);
        self
    }

    /// Add a pane. Call order also sets keyboard focus order, so add the
    /// toolbar, history, diff and files in the order you want Tab to visit.
    pub fn add(mut self, pane: Pane, widget: impl Widget + 'static) -> Self {
        self.children.push(Child {
            widget: Box::new(widget),
            pane,
        });
        self
    }

    /// Add a floating overlay (e.g. a modal dialog) over the whole shell.
    pub fn add_overlay(mut self, widget: impl Widget + 'static) -> Self {
        self.overlays.push(Box::new(widget));
        self
    }

    pub fn focus_pane(&mut self, pane: Pane) -> bool {
        let Some(index) = self.children.iter().position(|c| c.pane == pane) else {
            return false;
        };
        if !self.children[index].widget.focusable() {
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
        self.children.iter().filter(|c| c.widget.focusable()).count()
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

impl Default for Panes {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Panes {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn layout(&mut self, bounds: Rect) {
        self.bounds = bounds;
        for child in &mut self.children {
            let rect = rect_for(
                child.pane,
                bounds,
                self.menu_h,
                self.toolbar_h,
                self.files_w,
                self.history_frac,
            );
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

            match retrogui_tab_action(event) {
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

/// Free-function form of `rect_for` usable from `&mut self` layout without
/// borrowing all of `self`.
fn rect_for(
    pane: Pane,
    b: Rect,
    menu_h: i32,
    toolbar_h: i32,
    files_w: i32,
    history_frac: f32,
) -> Rect {
    let content_y = b.y + menu_h + toolbar_h;
    let content_h = (b.h - menu_h - toolbar_h).max(0);
    let history_h = (content_h as f32 * history_frac).round() as i32;
    let lower_y = content_y + history_h;
    let lower_h = (content_h - history_h).max(0);
    let files_w = files_w.min((b.w - 80).max(0));
    let diff_w = (b.w - files_w).max(0);
    match pane {
        Pane::Menu => Rect::new(b.x, b.y, b.w, menu_h),
        Pane::Toolbar => Rect::new(b.x, b.y + menu_h, b.w, toolbar_h),
        Pane::History => Rect::new(b.x, content_y, b.w, history_h),
        Pane::Diff => Rect::new(b.x, lower_y, diff_w, lower_h),
        Pane::Files => Rect::new(b.x + diff_w, lower_y, files_w, lower_h),
    }
}

// Tab handling mirrors retrogui's internal `tab_action`, which isn't public.
enum TabKind {
    Cycle(i32),
    Swallow,
}

fn retrogui_tab_action(event: &Event) -> Option<TabKind> {
    use retrogui::{Key, NamedKey};
    match event {
        Event::KeyDown {
            key: Key::Named(NamedKey::Tab),
            modifiers,
        } if !modifiers.control && !modifiers.alt && !modifiers.logo => {
            Some(TabKind::Cycle(if modifiers.shift { -1 } else { 1 }))
        }
        Event::Char { ch: '\t', modifiers }
            if !modifiers.control && !modifiers.alt && !modifiers.logo =>
        {
            Some(TabKind::Swallow)
        }
        _ => None,
    }
}
