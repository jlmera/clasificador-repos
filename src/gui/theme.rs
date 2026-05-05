//! Paleta de colores del tema verde sage (idéntica al Python).

use egui::Color32;

pub const BG:        Color32 = Color32::from_rgb(0xff, 0xff, 0xff);
pub const PANEL:     Color32 = Color32::from_rgb(0xf6, 0xf8, 0xfa);
pub const PANEL2:    Color32 = Color32::from_rgb(0xea, 0xee, 0xf2);
pub const BORDER:    Color32 = Color32::from_rgb(0xd0, 0xd7, 0xde);
pub const FG:        Color32 = Color32::from_rgb(0x1f, 0x23, 0x28);
pub const MUTED:     Color32 = Color32::from_rgb(0x59, 0x63, 0x6e);
pub const ACCENT:    Color32 = Color32::from_rgb(0x8F, 0xB4, 0x6B);   // verde sage
pub const ACCENT_H:  Color32 = Color32::from_rgb(0x49, 0x62, 0x29);   // verde oscuro
pub const OK:        Color32 = Color32::from_rgb(0x12, 0x5F, 0x27);
pub const WARN:      Color32 = Color32::from_rgb(0x9a, 0x67, 0x00);
pub const ERR:       Color32 = Color32::from_rgb(0xd1, 0x24, 0x2f);
pub const INFO:      Color32 = Color32::from_rgb(0x82, 0x50, 0xdf);
pub const PILL_FG:   Color32 = Color32::from_rgb(0x3D, 0x58, 0x27);

pub fn apply(ctx: &egui::Context) {
    use egui::{style::Visuals, Stroke};
    let mut style = (*ctx.style()).clone();
    let mut v = Visuals::light();
    v.override_text_color = Some(FG);
    v.panel_fill = BG;
    v.window_fill = BG;
    v.faint_bg_color = PANEL;
    v.extreme_bg_color = PANEL2;
    v.widgets.noninteractive.bg_fill = PANEL;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, MUTED);
    v.widgets.inactive.bg_fill = PANEL2;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, FG);
    v.widgets.hovered.bg_fill = BORDER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, FG);
    v.widgets.active.bg_fill = ACCENT;
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT_H);
    v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0, ACCENT_H);
    v.hyperlink_color = ACCENT_H;
    style.visuals = v;
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    ctx.set_style(style);
}
