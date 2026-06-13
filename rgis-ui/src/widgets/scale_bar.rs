use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

pub fn render_map_scale(
    query: Query<
        &bevy::transform::components::Transform,
        bevy::ecs::query::With<bevy::camera::Camera>,
    >,
    mut bevy_egui_ctx: EguiContexts,
    rgis_settings: Res<rgis_settings::RgisSettings>,
) {
    if !rgis_settings.show_scale {
        return;
    }

    let Ok(transform) = query.single() else { return; };
    let scale = transform.scale.x;
    let bar_max_width = 100.;
    let bar_in_meter = desired_bar_length(scale * bar_max_width);
    let bar_width = bar_in_meter / scale;
    let bar_text = distance_to_readable_string(bar_in_meter);

    let Ok(egui_ctx) = bevy_egui_ctx.ctx_mut() else { return; };

    egui::Window::new("Scale")
        .frame(egui::Frame::NONE)
        .open(&mut true)
        .title_bar(false)
        .anchor(egui::Align2::LEFT_BOTTOM, [4., -2.])
        .fixed_size(egui::Vec2 {
            x: bar_max_width,
            y: 0.,
        })
        .show(egui_ctx, |ui| {
            let bar_color = if ui.visuals().dark_mode {
                egui::Color32::WHITE
            } else {
                egui::Color32::BLACK
            };
            let stroke = egui::Stroke::new(2.0, bar_color);
            let tick_height = 8.0;
            let bar_y_offset = tick_height / 2.0;

            let (response, painter) =
                ui.allocate_painter(egui::Vec2::new(bar_max_width, tick_height + 14.0), egui::Sense::hover());
            crate::dev_ui::track("scale-bar", &response, "Map scale bar");
            let origin = response.rect.left_top();

            // Left tick
            painter.line_segment(
                [
                    origin + egui::vec2(0.0, 0.0),
                    origin + egui::vec2(0.0, tick_height),
                ],
                stroke,
            );
            // Horizontal bar
            painter.line_segment(
                [
                    origin + egui::vec2(0.0, bar_y_offset),
                    origin + egui::vec2(bar_width, bar_y_offset),
                ],
                stroke,
            );
            // Right tick
            painter.line_segment(
                [
                    origin + egui::vec2(bar_width, 0.0),
                    origin + egui::vec2(bar_width, tick_height),
                ],
                stroke,
            );

            // Label below the bar
            painter.text(
                origin + egui::vec2(bar_width / 2.0, tick_height + 2.0),
                egui::Align2::CENTER_TOP,
                &bar_text,
                egui::FontId::proportional(11.0),
                bar_color,
            );
        });
}

fn desired_bar_length(full_bar_length: f32) -> f32 {
    let mut desired_length = 2e38;
    for i in full_bar_length.log10().floor() as i32..=f32::MAX_10_EXP {
        for j in [1., 2., 5.] {
            let length = 10_f32.powi(i) * j;
            if full_bar_length < length {
                break;
            }
            desired_length = length;
        }
    }
    desired_length
}

pub fn distance_to_readable_string(distance: f32) -> String {
    let res = get_proper_distance_unit(distance);
    format!("{}{}m", (res.1 * distance).round(), res.0)
}

fn get_proper_distance_unit(distance: f32) -> (&'static str, f32) {
    match distance.log10().floor() as i32 {
        i32::MIN..=-16 => ("a", 1e18),
        -15..=-13 => ("f", 1e15),
        -12..=-10 => ("p", 1e12),
        -9..=-7 => ("n", 1e9),
        -6..=-4 => ("µ", 1e6),
        -3..=-3 => ("m", 1e3),
        -2..=-1 => ("c", 1e2),
        0..=2 => ("", 1e0),
        3..=8 => ("k", 1e-3), //human readable
        // 6..=8 => ("M", 1e-6),
        9..=11 => ("G", 1e-9),
        12..=14 => ("T", 1e-12),
        15..=17 => ("P", 1e-15),
        18..=20 => ("E", 1e-18),
        21..=23 => ("Z", 1e-21),
        24..=26 => ("Y", 1e-24),
        27..=i32::MAX => ("Y", 1e-24),
    }
}
