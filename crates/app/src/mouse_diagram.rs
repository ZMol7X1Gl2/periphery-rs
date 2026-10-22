//! A right-handed mouse seen from above, drawn low-poly: a height field over
//! the outline is cut into a coarse triangle mesh and every facet is shaded
//! flat by a fixed light. Only the buttons a device reports are drawn as
//! buttons; clicking one selects it. Coordinates are in units of the body's
//! half length, with the origin in the middle of the body and y growing
//! towards the back.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use eframe::egui::{self, Align2, Color32, FontId, Mesh, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};
use periphery_core::ButtonPos;

use crate::theme::{self, radius, size};

/// Half the body length in points.
const R: f32 = 165.0;
/// Space above the body, and below it for the shadow.
const MARGIN: f32 = 8.0;

/// The right half of the outline, front to back: a flat, wide front with
/// rounded corners, nearly straight sides and a round back.
const OUTLINE: [(f32, f32); 14] = [
    (0.45, -1.0),
    (0.475, -0.975),
    (0.485, -0.92),
    (0.49, -0.7),
    (0.48, -0.4),
    (0.47, -0.1),
    (0.48, 0.2),
    (0.48, 0.42),
    (0.46, 0.6),
    (0.42, 0.75),
    (0.35, 0.87),
    (0.25, 0.95),
    (0.13, 0.99),
    (0.0, 1.0),
];

/// Where the main buttons end and the palm rest begins.
const SPLIT: f32 = -0.03;
/// The strip of shell between the main buttons and the edge.
const RIM_WIDTH: f32 = 0.055;
/// Mesh rows (y) and columns (fraction of the half width). Column ±RIM is
/// only nominal: those vertices keep the rim strip [`RIM_WIDTH`] wide.
const ROWS: [f32; 14] = [-1.0, -0.97, -0.88, -0.7, -0.45, -0.22, SPLIT, 0.2, 0.42, 0.62, 0.76, 0.87, 0.94, 0.985];
const RIM: f32 = 0.88;
const COLS: [f32; 9] = [-1.0, -RIM, -0.58, -0.28, 0.0, 0.28, 0.58, RIM, 1.0];
const MID: usize = COLS.len() / 2;
/// Light from the top left, in front of the mouse.
const LIGHT: [f32; 3] = [-0.75, -0.55, 0.55];

const WHEEL: Rect = Rect { min: pos2(-0.055, -0.76), max: pos2(0.055, -0.46) };

/// What the diagram shows for one button.
pub struct ButtonInfo {
    pub name: String,
    /// The current binding, shown on the big buttons and on hover.
    pub binding: String,
}

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

/// Half the body width at height `y` (-1 front … 1 back).
fn half_width(y: f32) -> f32 {
    let side = right_side();
    match side.windows(2).find(|w| (w[0].y..=w[1].y).contains(&y)) {
        Some(w) => egui::lerp(w[0].x..=w[1].x, (y - w[0].y) / (w[1].y - w[0].y).max(1e-6)),
        None => 0.0,
    }
}

/// Shell height at `u` (-1…1 across the body) and length `y`: flat on top,
/// falling away steeply at the sides, highest under the palm.
fn height(u: f32, y: f32) -> f32 {
    let hump = 0.5 + 0.5 * (-((y - 0.3) / 0.8).powi(2)).exp();
    0.5 * (1.0 - u.powi(4)).max(0.0).sqrt() * hump
}

/// A small fixed offset per inner vertex, so the facets are irregular. The
/// outline, the rims, the middle seam and the split row stay put so the
/// buttons' edges are straight.
fn jitter(i: usize, j: usize) -> (f32, f32) {
    if [0, 1, MID, COLS.len() - 2, COLS.len() - 1].contains(&j) || ROWS[i] == SPLIT || i == 0 || i == ROWS.len() - 1 {
        return (0.0, 0.0);
    }
    let h = (i as f64 * 12.9898 + j as f64 * 78.233).sin() * 43758.5453;
    let a = (h - h.floor()) as f32;
    // Calmer towards the round back, where big offsets make it lumpy.
    let k = if ROWS[i] < 0.6 { 1.0 } else { 0.4 };
    ((a - 0.5) * 0.16 * k, ((a * 7.0).fract() - 0.5) * 0.09 * k)
}

