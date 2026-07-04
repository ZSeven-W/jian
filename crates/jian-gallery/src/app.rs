use jian_core::scroll::ScrollState;
use jian_core::text_input::TextInputState;
use jian_widgets::components::button::{Button, ButtonVariant};
use jian_widgets::components::dialog::{Dialog, DialogHit};
use jian_widgets::components::menu::{Menu, MenuHit, MenuItem, MenuState};
use jian_widgets::components::scroll_area::ScrollArea;
use jian_widgets::components::select::{Select, SelectHit, SelectItem, SelectState};
use jian_widgets::components::switch::Switch;
use jian_widgets::components::text_area::TextArea;
use jian_widgets::components::text_input::TextInputView;
use jian_widgets::{Color, Density, Painter, Point2D, Rect, TextLayout, Tokens};

const FONT_FAMILY: &str = "Inter";
const PAD: f32 = 24.0;
const GAP: f32 = 12.0;
const BUTTON_W: f32 = 156.0;
const BUTTON_H: f32 = 44.0;
const FIELD_W: f32 = 344.0;
const SCROLL_CONTENT_H: f32 = 44.0 * 16.0;

const MENU_ITEMS: &[MenuItem<'static>] = &[
    MenuItem {
        label: "Menu item: Duplicate",
        icon_d: None,
        danger: false,
        disabled: false,
        separator_above: false,
    },
    MenuItem {
        label: "Menu item: Rename",
        icon_d: None,
        danger: false,
        disabled: false,
        separator_above: false,
    },
    MenuItem {
        label: "Menu item: Delete",
        icon_d: None,
        danger: true,
        disabled: false,
        separator_above: true,
    },
];

#[derive(Debug, Clone)]
pub struct GalleryLayout {
    pub viewport: Rect,
    pub buttons: Vec<Rect>,
    pub text_input: Rect,
    pub text_area: Rect,
    pub select_anchor: Rect,
    pub menu_trigger: Rect,
    pub scroll_view: Rect,
    pub scroll_track: Rect,
    pub switches: Vec<Rect>,
    pub dialog_button: Rect,
}

