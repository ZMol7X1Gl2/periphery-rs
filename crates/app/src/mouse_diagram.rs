//! A generic right-handed mouse seen from above, drawn with the painter.
//! Only the buttons a device reports are drawn as buttons; clicking one
//! selects it. Coordinates are in units of the body's half length, with the
//! origin in the middle of the body and y growing towards the back.

use std::collections::BTreeMap;

use std::sync::OnceLock;

use eframe::egui::{self, Align2, Color32, FontId, Mesh, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};
use periphery_core::ButtonPos;

use crate::theme::{radius, size};

/// Half the body length in points.
const R: f32 = 150.0;
/// Half the gap between the left and right buttons, where the wheel sits.
const GAP: f32 = 0.075;
/// Where the main buttons end and the palm rest begins.
const SPLIT: f32 = -0.08;
const SIZE: egui::Vec2 = vec2(1.5 * R, 2.5 * R);

/// What the diagram shows for one button.
pub struct ButtonInfo {
    pub name: String,
    /// The current binding, shown on the big buttons and on hover.
    pub binding: String,
}

/// The right half of the outline, front to back: a blunt nose, nearly
/// straight sides with a slight waist for the grip, and a wider round back.
const OUTLINE: [(f32, f32); 14] = [
    (0.0, -1.0),
    (0.2, -0.985),
    (0.34, -0.93),
    (0.42, -0.8),
    (0.45, -0.6),
    (0.45, -0.35),
    (0.43, -0.1),
    (0.42, 0.12),
    (0.44, 0.35),
    (0.47, 0.58),
    (0.46, 0.78),
    (0.38, 0.93),
    (0.22, 1.0),
    (0.0, 1.02),
];

/// [`OUTLINE`] smoothed with a Catmull-Rom spline.
fn right_side() -> &'static [Pos2] {
    static SIDE: OnceLock<Vec<Pos2>> = OnceLock::new();
    SIDE.get_or_init(|| {
        let p: Vec<Pos2> = OUTLINE.iter().map(|(x, y)| pos2(*x, *y)).collect();
        let at = |i: isize| p[i.clamp(0, p.len() as isize - 1) as usize];
        let mut out = Vec::new();
        for i in 0..p.len() as isize - 1 {
            let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
            for k in 0..8 {
                let t = k as f32 / 8.0;
                let (t2, t3) = (t * t, t * t * t);
                let c = |a: f32, b: f32, c: f32, d: f32| {
                    0.5 * (2.0 * b + (c - a) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (3.0 * b - a - 3.0 * c + d) * t3)
                };
                out.push(pos2(c(p0.x, p1.x, p2.x, p3.x), c(p0.y, p1.y, p2.y, p3.y)));
            }
        }
        out.push(*p.last().unwrap());
        out
    })
}

/// Half the body width at height `y` (-1 front … about 1 back).
fn half_width(y: f32) -> f32 {
    let side = right_side();
    match side.windows(2).find(|w| (w[0].y..=w[1].y).contains(&y)) {
        Some(w) => egui::lerp(w[0].x..=w[1].x, (y - w[0].y) / (w[1].y - w[0].y).max(1e-6)),
        None => 0.0,
    }
}

fn steps(from: f32, to: f32, n: usize) -> impl Iterator<Item = f32> {
    (0..=n).map(move |i| from + (to - from) * i as f32 / n as f32)
}

fn body() -> Vec<Pos2> {
    let side = right_side();
    side.iter().copied().chain(side.iter().rev().map(|p| pos2(-p.x, p.y))).collect()
}

/// Fills a polygon that is star-shaped around its centroid (the outline has
/// a waist, so it isn't convex) and strokes its edge.
fn polygon(painter: &Painter, pts: &[Pos2], fill: Color32, stroke: Stroke) {
    let centre = pts.iter().fold(egui::Vec2::ZERO, |acc, p| acc + p.to_vec2()) / pts.len() as f32;
    let mut mesh = Mesh::default();
    mesh.colored_vertex(centre.to_pos2(), fill);
    for p in pts {
        mesh.colored_vertex(*p, fill);
    }
    let n = pts.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    painter.add(Shape::mesh(mesh));
    let edge = if stroke.width > 0.0 { stroke } else { Stroke::new(1.0_f32, fill) };
    painter.add(Shape::closed_line(pts.to_vec(), edge));
}

