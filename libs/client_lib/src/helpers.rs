use bevy::{
    math::{Mat4, Vec2, Vec3, Vec4},
    window::Window,
};
use bevy_rapier3d::rapier::{geometry::Ray, na};

// Heavily inspired by https://github.com/bevyengine/bevy/pull/432/.
pub fn cursor_pos_to_ray(
    cursor_viewport: Vec2,
    window: &Window,
    camera_transform: &Mat4,
    camera_perspective: &Mat4,
) -> Ray {
    // Calculate the cursor pos in NDC space [(-1,-1), (1,1)].
    let cursor_ndc = Vec4::from((
        (cursor_viewport.x / window.width() as f32) * 2.0 - 1.0,
        (cursor_viewport.y / window.height() as f32) * 2.0 - 1.0,
        -1.0, // let the cursor be on the far clipping plane
        1.0,
    ));

    let object_to_world = camera_transform;
    let object_to_ndc = camera_perspective;

    // Transform the cursor position into object/camera space. This also turns the cursor into
    // a vector that's pointing from the camera center onto the far plane.
    let mut ray_camera = object_to_ndc.inverse().mul_vec4(cursor_ndc);
    ray_camera.z = -1.0;
    ray_camera.w = 0.0; // treat the vector as a direction (0 = Direction, 1 = Position)

    // Transform the cursor into world space.
    let ray_world = object_to_world.mul_vec4(ray_camera);
    let ray_world = ray_world.truncate();

    let camera_pos = camera_transform.w_axis.truncate();
    let camera_pos = na::Point3::new(camera_pos.x, camera_pos.y, camera_pos.z);
    Ray::new(
        camera_pos,
        na::Vector3::from_row_slice(ray_world.normalize().as_ref()),
    )
}

pub fn intersect_ray_plane(ray: &Ray, size: f32) -> Option<Vec3> {
    let plane_normal = Vec3::new(0.0, 1.0, 0.0);
    let ray_origin = Vec3::new(ray.origin.x, ray.origin.y, ray.origin.z);
    let ray_direction = Vec3::new(ray.dir.x, ray.dir.y, ray.dir.z);

    let denominator = plane_normal.dot(ray_direction);
    if denominator.abs() > f32::EPSILON {
        let t = (-ray_origin).dot(plane_normal) / denominator;
        if t >= f32::EPSILON {
            return Some(ray_origin + t * ray_direction).and_then(|intersection_point| {
                // Checks that the intersection point is within the plane. Assumes plane's origin
                // at zero coordinates.
                if intersection_point.y.abs() <= f32::EPSILON
                    && intersection_point.x.abs() <= size / 2.0
                    && intersection_point.z.abs() <= size / 2.0
                {
                    Some(intersection_point)
                } else {
                    None
                }
            });
        }
    }
    None
}