type Vertex = [f32; 3];

fn vertex(i: usize, j: usize) -> Vertex {
    let (du, dy) = jitter(i, j);
    let y = ROWS[i] + dy;
    let mut u = (COLS[j] + du).clamp(-1.0, 1.0);
    let w = half_width(y);
    let mut x = u * w;
    if COLS[j].abs() == RIM {
        x = (w - RIM_WIDTH).copysign(COLS[j]);
        u = x / w;
    }
    [x, y, height(u, y)]
}

fn grid() -> &'static Vec<Vec<Vertex>> {
    static GRID: OnceLock<Vec<Vec<Vertex>>> = OnceLock::new();
    GRID.get_or_init(|| (0..ROWS.len()).map(|i| (0..COLS.len()).map(|j| vertex(i, j)).collect()).collect())
}

/// One flat-shaded triangle: its corners, the main button it belongs to
/// (`None` for the shell) and how much light it catches (0…1).
struct Facet {
    pts: [Pos2; 3],
    button: Option<ButtonPos>,
    light: f32,
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
    [v[0] / l, v[1] / l, v[2] / l]
}

fn lambert(t: [Vertex; 3]) -> f32 {
    let [a, b, c] = t;
    let (u, v) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
    let mut n = normalize([u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]]);
    if n[2] < 0.0 {
        n = [-n[0], -n[1], -n[2]];
    }
    let l = normalize(LIGHT);
    (n[0] * l[0] + n[1] * l[1] + n[2] * l[2]).max(0.0)
}

/// The mesh, cut along the grid. A cell inside the rims and in front of the
/// split row belongs to the left or right button by its column, so button
/// edges follow the seams exactly.
fn facets() -> &'static [Facet] {
    static FACETS: OnceLock<Vec<Facet>> = OnceLock::new();
    FACETS.get_or_init(|| {
        let g = grid();
        let mut out = Vec::new();
        for i in 0..ROWS.len() - 1 {
            for j in 0..COLS.len() - 1 {
                let (a, b, c, d) = (g[i][j], g[i][j + 1], g[i + 1][j + 1], g[i + 1][j]);
                let tris = if (i + j) % 2 == 0 { [[a, b, c], [a, c, d]] } else { [[a, b, d], [b, c, d]] };
                let button = (ROWS[i + 1] <= SPLIT && (1..COLS.len() - 2).contains(&j))
                    .then_some(if j < MID { ButtonPos::Left } else { ButtonPos::Right });
                for t in tris {
                    let pts = t.map(|v| pos2(v[0], v[1]));
                    let area = ((pts[1] - pts[0]).x * (pts[2] - pts[0]).y - (pts[2] - pts[0]).x * (pts[1] - pts[0]).y).abs();
                    if area > 1e-5 {
                        // A little variation per facet, as in low-poly art.
                        let wobble = (out.len() as f32 * 91.7).sin() * 0.04;
                        out.push(Facet { pts, button, light: (lambert(t) + wobble).clamp(0.0, 1.0) });
                    }
                }
            }
        }
        out
    })
}

/// x of the mesh's left edge at `y`. It runs in straight segments between
/// rows, just inside the smooth outline.
fn left_edge(y: f32) -> f32 {
    let g = grid();
    for w in g.windows(2) {
        let (a, b) = (w[0][0], w[1][0]);
        if (a[1]..=b[1]).contains(&y) {
            return egui::lerp(a[0]..=b[0], (y - a[1]) / (b[1] - a[1]).max(1e-6));
        }
    }
    -half_width(y)
}

fn unit_rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
    Rect::from_min_max(pos2(x0, y0), pos2(x1, y1))
}

/// Side buttons are thin strips inside the left rim, from `y0` to `y1`.
fn side(y0: f32, y1: f32) -> Rect {
    let x = left_edge(y0).max(left_edge(y1)).max(left_edge((y0 + y1) / 2.0));
    unit_rect(x + 0.012, y0, x + RIM_WIDTH - 0.012, y1)
}