/// The left main button; the right one is its mirror image.
fn left_button() -> Vec<Pos2> {
    let mut pts: Vec<Pos2> = steps(SPLIT, -1.0, 32).map(|y| pos2(-half_width(y).max(GAP), y)).collect();
    pts.push(pos2(-GAP, SPLIT));
    pts
}

/// The slot between the main buttons that the wheel sits in.
fn channel() -> Vec<Pos2> {
    let left = steps(SPLIT, -1.0, 16).map(|y| pos2(-half_width(y).min(GAP), y));
    let right = steps(-1.0, SPLIT, 16).map(|y| pos2(half_width(y).min(GAP), y));
    left.chain(right).collect()
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

fn mirror(pts: &[Pos2]) -> Vec<Pos2> {
    pts.iter().rev().map(|p| pos2(-p.x, p.y)).collect()
}

fn unit_rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
    Rect::from_min_max(pos2(x0, y0), pos2(x1, y1))
}

/// A side button on the left flank, from `y0` to `y1`.
fn side(y0: f32, y1: f32) -> Rect {
    let x = -half_width((y0 + y1) / 2.0);
    unit_rect(x - 0.075, y0, x + 0.025, y1)
}

const WHEEL: (f32, f32, f32, f32) = (-0.05, -0.72, 0.05, -0.42);

/// The clickable area of every button except the two main ones.
fn hotspot(pos: ButtonPos) -> Option<Rect> {
    Some(match pos {
        ButtonPos::Left | ButtonPos::Right => return None,
        ButtonPos::Middle => unit_rect(WHEEL.0, WHEEL.1, WHEEL.2, WHEEL.3),
        ButtonPos::WheelUp => unit_rect(-0.06, -0.86, 0.06, -0.75),
        ButtonPos::WheelDown => unit_rect(-0.06, -0.39, 0.06, -0.30),
        ButtonPos::TiltLeft => unit_rect(-0.19, -0.62, -0.08, -0.52),
        ButtonPos::TiltRight => unit_rect(0.08, -0.62, 0.19, -0.52),
        ButtonPos::Top => unit_rect(-0.045, -0.27, 0.045, -0.13),
        ButtonPos::TopFront => unit_rect(-0.045, -0.28, 0.045, -0.21),
        ButtonPos::TopBack => unit_rect(-0.045, -0.19, 0.045, -0.12),
        ButtonPos::Forward => side(0.0, 0.2),
        ButtonPos::Back => side(0.23, 0.43),
        ButtonPos::Thumb => side(0.5, 0.62),
        ButtonPos::Bottom => unit_rect(-0.2, 1.12, 0.2, 1.26),
    })
}

fn inside(poly: &[Pos2], p: Pos2) -> bool {
    let mut hit = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            hit = !hit;
        }
        j = i;
    }
    hit
}

fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { format!("{}…", s.chars().take(max - 1).collect::<String>()) }
}

