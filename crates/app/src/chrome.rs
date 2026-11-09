//! The app's own widgets: large toolbar buttons with an icon over a label, device chips, and the
//! cards of the home screen.

use crate::canvas::{draw_device_icon, kind_colour};
use crate::icons::{self, Icon};
use egui::{
    Align2, CornerRadius, FontId, Rect, Response, Role, Sense, Stroke, StrokeKind, Ui, Vec2,
    WidgetInfo,
};
use engine::lab::LabKind;

/// Size of a toolbar button.
pub const TOOL: Vec2 = Vec2::new(62.0, 54.0);

/// A large button: an icon above a short label. `selected` draws it pressed in.
pub fn tool_button(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    selected: bool,
    enabled: bool,
) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(TOOL, sense);
    resp.widget_info(|| WidgetInfo::selected(Role::Button, enabled, selected, label));
    if ui.is_rect_visible(rect) {
        let v = ui.visuals();
        let w = if !enabled {
            &v.widgets.noninteractive
        } else if selected {
            &v.widgets.active
        } else {
            v.widgets.style(&resp)
        };
        let fill = if selected {
            v.selection.bg_fill
        } else if resp.hovered() && enabled {
            w.weak_bg_fill
        } else {
            egui::Color32::TRANSPARENT
        };
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(6), fill);
        if selected {
            p.rect_stroke(
                rect,
                CornerRadius::same(6),
                Stroke::new(1.0, v.selection.stroke.color),
                StrokeKind::Inside,
            );
        }
        let ink = if !enabled {
            v.weak_text_color().gamma_multiply(0.6)
        } else if selected {
            v.selection.stroke.color
        } else {
            v.text_color()
        };
        let icon_rect =
            Rect::from_center_size(rect.center_top() + Vec2::new(0.0, 19.0), Vec2::splat(24.0));
        icons::paint(p, icon_rect, icon, ink);
        p.text(
            rect.center_bottom() - Vec2::new(0.0, 5.0),
            Align2::CENTER_BOTTOM,
            label,
            FontId::proportional(12.0),
            ink,
        );
    }
    resp
}

/// A device chip for the top bar: its symbol and name. Clicking it opens the device.
pub fn device_chip(ui: &mut Ui, kind: LabKind, name: &str, selected: bool) -> Response {
    let font = FontId::proportional(14.0);
    let text_w = ui
        .fonts_mut(|f| f.layout_no_wrap(name.to_string(), font.clone(), egui::Color32::WHITE))
        .size()
        .x;
    let size = Vec2::new(text_w + 44.0, 34.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, name));
    if ui.is_rect_visible(rect) {
        let v = ui.visuals();
        let fill = if selected {
            v.selection.bg_fill
        } else if resp.hovered() {
            v.widgets.hovered.weak_bg_fill
        } else {
            v.widgets.inactive.weak_bg_fill
        };
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(17), fill);
        let icon = Rect::from_center_size(
            rect.left_center() + Vec2::new(19.0, 0.0),
            Vec2::new(24.0, 18.0),
        );
        draw_device_icon(p, icon, kind, kind_colour(kind), Stroke::NONE);
        let ink = if selected {
            v.selection.stroke.color
        } else {
            v.text_color()
        };
        p.text(
            rect.left_center() + Vec2::new(36.0, 0.0),
            Align2::LEFT_CENTER,
            name,
            font,
            ink,
        );
    }
    resp
}

/// A card: a framed, clickable area. Returns what `add` returned and whether the card was
/// clicked anywhere outside a widget of its own.
pub fn card<R>(
    ui: &mut Ui,
    id: egui::Id,
    size: Vec2,
    enabled: bool,
    add: impl FnOnce(&mut Ui) -> R,
) -> (R, Response) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let resp = ui.interact(
        rect,
        id,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let v = ui.visuals().clone();
    let hot = enabled && resp.hovered();
    ui.painter().rect(
        rect,
        CornerRadius::same(10),
        if hot {
            v.widgets.hovered.weak_bg_fill
        } else {
            v.faint_bg_color
        },
        Stroke::new(
            if hot { 1.5 } else { 1.0 },
            if hot {
                v.selection.stroke.color
            } else {
                v.widgets.noninteractive.bg_stroke.color
            },
        ),
        StrokeKind::Inside,
    );
    // A child that does not move the parent's cursor: the card has already taken its space.
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id)
            .max_rect(rect.shrink(14.0))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    // Text in a card is not selectable, so a click on it is a click on the card.
    inner.style_mut().interaction.selectable_labels = false;
    if !enabled {
        inner.disable();
    }
    (add(&mut inner), resp)
}