/// The clickable area of every button except the two main ones.
fn hotspot(pos: ButtonPos) -> Option<Rect> {
    Some(match pos {
        ButtonPos::Left | ButtonPos::Right => return None,
        ButtonPos::Middle => WHEEL,
        ButtonPos::WheelUp => unit_rect(-0.05, -0.9, 0.05, -0.8),
        ButtonPos::WheelDown => unit_rect(-0.05, -0.42, 0.05, -0.33),
        ButtonPos::TiltLeft => unit_rect(-0.17, -0.66, -0.085, -0.56),
        ButtonPos::TiltRight => unit_rect(0.085, -0.66, 0.17, -0.56),
        ButtonPos::Top => unit_rect(-0.04, -0.27, 0.04, -0.12),
        ButtonPos::TopFront => unit_rect(-0.04, -0.28, 0.04, -0.21),
        ButtonPos::TopBack => unit_rect(-0.04, -0.19, 0.04, -0.12),
        ButtonPos::Forward => side(-0.36, -0.1),
        ButtonPos::Back => side(-0.06, 0.22),
        ButtonPos::Thumb => side(0.27, 0.45),
        ButtonPos::Bottom => unit_rect(-0.2, 1.08, 0.2, 1.22),
    })
}

fn inside(t: &[Pos2; 3], p: Pos2) -> bool {
    let cross = |a: Pos2, b: Pos2| (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
    let (d1, d2, d3) = (cross(t[0], t[1]), cross(t[1], t[2]), cross(t[2], t[0]));
    !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
}

/// The main button under `p` (in body units), if any.
fn main_button_at(p: Pos2) -> Option<ButtonPos> {
    facets().iter().find(|f| f.button.is_some() && inside(&f.pts, p)).and_then(|f| f.button)
}

fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { format!("{}…", s.chars().take(max - 1).collect::<String>()) }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// The shades a facet's light picks between, for the shell and for the
/// selected button, plus the seams between parts.
struct Palette {
    shell: (Color32, Color32),
    chosen: (Color32, Color32),
    seam: Color32,
    /// Outline of side and top buttons, so they show on a dark shell.
    edge: Color32,
    /// How much lighter small buttons are than the shell around them.
    lift: f32,
}

impl Palette {
    fn new(v: &egui::Visuals) -> Self {
        let (accent, _) = theme::accent(v);
        let rgb = |hex: u32| Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8);
        if v.dark_mode {
            Palette {
                shell: (rgb(0x1d1d21), rgb(0x4a4a52)),
                chosen: (rgb(0x8f8c85), accent),
                seam: rgb(0x0b0b0c),
                edge: rgb(0x8a8890),
                lift: 0.35,
            }
        } else {
            Palette {
                shell: (rgb(0xb9b5ac), rgb(0xf6f4ef)),
                chosen: (accent, rgb(0x3a3a40)),
                seam: rgb(0x2a2a2e),
                edge: rgb(0x2a2a2e),
                lift: 0.0,
            }
        }
    }

    fn shade(&self, light: f32, chosen: bool, hovered: bool) -> Color32 {
        let (lo, hi) = if chosen { self.chosen } else { self.shell };
        let t = light.powf(1.3) + if hovered { 0.18 } else { 0.0 };
        mix(lo, hi, t.min(1.0))
    }
}

/// Fills one triangle per facet with its own flat colour.
fn add_facets(painter: &Painter, to_screen: impl Fn(Pos2) -> Pos2, facets: impl Iterator<Item = ([Pos2; 3], Color32)>) {
    let mut mesh = Mesh::default();
    for (pts, color) in facets {
        let base = mesh.vertices.len() as u32;
        for p in pts {
            mesh.colored_vertex(to_screen(p), color);
        }
        mesh.add_triangle(base, base + 1, base + 2);
    }
    painter.add(Shape::mesh(mesh));
}