/// Draws the mouse and returns the button clicked this frame.
pub fn show(ui: &mut Ui, buttons: &BTreeMap<ButtonPos, ButtonInfo>, selected: Option<ButtonPos>) -> Option<ButtonPos> {
    let (rect, _) = ui.allocate_exact_size(SIZE, Sense::hover());
    let origin = pos2(rect.center().x, rect.top() + R + 6.0);
    let to_screen = |p: Pos2| origin + p.to_vec2() * R;
    let to_screen_rect = |r: Rect| Rect::from_min_max(to_screen(r.min), to_screen(r.max));
    let screen = |pts: &[Pos2]| pts.iter().map(|p| to_screen(*p)).collect::<Vec<_>>();

    let v = ui.visuals().clone();
    let outline = v.widgets.noninteractive.bg_stroke;
    let painter = ui.painter_at(rect);
    let hover = ui.input(|i| i.pointer.hover_pos());
    let fill = |pos: ButtonPos, hovered: bool| {
        if selected == Some(pos) {
            v.selection.bg_fill
        } else if hovered {
            v.widgets.hovered.weak_bg_fill
        } else {
            v.widgets.inactive.weak_bg_fill
        }
    };
    let stroke = |pos: ButtonPos| if selected == Some(pos) { v.selection.stroke } else { outline };
    let mut clicked = None;

    let shell = mix(v.panel_fill, v.widgets.inactive.weak_bg_fill, 0.45);
    polygon(&painter, &screen(&body()), shell, Stroke::new(1.5_f32, outline.color));
    polygon(&painter, &screen(&channel()), v.extreme_bg_color, Stroke::NONE);

    // The main buttons are exact shapes, so test the pointer against them.
    let left = left_button();
    for (pos, poly) in [(ButtonPos::Left, left.clone()), (ButtonPos::Right, mirror(&left))] {
        let pts = screen(&poly);
        let Some(info) = buttons.get(&pos) else {
            polygon(&painter, &pts, shell, outline);
            continue;
        };
        let bounds = Rect::from_points(&pts);
        let resp = ui.interact(bounds, ui.id().with(("mouse-button", pos)), Sense::click());
        let over = hover.is_some_and(|p| inside(&pts, p)) && resp.hovered();
        polygon(&painter, &pts, fill(pos, over), stroke(pos));
        let side = if pos == ButtonPos::Left { -1.0 } else { 1.0 };
        let text_color = if selected == Some(pos) { v.selection.stroke.color } else { v.text_color() };
        painter.text(to_screen(pos2(side * 0.28, -0.3)), Align2::CENTER_CENTER, shorten(&info.binding, 11), FontId::proportional(size::SMALL), text_color);
        if resp.clicked() && resp.interact_pointer_pos().is_some_and(|p| inside(&pts, p)) {
            clicked = Some(pos);
        }
        if over {
            resp.on_hover_text(format!("{}: {}", info.name, info.binding));
        }
    }

    // The wheel slot is always drawn; it's part of what makes it a mouse.
    let wheel = to_screen_rect(unit_rect(WHEEL.0, WHEEL.1, WHEEL.2, WHEEL.3));
    painter.rect(wheel.expand(3.0), radius::MD, v.extreme_bg_color, outline);

    for pos in ButtonPos::ALL {
        let (Some(info), Some(spot)) = (buttons.get(&pos), hotspot(pos)) else { continue };
        let r = to_screen_rect(spot);
        let resp = ui.interact(r, ui.id().with(("mouse-button", pos)), Sense::click());
        let f = fill(pos, resp.hovered());
        let st = stroke(pos);
        let arrow = |dir: egui::Vec2| {
            let c = r.center();
            let (a, b) = (dir * 6.0, dir.rot90() * 5.0);
            Shape::convex_polygon(vec![c + a, c - a + b, c - a - b], if selected == Some(pos) { v.selection.stroke.color } else { v.text_color() }, Stroke::NONE)
        };
        match pos {
            ButtonPos::WheelUp | ButtonPos::WheelDown | ButtonPos::TiltLeft | ButtonPos::TiltRight => {
                painter.rect(r, radius::SM, f, st);
                let dir = match pos {
                    ButtonPos::WheelUp => vec2(0.0, -1.0),
                    ButtonPos::WheelDown => vec2(0.0, 1.0),
                    ButtonPos::TiltLeft => vec2(-1.0, 0.0),
                    _ => vec2(1.0, 0.0),
                };
                painter.add(arrow(dir));
            }
            ButtonPos::Middle => {
                painter.rect(r, radius::MD, f, st);
                for i in 1..6 {
                    let y = egui::lerp(r.top()..=r.bottom(), i as f32 / 6.0);
                    painter.line_segment([pos2(r.left() + 3.0, y), pos2(r.right() - 3.0, y)], Stroke::new(1.0_f32, outline.color));
                }
            }
            ButtonPos::Bottom => {
                painter.rect(r, r.height() / 2.0, f, st);
                painter.text(r.center(), Align2::CENTER_CENTER, "underside", FontId::proportional(size::SMALL), v.weak_text_color());
            }
            _ => {
                painter.rect(r, radius::SM, f, st);
            }
        }
        if resp.clicked() {
            clicked = Some(pos);
        }
        resp.on_hover_text(format!("{}: {}", info.name, info.binding));
    }
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotspots_stay_apart() {
        let spots: Vec<(ButtonPos, Rect)> = ButtonPos::ALL.iter().filter_map(|p| Some((*p, hotspot(*p)?))).collect();
        for (i, (a, ra)) in spots.iter().enumerate() {
            for (b, rb) in &spots[i + 1..] {
                // Top is the single-button variant of TopFront/TopBack.
                let alternatives = matches!((a, b), (ButtonPos::Top, ButtonPos::TopFront | ButtonPos::TopBack));
                assert!(alternatives || !ra.intersects(*rb), "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn main_buttons_are_mirrored_and_contain_their_centre() {
        let left = left_button();
        assert!(inside(&left, pos2(-0.28, -0.3)));
        assert!(!inside(&left, pos2(0.28, -0.3)));
        assert!(inside(&mirror(&left), pos2(0.28, -0.3)));
    }
}
