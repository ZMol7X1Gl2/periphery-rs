//! Periphery's look: the logo's black and off-white, warm neutral greys,
//! colour only for status, the Inter typeface with Phosphor icons, fixed type
//! and corner scales, and a few shared widgets (cards, chips, tabs).

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, Margin, Response, RichText, Rounding, Stroke, TextStyle, Theme, Ui, Visuals};

/// Icons, drawn from the Phosphor font: `theme::icon::MOUSE`.
pub use egui_phosphor::regular as icon;

/// The logo's black and off-white. The accent is whichever of the two
/// contrasts with the theme: off-white on dark, black on light.
const INK: Color32 = Color32::from_rgb(11, 11, 12);
const PAPER: Color32 = Color32::from_rgb(239, 236, 230);
pub const WARN: Color32 = Color32::from_rgb(230, 170, 60);
pub const GOOD: Color32 = Color32::from_rgb(80, 190, 120);
/// Errors and actions that can't be undone.
pub const DANGER: Color32 = Color32::from_rgb(229, 72, 77);

/// Text sizes in points. Everything the app writes uses one of these.
pub mod size {
    /// The selected device's name.
    pub const TITLE: f32 = 22.0;
    /// The app name and the heading of an editor.
    pub const HEADING: f32 = 17.0;
    /// Card titles.
    pub const SECTION: f32 = 15.0;
    pub const BODY: f32 = 14.0;
    pub const MONO: f32 = 13.0;
    pub const SMALL: f32 = 12.0;
    /// Long labels on keycaps.
    pub const TINY: f32 = 10.0;
}

/// Corner radii in points.
pub mod radius {
    /// Small marks: keys on the mouse drawing, indicator bars.
    pub const SM: f32 = 4.0;
    /// Buttons, inputs, menus, banners.
    pub const MD: f32 = 8.0;
    /// Cards, windows, tab tracks and chips.
    pub const LG: f32 = 12.0;
}

const SEMIBOLD: &str = "semibold";

/// Inter's semibold face. Ask for it with [`Semibold::semibold`].
pub fn semibold() -> FontFamily {
    FontFamily::Name(SEMIBOLD.into())
}

/// egui's `strong()` only brightens text; this also sets it in the
/// semibold face.
pub trait Semibold {
    fn semibold(self) -> Self;
}

impl Semibold for RichText {
    fn semibold(self) -> Self {
        self.family(semibold()).strong()
    }
}

/// What marks the chosen option (tabs, choices, switches, the selected
/// device): the accent fill and the colour of text on it. egui's own
/// selection colour stays a muted grey, because egui also paints it behind
/// selected text.
pub fn accent(v: &Visuals) -> (Color32, Color32) {
    if v.dark_mode { (PAPER, INK) } else { (INK, PAPER) }
}

fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// Installs the fonts and the theme for both dark and light mode.
pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.set_visuals_of(Theme::Dark, dark());
    ctx.set_visuals_of(Theme::Light, light());
    for theme in [Theme::Dark, Theme::Light] {
        ctx.style_mut_of(theme, |s| {
            s.text_styles = [
                (TextStyle::Heading, FontId::new(size::TITLE, semibold())),
                (TextStyle::Body, FontId::proportional(size::BODY)),
                (TextStyle::Button, FontId::proportional(size::BODY)),
                (TextStyle::Small, FontId::proportional(size::SMALL)),
                (TextStyle::Monospace, FontId::monospace(size::MONO)),
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

/// Inter for text, then Phosphor for icons and egui's own fonts for symbols
/// neither has. The bundled Inter is cut down to the scripts the app shows,
/// without its private use area, where Phosphor keeps its icons.
fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("phosphor".into(), egui_phosphor::Variant::Regular.font_data());
    fonts.font_data.insert("inter".into(), FontData::from_static(include_bytes!("../assets/fonts/Inter-Regular.ttf")));
    fonts.font_data.insert("inter-semibold".into(), FontData::from_static(include_bytes!("../assets/fonts/Inter-SemiBold.ttf")));
    let fallbacks = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let family = |text: &str| [text, "phosphor"].map(String::from).into_iter().chain(fallbacks.iter().cloned()).collect::<Vec<_>>();
    fonts.families.insert(FontFamily::Proportional, family("inter"));
    fonts.families.insert(semibold(), family("inter-semibold"));
    fonts
}

fn shape(v: &mut Visuals) {
    let r = Rounding::same(radius::MD);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.rounding = r;
    }
    v.window_rounding = Rounding::same(radius::LG);
    v.menu_rounding = r;
    let (accent, _) = accent(v);
    v.hyperlink_color = accent;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, accent);
    v.slider_trailing_fill = true;
    v.window_shadow.blur = 24.0;
    v.popup_shadow.blur = 12.0;
}

fn dark() -> Visuals {
    let mut v = Visuals::dark();
    v.panel_fill = rgb(0x111113);
    v.window_fill = rgb(0x18181b);
    v.faint_bg_color = rgb(0x18181b);
    v.extreme_bg_color = rgb(0x0b0b0c);
    v.code_bg_color = rgb(0x0b0b0c);
    v.override_text_color = None;
    v.selection.bg_fill = rgb(0x5e5d63);
    v.selection.stroke = Stroke::new(1.0_f32, PAPER);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = rgb(0x18181b);
    w.noninteractive.weak_bg_fill = rgb(0x18181b);
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, rgb(0x28282c));
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, rgb(0xc8c6c0));
    w.inactive.bg_fill = rgb(0x262629);
    w.inactive.weak_bg_fill = rgb(0x232326);
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0_f32, rgb(0xe6e4de));
    w.hovered.bg_fill = rgb(0x303034);
    w.hovered.weak_bg_fill = rgb(0x2d2d31);
    w.hovered.bg_stroke = Stroke::new(1.0_f32, rgb(0x424147));
    w.hovered.fg_stroke = Stroke::new(1.5_f32, rgb(0xf5f3ee));
    w.active.bg_fill = rgb(0x38383d);
    w.active.weak_bg_fill = rgb(0x38383d);
    w.active.fg_stroke = Stroke::new(2.0_f32, PAPER);
    w.open = w.hovered;
    shape(&mut v);
    v
}