/// Draws the mouse and returns the button clicked this frame.
pub fn show(ui: &mut Ui, buttons: &BTreeMap<ButtonPos, ButtonInfo>, selected: Option<ButtonPos>) -> Option<ButtonPos> {
    // Room below the body only when there's an underside button to show.
    let bottom = if buttons.contains_key(&ButtonPos::Bottom) { hotspot(ButtonPos::Bottom).unwrap().bottom() } else { 1.0 };
    let (rect, _) = ui.allocate_exact_size(vec2(1.1 * R, (1.0 + bottom) * R + 2.0 * MARGIN), Sense::hover());
    let origin = pos2(rect.center().x, rect.top() + R + MARGIN);
    let to_screen = |p: Pos2| origin + p.to_vec2() * R;
    let to_screen_rect = |r: Rect| Rect::from_min_max(to_screen(r.min), to_screen(r.max));
    let to_body = |p: Pos2| pos2((p.x - origin.x) / R, (p.y - origin.y) / R);

    let v = ui.visuals().clone();
    let pal = Palette::new(&v);
    let (_, on_accent) = theme::accent(&v);
    let painter = ui.painter_at(rect);
    let hover = ui.input(|i| i.pointer.hover_pos()).filter(|p| rect.contains(*p));
    let mut clicked = None;

    // The main buttons are exact shapes, so test the pointer against them.
    let front = Rect::from_min_max(to_screen(pos2(-0.5, -1.0)), to_screen(pos2(0.5, SPLIT)));
    let resp = ui.interact(front, ui.id().with("mouse-main-buttons"), Sense::click());
    let over_main = hover.filter(|_| resp.hovered()).map(to_body).and_then(main_button_at).filter(|b| buttons.contains_key(b));
    if resp.clicked() {
        clicked = resp.interact_pointer_pos().map(to_body).and_then(main_button_at).filter(|b| buttons.contains_key(b));
    }

    // Shadow, then the shell, its facets coloured by part.
    let shadow = if v.dark_mode { Color32::from_black_alpha(90) } else { Color32::from_black_alpha(24) };
    painter.add(Shape::ellipse_filled(origin + vec2(3.0, 4.0), vec2(0.45 * R, 0.96 * R), shadow));
    add_facets(
        &painter,
        to_screen,
        facets().iter().map(|f| {
            let pressable = f.button.filter(|b| buttons.contains_key(b));
            let chosen = pressable.is_some() && pressable == selected;
            (f.pts, pal.shade(f.light, chosen, pressable.is_some() && pressable == over_main))
        }),
    );

    // Seams: between the main buttons, along their rims and where they end.
    let g = grid();
    let split = ROWS.iter().position(|r| *r == SPLIT).unwrap();
    let at = |i: usize, j: usize| pos2(g[i][j][0], g[i][j][1]);
    let line = |pts: Vec<Pos2>, width: f32| {
        painter.add(Shape::line(pts.into_iter().map(to_screen).collect(), Stroke::new(width, pal.seam)));
    };
    line((0..=split).map(|i| at(i, MID)).collect(), 2.0);
    line((1..COLS.len() - 1).map(|j| at(split, j)).collect(), 1.6);
    for j in [1, COLS.len() - 2] {
        line((0..=split).map(|i| at(i, j)).collect(), 1.3);
    }

    // Labels on the main buttons.
    for (pos, x) in [(ButtonPos::Left, -0.22), (ButtonPos::Right, 0.22)] {
        let Some(info) = buttons.get(&pos) else { continue };
        let color = if selected == Some(pos) { on_accent } else { v.text_color() };
        painter.text(to_screen(pos2(x, -0.3)), Align2::CENTER_CENTER, shorten(&info.binding, 11), FontId::proportional(size::SMALL), color);
    }
    if let Some(pos) = over_main {
        resp.on_hover_text(format!("{}: {}", buttons[&pos].name, buttons[&pos].binding));
    }

    // The wheel is always drawn; it's part of what makes it a mouse: a
    // faceted roller in a dark slot.
    let wheel = to_screen_rect(WHEEL);
    painter.rect_filled(wheel.expand(3.0), radius::MD, pal.seam);
    let wheel_resp = buttons
        .contains_key(&ButtonPos::Middle)
        .then(|| ui.interact(wheel, ui.id().with(("mouse-button", ButtonPos::Middle)), Sense::click()));
    let wheel_on = selected == Some(ButtonPos::Middle) && wheel_resp.is_some();
    let wheel_hover = wheel_resp.as_ref().is_some_and(|r| r.hovered());
    const ROLLER: [f32; 6] = [0.35, 0.75, 0.95, 0.75, 0.5, 0.3];
    let mut roller = Mesh::default();
    for (k, f) in ROLLER.iter().enumerate() {
        let x0 = egui::lerp(wheel.left()..=wheel.right(), k as f32 / ROLLER.len() as f32);
        let x1 = egui::lerp(wheel.left()..=wheel.right(), (k + 1) as f32 / ROLLER.len() as f32);
        roller.add_colored_rect(Rect::from_x_y_ranges(x0..=x1, wheel.y_range()), pal.shade(*f, wheel_on, wheel_hover));
    }
    painter.add(Shape::mesh(roller));
    for k in 1..6 {
        let y = egui::lerp(wheel.top()..=wheel.bottom(), k as f32 / 6.0);
        painter.line_segment([pos2(wheel.left(), y), pos2(wheel.right(), y)], Stroke::new(1.0_f32, pal.seam.gamma_multiply(0.5)));
    }
    if let Some(r) = wheel_resp {
        if r.clicked() {
            clicked = Some(ButtonPos::Middle);
        }
        let info = &buttons[&ButtonPos::Middle];
        r.on_hover_text(format!("{}: {}", info.name, info.binding));
    }

    // Every other button the device has, on top of the shell.
    for pos in ButtonPos::ALL {
        let (Some(info), Some(spot)) = (buttons.get(&pos), hotspot(pos)) else { continue };
        if pos == ButtonPos::Middle {
            continue;
        }
        let r = to_screen_rect(spot);
        let resp = ui.interact(r, ui.id().with(("mouse-button", pos)), Sense::click());
        let on = selected == Some(pos);
        let fill = pal.shade((0.6 + pal.lift).min(1.0), on, resp.hovered());
        let mark = if on { on_accent } else { v.text_color() };
        let arrow = |dir: egui::Vec2| {
            let c = r.center();
            let (a, b) = (dir * 5.0, dir.rot90() * 4.0);
            Shape::convex_polygon(vec![c + a, c - a + b, c - a - b], mark, Stroke::NONE)
        };
        match pos {
            ButtonPos::WheelUp | ButtonPos::WheelDown | ButtonPos::TiltLeft | ButtonPos::TiltRight => {
                painter.rect(r, radius::SM, fill, Stroke::new(1.0_f32, pal.edge));
                let dir = match pos {
                    ButtonPos::WheelUp => vec2(0.0, -1.0),
                    ButtonPos::WheelDown => vec2(0.0, 1.0),
                    ButtonPos::TiltLeft => vec2(-1.0, 0.0),
                    _ => vec2(1.0, 0.0),
                };
                painter.add(arrow(dir));
            }
            ButtonPos::Bottom => {
                painter.rect(r, r.height() / 2.0, fill, Stroke::new(1.0_f32, pal.edge));
                painter.text(r.center(), Align2::CENTER_CENTER, "underside", FontId::proportional(size::SMALL), mark);
            }
            _ => {
                painter.rect(r, radius::SM / 2.0, fill, Stroke::new(1.3_f32, pal.edge));
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
    fn main_buttons_are_mirrored_and_end_at_the_split() {
        assert_eq!(main_button_at(pos2(-0.24, -0.3)), Some(ButtonPos::Left));
        assert_eq!(main_button_at(pos2(0.24, -0.3)), Some(ButtonPos::Right));
        // The palm rest and the rim strip belong to no button.
        assert_eq!(main_button_at(pos2(-0.24, 0.3)), None);
        assert_eq!(main_button_at(pos2(-half_width(-0.3) + 0.02, -0.3)), None);
    }

    #[test]
    fn side_buttons_sit_inside_the_body() {
        for pos in [ButtonPos::Forward, ButtonPos::Back, ButtonPos::Thumb] {
            let r = hotspot(pos).unwrap();
            for y in [r.top(), r.bottom()] {
                assert!(r.left() > left_edge(y), "{pos:?} sticks out at y={y}");
            }
            // And inside the rim strip, clear of the main buttons.
            assert_eq!(main_button_at(r.right_center()), None, "{pos:?}");
        }
    }
}