impl GalleryLayout {
    pub fn switch_hits(&self, tokens: &Tokens) -> Vec<Rect> {
        self.switches
            .iter()
            .map(|rect| Switch::hit_rect(*rect, tokens))
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GalleryHit {
    None,
    Button(usize),
    TextInput,
    TextArea,
    SelectAnchor,
    SelectRow(usize),
    MenuTrigger,
    MenuRow(usize),
    ScrollView,
    ScrollThumb,
    Switch(usize),
    DialogButton,
    DialogClose,
    DialogScrim,
}

pub struct GalleryApp {
    tokens: Tokens,
    input: TextInputState,
    text_area: TextInputState,
    select_state: SelectState,
    selected_index: usize,
    select_labels: Vec<String>,
    menu_open: bool,
    menu_anchor: Point2D,
    menu_state: MenuState,
    scroll: ScrollState,
    switch_on: [bool; 2],
    dialog_open: bool,
    hover: GalleryHit,
    pressed: GalleryHit,
    focused: GalleryHit,
}

impl GalleryApp {
    pub fn new() -> Self {
        let mut tokens = Tokens::dark();
        tokens.density = Density::Touch;
        let select_labels = (1..=20).map(|i| format!("Select option {i:02}")).collect();
        Self {
            tokens,
            input: TextInputState::with_text("设计输入 CJK text"),
            text_area: TextInputState::with_text("多行文本区域\nTouch density text area"),
            select_state: SelectState::default(),
            selected_index: 0,
            select_labels,
            menu_open: false,
            menu_anchor: Point2D::ZERO,
            menu_state: MenuState::default(),
            scroll: ScrollState::default(),
            switch_on: [true, false],
            dialog_open: false,
            hover: GalleryHit::None,
            pressed: GalleryHit::None,
            focused: GalleryHit::None,
        }
    }

    pub fn tokens(&self) -> Tokens {
        self.tokens
    }

    pub fn layout(&self, viewport: Rect) -> GalleryLayout {
        let x = viewport.origin.x + PAD;
        let mut y = viewport.origin.y + PAD + 38.0;

        let mut buttons = Vec::with_capacity(8);
        for row in 0..4 {
            for col in 0..2 {
                buttons.push(Rect::xywh(
                    x + col as f32 * (BUTTON_W + GAP),
                    y + row as f32 * (BUTTON_H + GAP),
                    BUTTON_W,
                    BUTTON_H,
                ));
            }
        }
        y += 4.0 * BUTTON_H + 3.0 * GAP + 34.0;

        let text_input = Rect::xywh(x, y, FIELD_W, BUTTON_H);
        y += BUTTON_H + GAP;
        let text_area = Rect::xywh(x, y, FIELD_W, 104.0);
        y += 104.0 + GAP;
        let select_anchor = Rect::xywh(x, y, FIELD_W, BUTTON_H);
        y += BUTTON_H + GAP;
        let menu_trigger = Rect::xywh(x, y, FIELD_W, BUTTON_H);

        let right_x = x + FIELD_W + 42.0;
        let scroll_view = Rect::xywh(right_x, viewport.origin.y + PAD + 38.0, 300.0, 184.0);
        let scroll_track = Rect::xywh(
            scroll_view.origin.x + scroll_view.size.x - 8.0,
            scroll_view.origin.y,
            8.0,
            scroll_view.size.y,
        );
        let switches = vec![
            Rect::xywh(
                right_x,
                scroll_view.origin.y + scroll_view.size.y + 42.0,
                40.0,
                22.0,
            ),
            Rect::xywh(
                right_x,
                scroll_view.origin.y + scroll_view.size.y + 42.0 + 56.0,
                40.0,
                22.0,
            ),
        ];
        let dialog_button = Rect::xywh(
            right_x,
            switches
                .last()
                .map_or(scroll_view.origin.y, |r| r.origin.y + 52.0),
            220.0,
            BUTTON_H,
        );

        GalleryLayout {
            viewport,
            buttons,
            text_input,
            text_area,
            select_anchor,
            menu_trigger,
            scroll_view,
            scroll_track,
            switches,
            dialog_button,
        }
    }

    pub fn paint(&mut self, p: &mut dyn Painter, viewport: Rect, now_ms: u64) {
        let layout = self.layout(viewport);
        p.fill_rect(viewport, self.tokens.background);
        draw_text(
            p,
            "jian-widgets touch gallery",
            Point2D::new(viewport.origin.x + PAD, viewport.origin.y + PAD),
            20.0,
            600,
            self.tokens.foreground,
        );

        let specs = button_specs();
        for (i, rect) in layout.buttons.iter().copied().enumerate() {
            let (label, variant, enabled) = specs[i];
            Button {
                label,
                icon_paths: None,
                variant,
                enabled,
                hovered: self.hover == GalleryHit::Button(i),
                pressed: self.pressed == GalleryHit::Button(i),
                font_size: 0.0,
            }
            .paint(p, rect, &self.tokens);
        }

        self.paint_field_chrome(p, layout.text_input, self.focused == GalleryHit::TextInput);
        TextInputView {
            state: &self.input,
            placeholder: "Text input",
            focused: self.focused == GalleryHit::TextInput,
            font_size: 0.0,
            now_ms,
            pad_x: 10.0,
            baseline_delta_y: 0.0,
            mask: None,
        }
        .paint(p, layout.text_input, &self.tokens);

        self.paint_field_chrome(p, layout.text_area, self.focused == GalleryHit::TextArea);
        TextArea {
            state: &self.text_area,
            placeholder: "Text area",
            focused: self.focused == GalleryHit::TextArea,
            font_size: 0.0,
            now_ms,
            pad_x: 10.0,
            max_visible_lines: 4,
        }
        .paint(p, layout.text_area, &self.tokens);

        let selected_label = self
            .select_labels
            .get(self.selected_index)
            .map_or("Select", String::as_str);
        Button {
            label: selected_label,
            icon_paths: None,
            variant: ButtonVariant::Outline,
            enabled: true,
            hovered: self.hover == GalleryHit::SelectAnchor,
            pressed: self.pressed == GalleryHit::SelectAnchor,
            font_size: 0.0,
        }
        .paint(p, layout.select_anchor, &self.tokens);

        Button {
            label: "Open menu",
            icon_paths: None,
            variant: ButtonVariant::Outline,
            enabled: true,
            hovered: self.hover == GalleryHit::MenuTrigger,
            pressed: self.pressed == GalleryHit::MenuTrigger,
            font_size: 0.0,
        }
        .paint(p, layout.menu_trigger, &self.tokens);

        self.paint_scroll_list(p, &layout);

        for (i, rect) in layout.switches.iter().copied().enumerate() {
            Switch {
                on: self.switch_on[i],
                enabled: true,
                hovered: self.hover == GalleryHit::Switch(i),
                pressed: self.pressed == GalleryHit::Switch(i),
            }
            .paint(p, rect, &self.tokens);
            draw_text(
                p,
                if i == 0 { "Switch on" } else { "Switch off" },
                Point2D::new(rect.origin.x + 58.0, rect.origin.y + 2.0),
                self.tokens.density.font_size(),
                400,
                self.tokens.foreground,
            );
        }

        Button {
            label: "Open dialog",
            icon_paths: None,
            variant: ButtonVariant::Primary,
            enabled: true,
            hovered: self.hover == GalleryHit::DialogButton,
            pressed: self.pressed == GalleryHit::DialogButton,
            font_size: 0.0,
        }
        .paint(p, layout.dialog_button, &self.tokens);

        if self.select_state.open {
            let items = self.select_items();
            Select {
                state: &self.select_state,
                items: &items,
            }
            .paint(p, layout.select_anchor, viewport, &self.tokens);
        }

        if self.menu_open {
            Menu {
                state: &self.menu_state,
                items: MENU_ITEMS,
            }
            .paint(p, self.menu_anchor, viewport, &self.tokens);
        }

        if self.dialog_open {
            Dialog {
                title: "Touch Gallery Dialog",
                width: 360.0,
                height: 180.0,
            }
            .paint(p, viewport, &self.tokens);
        }
    }

    pub fn open_dialog(&mut self) {
        self.dialog_open = true;
    }

    pub fn open_menu(&mut self, anchor: Point2D) {
        self.menu_open = true;
        self.menu_anchor = anchor;
        self.menu_state.hover = None;
    }

    pub fn open_select(&mut self) {
        self.select_state.open = true;
        self.select_state.hover = Some(self.selected_index);
    }

    pub fn set_hover(&mut self, point: Point2D, viewport: Rect) -> GalleryHit {
        let hit = self.hit_test(point, viewport);
        self.hover = hit;
        self.update_popup_hover(hit);
        hit
    }

    pub fn press(&mut self, point: Point2D, viewport: Rect, now_ms: u64) -> GalleryHit {
        let hit = self.set_hover(point, viewport);
        self.pressed = hit;
        match hit {
            GalleryHit::TextInput => {
                self.focused = GalleryHit::TextInput;
                self.input.touch(now_ms);
            }
            GalleryHit::TextArea => {
                self.focused = GalleryHit::TextArea;
                self.text_area.touch(now_ms);
            }
            _ => {}
        }
        hit
    }

    pub fn release(&mut self, point: Point2D, viewport: Rect) -> GalleryHit {
        let hit = self.set_hover(point, viewport);
        let pressed = self.pressed;
        self.pressed = GalleryHit::None;
        if hit != pressed {
            return hit;
        }
        match hit {
            GalleryHit::SelectAnchor => self.toggle_select(),
            GalleryHit::SelectRow(row) if row < self.select_labels.len() => {
                self.selected_index = row;
                self.select_state.open = false;
                self.select_state.pressed = None;
            }
            GalleryHit::MenuTrigger => self.open_menu(Point2D::new(
                point.x,
                point.y + self.tokens.density.row_height() * 0.5,
            )),
            GalleryHit::MenuRow(_) => {
                self.menu_open = false;
                self.menu_state.hover = None;
            }
            GalleryHit::Switch(i) => {
                if let Some(v) = self.switch_on.get_mut(i) {
                    *v = !*v;
                }
            }
            GalleryHit::DialogButton => self.open_dialog(),
            GalleryHit::DialogClose | GalleryHit::DialogScrim => self.dialog_open = false,
            _ => {}
        }
        hit
    }

    pub fn cancel_press(&mut self) {
        self.pressed = GalleryHit::None;
        self.hover = GalleryHit::None;
        self.menu_state.hover = None;
        self.select_state.hover = None;
    }

    pub fn scroll_at(&mut self, point: Point2D, delta_y: f32, viewport: Rect) {
        let layout = self.layout(viewport);
        if layout.scroll_view.contains(point) {
            self.scroll
                .scroll_by(delta_y, SCROLL_CONTENT_H, layout.scroll_view.size.y);
        }
    }

    pub fn type_text(&mut self, text: &str, now_ms: u64) {
        match self.focused {
            GalleryHit::TextInput => self.input.insert_str(text, now_ms),
            GalleryHit::TextArea => self.text_area.insert_str(text, now_ms),
            _ => {}
        }
    }

    pub fn backspace(&mut self, now_ms: u64) {
        match self.focused {
            GalleryHit::TextInput => self.input.backspace(now_ms),
            GalleryHit::TextArea => self.text_area.backspace(now_ms),
            _ => {}
        }
    }

    pub fn next_blink_flip_ms(&self, now_ms: u64) -> Option<u64> {
        match self.focused {
            GalleryHit::TextInput => Some(self.input.next_blink_flip_ms(now_ms)),
            GalleryHit::TextArea => Some(self.text_area.next_blink_flip_ms(now_ms)),
            _ => None,
        }
    }

    fn toggle_select(&mut self) {
        if self.select_state.open {
            self.select_state.open = false;
        } else {
            self.open_select();
        }
    }

    fn hit_test(&self, point: Point2D, viewport: Rect) -> GalleryHit {
        let layout = self.layout(viewport);
        if self.dialog_open {
            let dialog = Dialog {
                title: "Touch Gallery Dialog",
                width: 360.0,
                height: 180.0,
            };
            return match dialog.hit(viewport, point) {
                DialogHit::Close => GalleryHit::DialogClose,
                DialogHit::Inside => GalleryHit::None,
                DialogHit::Scrim => GalleryHit::DialogScrim,
            };
        }
        if self.menu_open {
            match Menu::hit(
                self.menu_anchor,
                viewport,
                MENU_ITEMS.len(),
                point,
                &self.tokens,
            ) {
                MenuHit::Row(row) => return GalleryHit::MenuRow(row),
                MenuHit::Inside => return GalleryHit::None,
                MenuHit::Outside => {}
            }
        }
        if self.select_state.open {
            match Select::hit(
                &self.select_state,
                layout.select_anchor,
                viewport,
                self.select_labels.len(),
                point,
                &self.tokens,
            ) {
                SelectHit::Row(row) => return GalleryHit::SelectRow(row),
                SelectHit::Inside => return GalleryHit::None,
                SelectHit::Outside => {}
            }
        }
        for (i, rect) in layout.buttons.iter().copied().enumerate() {
            if Button::hit(rect, point) {
                return GalleryHit::Button(i);
            }
        }
        if layout.text_input.contains(point) {
            return GalleryHit::TextInput;
        }
        if layout.text_area.contains(point) {
            return GalleryHit::TextArea;
        }
        if layout.select_anchor.contains(point) {
            return GalleryHit::SelectAnchor;
        }
        if layout.menu_trigger.contains(point) {
            return GalleryHit::MenuTrigger;
        }
        if layout.scroll_track.contains(point) {
            return GalleryHit::ScrollThumb;
        }
        if layout.scroll_view.contains(point) {
            return GalleryHit::ScrollView;
        }
        for (i, rect) in layout.switch_hits(&self.tokens).iter().copied().enumerate() {
            if rect.contains(point) {
                return GalleryHit::Switch(i);
            }
        }
        if layout.dialog_button.contains(point) {
            return GalleryHit::DialogButton;
        }
        GalleryHit::None
    }

    fn update_popup_hover(&mut self, hit: GalleryHit) {
        self.menu_state.hover = match hit {
            GalleryHit::MenuRow(row) => Some(row),
            _ if !self.menu_open => None,
            _ => self.menu_state.hover,
        };
        self.select_state.hover = match hit {
            GalleryHit::SelectRow(row) => Some(row),
            _ if !self.select_state.open => None,
            _ => self.select_state.hover,
        };
    }

    fn paint_field_chrome(&self, p: &mut dyn Painter, rect: Rect, focused: bool) {
        p.fill_round_rect(rect, 6.0, self.tokens.card);
        p.stroke_round_rect(
            rect,
            6.0,
            if focused {
                self.tokens.primary
            } else {
                self.tokens.border
            },
            1.0,
        );
    }

    fn paint_scroll_list(&mut self, p: &mut dyn Painter, layout: &GalleryLayout) {
        p.fill_round_rect(layout.scroll_view, 6.0, self.tokens.card);
        p.stroke_round_rect(layout.scroll_view, 6.0, self.tokens.border, 1.0);
        p.save();
        p.clip_round_rect(layout.scroll_view, 6.0);
        for i in 0..16 {
            let y = layout.scroll_view.origin.y + i as f32 * 44.0 - self.scroll.offset;
            let row = Rect::xywh(
                layout.scroll_view.origin.x,
                y,
                layout.scroll_view.size.x,
                44.0,
            );
            if row.origin.y + row.size.y < layout.scroll_view.origin.y
                || row.origin.y > layout.scroll_view.origin.y + layout.scroll_view.size.y
            {
                continue;
            }
            if i % 2 == 0 {
                p.fill_rect(row, self.tokens.muted.with_alpha(0.35));
            }
            draw_text(
                p,
                &format!("Scroll row {:02}", i + 1),
                Point2D::new(row.origin.x + 12.0, row.origin.y + 12.0),
                self.tokens.density.font_size(),
                400,
                self.tokens.foreground,
            );
        }
        p.restore();

        ScrollArea {
            state: &mut self.scroll,
            content_h: SCROLL_CONTENT_H,
            view_h: layout.scroll_view.size.y,
            hovered: self.hover == GalleryHit::ScrollView || self.hover == GalleryHit::ScrollThumb,
            drag: None,
        }
        .paint_scrollbar(p, layout.scroll_track, &self.tokens);
    }

    fn select_items(&self) -> Vec<SelectItem<'_>> {
        self.select_labels
            .iter()
            .enumerate()
            .map(|(i, label)| SelectItem {
                label,
                selected: i == self.selected_index,
                disabled: false,
            })
            .collect()
    }
}

impl Default for GalleryApp {
    fn default() -> Self {
        Self::new()
    }
}

fn button_specs() -> [(&'static str, ButtonVariant, bool); 8] {
    [
        ("Ghost", ButtonVariant::Ghost, true),
        ("Ghost disabled", ButtonVariant::Ghost, false),
        ("Primary", ButtonVariant::Primary, true),
        ("Primary disabled", ButtonVariant::Primary, false),
        ("Outline", ButtonVariant::Outline, true),
        ("Outline disabled", ButtonVariant::Outline, false),
        ("Destructive", ButtonVariant::Destructive, true),
        ("Destructive disabled", ButtonVariant::Destructive, false),
    ]
}

fn draw_text(
    p: &mut dyn Painter,
    content: &str,
    origin: Point2D,
    font_size: f32,
    weight: u16,
    color: Color,
) {
    let layout = TextLayout::single_run(
        content,
        FONT_FAMILY,
        font_size,
        color.to_jian(),
        Point2D::ZERO,
    )
    .with_font_weight(weight);
    p.draw_text(&layout, origin);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_press_clears_pressed_and_hover_feedback() {
        let mut app = GalleryApp::new();
        let viewport = Rect::xywh(0.0, 0.0, 920.0, 720.0);
        let button = app.layout(viewport).buttons[0];
        let point = Point2D::new(
            button.origin.x + button.size.x * 0.5,
            button.origin.y + button.size.y * 0.5,
        );

        app.press(point, viewport, 0);
        assert_eq!(app.pressed, GalleryHit::Button(0));
        assert_eq!(app.hover, GalleryHit::Button(0));

        app.cancel_press();

        assert_eq!(app.pressed, GalleryHit::None);
        assert_eq!(app.hover, GalleryHit::None);
    }
}
