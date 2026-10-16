//! Periphery's look: a dark palette with one accent colour, roomier spacing
//! and a few shared widgets (cards, chips, segmented tabs).

use eframe::egui::{self, Color32, FontId, Margin, Response, RichText, Rounding, Stroke, TextStyle, Theme, Ui, Visuals};

pub const ACCENT: Color32 = Color32::from_rgb(98, 112, 245);
const ACCENT_TEXT: Color32 = Color32::from_rgb(240, 242, 255);
pub const WARN: Color32 = Color32::from_rgb(230, 170, 60);
pub const GOOD: Color32 = Color32::from_rgb(80, 190, 120);

const RADIUS: f32 = 8.0;

fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// Installs the theme for both dark and light mode.
pub fn apply(ctx: &egui::Context) {
    ctx.set_visuals_of(Theme::Dark, dark());
    ctx.set_visuals_of(Theme::Light, light());
    for theme in [Theme::Dark, Theme::Light] {
        ctx.style_mut_of(theme, |s| {
            s.text_styles = [
                (TextStyle::Heading, FontId::proportional(22.0)),
                (TextStyle::Body, FontId::proportional(14.0)),
                (TextStyle::Button, FontId::proportional(14.0)),
                (TextStyle::Small, FontId::proportional(11.5)),
                (TextStyle::Monospace, FontId::monospace(13.0)),
            ]
            .into();
            s.spacing.item_spacing = egui::vec2(8.0, 8.0);
            s.spacing.button_padding = egui::vec2(12.0, 5.0);
            s.spacing.interact_size.y = 28.0;
            s.spacing.slider_width = 220.0;
            s.spacing.combo_width = 200.0;
            s.spacing.menu_margin = Margin::same(8.0);
            s.spacing.window_margin = Margin::same(16.0);
        });
    }
}

fn shape(v: &mut Visuals) {
    let r = Rounding::same(RADIUS);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.rounding = r;
    }
    v.window_rounding = Rounding::same(12.0);
    v.menu_rounding = r;
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT_TEXT);
    v.hyperlink_color = ACCENT;
    v.slider_trailing_fill = true;
    v.window_shadow.blur = 24.0;
    v.popup_shadow.blur = 12.0;
}

fn dark() -> Visuals {
    let mut v = Visuals::dark();
    v.panel_fill = rgb(0x16181d);
    v.window_fill = rgb(0x1d2027);
    v.faint_bg_color = rgb(0x1d2027);
    v.extreme_bg_color = rgb(0x101216);
    v.code_bg_color = rgb(0x101216);
    v.override_text_color = None;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = rgb(0x1d2027);
    w.noninteractive.weak_bg_fill = rgb(0x1d2027);
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, rgb(0x2a2e37));
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, rgb(0xc9ccd6));
    w.inactive.bg_fill = rgb(0x2a2e38);
    w.inactive.weak_bg_fill = rgb(0x272b34);
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0_f32, rgb(0xe3e5ec));
    w.hovered.bg_fill = rgb(0x343946);
    w.hovered.weak_bg_fill = rgb(0x323743);
    w.hovered.bg_stroke = Stroke::new(1.0_f32, rgb(0x454b5a));
    w.hovered.fg_stroke = Stroke::new(1.5_f32, rgb(0xf2f3f7));
    w.active.bg_fill = rgb(0x3c4252);
    w.active.weak_bg_fill = rgb(0x3c4252);
    w.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    w.active.fg_stroke = Stroke::new(2.0_f32, rgb(0xffffff));
    w.open = w.hovered;
    shape(&mut v);
    v
}

fn light() -> Visuals {
    let mut v = Visuals::light();
    v.panel_fill = rgb(0xf4f5f8);
    v.window_fill = rgb(0xffffff);
    v.faint_bg_color = rgb(0xffffff);
    v.extreme_bg_color = rgb(0xeceef3);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, rgb(0xdfe2e9));
    v.widgets.inactive.weak_bg_fill = rgb(0xe8eaf0);
    v.widgets.inactive.bg_fill = rgb(0xe3e6ed);
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.hovered.weak_bg_fill = rgb(0xdde1ea);
    shape(&mut v);
    v
}

/// The app mark: an accent tile with a P.
pub fn logo(ui: &mut Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    ui.painter().rect_filled(rect, size * 0.28, ACCENT);
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "P", FontId::proportional(size * 0.68), ACCENT_TEXT);
}

/// The panel behind a group of settings.
pub fn card(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    let v = ui.visuals();
    egui::Frame::none()
        .fill(v.faint_bg_color)
        .stroke(v.widgets.noninteractive.bg_stroke)
        .rounding(12.0)
        .inner_margin(Margin::same(16.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() {
                ui.label(RichText::new(title).strong().size(15.5));
                ui.add_space(8.0);
            }
            add(ui);
        });
    ui.add_space(12.0);
}

/// A small rounded label, e.g. "Battery 95%" in the header.
pub fn chip(ui: &mut Ui, key: &str, value: &str) -> Response {
    let fill = ui.visuals().widgets.inactive.weak_bg_fill;
    egui::Frame::none()
        .fill(fill)
        .rounding(12.0)
        .inner_margin(Margin::symmetric(9.0, 3.0))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            ui.label(RichText::new(key).small().weak());
            ui.label(RichText::new(value).small().strong());
        })
        .response
}

/// A row of pill tabs inside a track. Returns true when the selection changed.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, current: &mut T, items: &[(T, String)]) -> bool {
    let mut changed = false;
    let track = ui.visuals().extreme_bg_color;
    egui::Frame::none().fill(track).rounding(10.0).inner_margin(Margin::same(3.0)).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (value, label) in items {
                let on = *current == *value;
                let text = if on { RichText::new(label).color(ACCENT_TEXT).strong() } else { RichText::new(label) };
                let mut b = egui::Button::new(text).rounding(8.0).min_size(egui::vec2(0.0, 30.0));
                b = if on { b.fill(ACCENT) } else { b.fill(Color32::TRANSPARENT) };
                if ui.add(b).clicked() && !on {
                    *current = *value;
                    changed = true;
                }
            }
        });
    });
    changed
}

/// A coloured banner for notices, errors and confirmations. Returns true
/// when its close button was clicked.
pub fn banner(ui: &mut Ui, color: Color32, text: &str, closable: bool) -> bool {
    let mut close = false;
    egui::Frame::none()
        .fill(color.gamma_multiply(0.16))
        .stroke(Stroke::new(1.0_f32, color.gamma_multiply(0.5)))
        .rounding(8.0)
        .inner_margin(Margin::symmetric(12.0, 7.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(text).color(color));
                if closable {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = ui.add(egui::Button::new("✕").frame(false).small()).clicked();
                    });
                }
            });
        });
    close
}
