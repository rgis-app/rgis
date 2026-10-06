use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::egui;
use bevy_egui_window::Window as _;

#[derive(SystemParam)]
pub struct Welcome<'w> {
    show_add_layer_window_event_writer: MessageWriter<'w, rgis_ui_messages::ShowAddLayerWindowMessage>,
}

impl egui::Widget for Welcome<'_> {
    fn ui(mut self, ui: &mut egui::Ui) -> egui::Response {
        ui.vertical_centered_justified(|ui| {
            ui.heading("Welcome to rgis");
            ui.label("A geospatial data viewer written in Rust.");
            ui.add_space(8.0);
            if ui.button("Add Layer...").clicked() {
                self.show_add_layer_window_event_writer.write_default();
            }
        })
        .response
    }
}

impl bevy_egui_window::Window for Welcome<'_> {
    type Item<'w, 's> = Welcome<'w>;
    const INITIALLY_OPEN: bool = true;

    fn title(&self) -> &str {
        "Welcome"
    }

    fn default_width(&self) -> f32 {
        350.0
    }
}

pub fn render_welcome_window_system(
    window: Welcome<'_>,
    mut bevy_egui_ctx: bevy_egui::EguiContexts,
    current_state: Res<State<bevy_egui_window::WindowVisibility<Welcome<'static>>>>,
    mut next_state: ResMut<NextState<bevy_egui_window::WindowVisibility<Welcome<'static>>>>,
    mut open_windows: ResMut<crate::OpenWindows>,
) -> Result {
    if crate::widget_registry::take_close_request("Welcome") {
        next_state.set(bevy_egui_window::WindowVisibility::Closed);
        return Ok(());
    }

    let ctx = bevy_egui_ctx.ctx_mut()?;

    let mut is_open = *current_state.get() == bevy_egui_window::WindowVisibility::Open;

    let response = egui::Window::new(window.title())
        .default_width(window.default_width())
        .open(&mut is_open)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0., 0.])
        .show(ctx, |ui| {
            ui.add(window);
        });

    if let Some(response) = response {
        let clicked_outside = ctx.input(|i| {
            i.pointer.any_click()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|pos| !response.response.rect.contains(pos))
        });
        if clicked_outside {
            is_open = false;
        }
    }

    if is_open {
        open_windows.record("Welcome");
    } else {
        next_state.set(bevy_egui_window::WindowVisibility::Closed);
    }

    Ok(())
}