fn light() -> Visuals {
    let mut v = Visuals::light();
    v.panel_fill = rgb(0xf4f3ef);
    v.window_fill = rgb(0xfdfcfa);
    v.faint_bg_color = rgb(0xfdfcfa);
    v.extreme_bg_color = rgb(0xeae8e2);
    v.selection.bg_fill = rgb(0xbfbbb2);
    v.selection.stroke = Stroke::new(1.0_f32, INK);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, rgb(0xdedbd4));
    v.widgets.inactive.weak_bg_fill = rgb(0xe7e4de);
    v.widgets.inactive.bg_fill = rgb(0xe2dfd8);
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.hovered.weak_bg_fill = rgb(0xdcd9d2);
    v.widgets.hovered.bg_fill = rgb(0xd8d5cd);
    shape(&mut v);
    v
}

/// The app mark (docs/assets/logo.svg and logo-dark.svg) as 96×96 RGBA, so
/// the app needs no SVG renderer. Re-export these when the SVGs change.
const LOGO: &[u8] = include_bytes!("../assets/logo-96.rgba");
const LOGO_DARK: &[u8] = include_bytes!("../assets/logo-dark-96.rgba");
const LOGO_SIZE: usize = 96;

/// The window icon: docs/assets/logo.svg as 256×256 RGBA.
pub fn window_icon() -> egui::IconData {
    egui::IconData { rgba: include_bytes!("../assets/icon-256.rgba").to_vec(), width: 256, height: 256 }
}

/// The app mark: a tile with stars and a P. Light on a dark theme, dark on a
/// light one, so it stands out from the sidebar either way.
pub fn logo(ui: &mut Ui, size: f32) {
    let dark = ui.visuals().dark_mode;
    let id = egui::Id::new(("periphery-logo", dark));
    let texture = ui.ctx().data(|d| d.get_temp::<egui::TextureHandle>(id)).unwrap_or_else(|| {
        let image = egui::ColorImage::from_rgba_unmultiplied([LOGO_SIZE; 2], if dark { LOGO } else { LOGO_DARK });
        let texture = ui.ctx().load_texture("logo", image, egui::TextureOptions::LINEAR);
        ui.ctx().data_mut(|d| d.insert_temp(id, texture.clone()));
        texture
    });
    ui.image((texture.id(), egui::vec2(size, size)));
}

/// The panel behind a group of settings.
pub fn card(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    let v = ui.visuals();
    egui::Frame::none()
        .fill(v.faint_bg_color)
        .stroke(v.widgets.noninteractive.bg_stroke)
        .rounding(radius::LG)
        .inner_margin(Margin::same(16.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() {
                ui.label(RichText::new(title).size(size::SECTION).semibold());
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
        .rounding(radius::LG)
        .inner_margin(Margin::symmetric(9.0, 3.0))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            ui.label(RichText::new(key).small().weak());
            ui.label(RichText::new(value).small().semibold());
        })
        .response
}

/// A button that shows whether it's the current choice. egui draws selected
/// buttons square and outlined; this keeps the theme's corners and a plain
/// accent fill.
pub fn choice_button<'a>(ui: &Ui, text: &str, selected: bool) -> egui::Button<'a> {
    if !selected {
        return egui::Button::new(text.to_string()).rounding(radius::MD);
    }
    let (fill, on) = accent(ui.visuals());
    egui::Button::new(RichText::new(text).color(on)).selected(true).rounding(radius::MD).fill(fill).stroke(Stroke::NONE)
}

/// A row of pill tabs inside a track. Returns true when the selection changed.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, current: &mut T, items: &[(T, String)]) -> bool {
    let mut changed = false;
    let track = ui.visuals().extreme_bg_color;
    let (fill, on_fill) = accent(ui.visuals());
    egui::Frame::none().fill(track).rounding(radius::LG).inner_margin(Margin::same(3.0)).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (value, label) in items {
                let on = *current == *value;
                let text = if on { RichText::new(label).color(on_fill).semibold() } else { RichText::new(label) };
                let mut b = egui::Button::new(text).rounding(radius::MD).min_size(egui::vec2(0.0, 30.0));
                b = if on { b.fill(fill) } else { b.fill(Color32::TRANSPARENT) };
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
        .rounding(radius::MD)
        .inner_margin(Margin::symmetric(12.0, 7.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(text).color(color));
                if closable {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = ui.add(egui::Button::new(icon::X).frame(false).small()).clicked();
                    });
                }
            });
        });
    close
}
