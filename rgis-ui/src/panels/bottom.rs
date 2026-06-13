use bevy_egui::egui;

pub struct Bottom<'a> {
    pub egui_ctx: &'a egui::Context,
    pub mouse_pos: &'a rgis_mouse::MousePos,
    pub target_crs: &'a rgis_crs::TargetCrs,
    pub cached_latlng: Option<(f64, f64)>,
    pub change_crs_window_visible: &'a mut crate::ChangeCrsWindowVisible,
    pub bottom_panel_height: &'a mut rgis_units::BottomPanelHeight,
}

/// Convert the current mouse position from the target CRS to WGS 84 lat/lng.
///
/// This is extracted as a free function so the caller can cache the result and
/// only recompute when the mouse position or target CRS actually changes,
/// avoiding a per-frame coordinate transformation and geodesy lock acquisition.
pub fn projected_to_latlng(
    mouse_pos: &rgis_mouse::MousePos,
    target_crs: &rgis_crs::TargetCrs,
    geodesy_ctx: &rgis_crs::GeodesyContext,
    wgs84_op_handle: &rgis_crs::Wgs84OpHandle,
) -> Option<(f64, f64)> {
    let geodesy_ctx_inner = geodesy_ctx.read().ok()?;

    let transformer = geo_geodesy::Transformer::from_geodesy(
        &*geodesy_ctx_inner,
        target_crs.0.op_handle,
        wgs84_op_handle.0,
        true, // WGS84 is geographic
    )
    .ok()?;

    let x = mouse_pos.0.x.0;
    let y = mouse_pos.0.y.0;
    let mut point = geo::Geometry::Point(geo::Point::new(x, y));
    transformer.transform(&mut point).ok()?;

    if let geo::Geometry::Point(p) = point {
        let lng = p.x();
        let lat = p.y();
        if lat.is_finite() && lng.is_finite() {
            return Some((lat, lng));
        }
    }
    None
}

impl Bottom<'_> {
    pub fn render(&mut self) {
        let inner_response = egui::TopBottomPanel::bottom("bottom").show(self.egui_ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                crate::dev_ui::label(
                    ui,
                    "app-version",
                    format!("rgis v{}", env!("CARGO_PKG_VERSION")),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.render_crs(ui);
                    ui.separator();
                    self.render_mouse_position(ui);
                });
            });
        });
        self.bottom_panel_height.0 = inner_response.response.rect.height();
    }

    fn render_crs(&mut self, ui: &mut egui::Ui) {
        let edit_btn = crate::dev_ui::button(ui, "edit-crs", "Edit");
        crate::widget_registry::register("Edit CRS", edit_btn.rect);
        if edit_btn.clicked() {
            self.change_crs_window_visible.0 = true;
        }

        match self.target_crs.0.epsg_code {
            Some(code) => crate::dev_ui::label(ui, "crs-readout", format!("CRS: EPSG:{code}")),
            None => crate::dev_ui::label(ui, "crs-readout", "CRS: Custom PROJ"),
        };
    }

    fn render_mouse_position(&mut self, ui: &mut egui::Ui) {
        if let Some((lat, lng)) = self.cached_latlng {
            crate::dev_ui::label(ui, "coords-readout", format!("Lat: {lat:.6}  Lng: {lng:.6}"));
        } else {
            let x = self.mouse_pos.0.x.0;
            let y = self.mouse_pos.0.y.0;
            crate::dev_ui::label(ui, "coords-readout", format!("X: {x:.2}  Y: {y:.2}"));
        }
    }
}
