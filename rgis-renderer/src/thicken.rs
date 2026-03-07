use bevy::prelude::*;
use bevy::mesh::Indices;

/// Converts a `LineList` mesh (1px hardware lines) into a `TriangleList` mesh
/// where each line segment becomes a quad.
///
/// The `width` parameter is a multiplier: a base width is computed from the
/// mesh's bounding box extent (extent / 300), and the final world-space width
/// is `base_width * width`. This means `width = 1.0` gives roughly 1-pixel
/// lines at the zoom level that fits the full geometry extent.
pub fn thicken_line_mesh(mesh: Mesh, width: f32) -> Mesh {
    let Some(positions) = mesh
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .and_then(|attr| attr.as_float3())
    else {
        return mesh;
    };

    let Some(Indices::U32(indices)) = mesh.indices() else {
        return mesh;
    };

    if positions.is_empty() || indices.is_empty() {
        return mesh;
    }

    // Compute bounding box extent to derive a scale-appropriate base width
    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;
    for pos in positions {
        min_x = min_x.min(pos[0]);
        max_x = max_x.max(pos[0]);
        min_y = min_y.min(pos[1]);
        max_y = max_y.max(pos[1]);
    }
    let extent = (max_x - min_x).max(max_y - min_y);
    let base_width = extent / 300.0;
    let half_width = (base_width * width) / 2.0;

    if half_width < f32::EPSILON {
        return mesh;
    }

    let segment_count = indices.len() / 2;
    let mut new_positions = Vec::with_capacity(segment_count * 4);
    let mut new_indices = Vec::with_capacity(segment_count * 6);

    for chunk in indices.chunks(2) {
        let a = positions[chunk[0] as usize];
        let b = positions[chunk[1] as usize];

        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let len = (dx * dx + dy * dy).sqrt();
        if len < f32::EPSILON {
            continue;
        }

        // Perpendicular offset
        let px = -dy / len * half_width;
        let py = dx / len * half_width;

        let base = new_positions.len() as u32;

        // Quad vertices
        new_positions.push([a[0] - px, a[1] - py, a[2]]);
        new_positions.push([a[0] + px, a[1] + py, a[2]]);
        new_positions.push([b[0] + px, b[1] + py, b[2]]);
        new_positions.push([b[0] - px, b[1] - py, b[2]]);

        // Two triangles
        new_indices.push(base);
        new_indices.push(base + 1);
        new_indices.push(base + 2);
        new_indices.push(base);
        new_indices.push(base + 2);
        new_indices.push(base + 3);
    }

    let mut new_mesh = Mesh::new(
        bevy::render::render_resource::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
    );
    new_mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, new_positions);
    new_mesh.insert_indices(Indices::U32(new_indices));
    new_mesh
}
