use bevy::prelude::*;

pub(crate) fn compute_target_for_rect(
    bounding_rect: geo::Rect<geo_projected::ProjectedScalar>,
    map_area: rgis_units::MapArea,
    current_transform: &Transform,
) -> Option<Transform> {
    let layer_center = bounding_rect.center();
    let scale = determine_scale(bounding_rect, map_area.size());
    // A single point has no size to fit, which would give a scale of zero.
    // Keep the current zoom instead.
    let camera_scale = if scale.is_normal() {
        crate::CameraScale(scale)
    } else {
        crate::CameraScale::from_transform(current_transform)
    };
    let mut camera_offset = match crate::CameraOffset::from_coord(layer_center) {
        Ok(offset) => offset,
        Err(e) => {
            error!("Error converting layer center to camera offset: {:?}", e);
            return None;
        }
    };
    camera_offset.pan_x(
        (map_area.right_offset_px - map_area.left_offset_px) / 2.,
        camera_scale,
    );
    camera_offset.pan_y(
        (map_area.top_offset_px - map_area.bottom_offset_px) / 2.,
        camera_scale,
    );
    let mut target = Transform::IDENTITY;
    target.translation = camera_offset.to_transform_translation_vec();
    target.scale = camera_scale.to_transform_scale_vec();
    Some(target)
}

pub(crate) fn center_camera_on_projected_world_rect(
    bounding_rect: geo::Rect<geo_projected::ProjectedScalar>,
    camera_transform: &mut Transform,
    map_area: rgis_units::MapArea,
) {
    if let Some(target) = compute_target_for_rect(bounding_rect, map_area, camera_transform) {
        camera_transform.translation = target.translation;
        camera_transform.scale = target.scale;
        debug!("New transform scale: {:?}", camera_transform.scale);
    }
}

pub(crate) fn set_camera_transform(
    transform: &mut Transform,
    camera_offset: crate::CameraOffset,
    camera_scale: crate::CameraScale,
) {
    transform.translation = camera_offset.to_transform_translation_vec();
    transform.scale = camera_scale.to_transform_scale_vec();
    debug!("New transform scale: {:?}", transform.scale);
}

pub(crate) fn determine_scale(
    bounding_rect: geo::Rect<geo_projected::ProjectedScalar>,
    canvas_size: rgis_units::ScreenSize,
) -> f32 {
    let width = canvas_size.width;
    let height = canvas_size.height;
    (bounding_rect.width().0 as f32 / width).max(bounding_rect.height().0 as f32 / height)
}
