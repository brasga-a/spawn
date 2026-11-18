//! `F3D-T02` (`F3D-FR-02`, `F3D-INV-05, 07`, `SPN-INV-050, 053`): headless right-handed
//! reversed-Z perspective projection tests (finite and infinite-far), single-site Vulkan
//! NDC Y-flip check, and Vulkan 1.3 polygon-orientation signed-area verification (`a > 0`
//! -> `VK_FRONT_FACE_COUNTER_CLOCKWISE`).

use std::f32::consts::{FRAC_PI_2, FRAC_PI_3, PI};

use proptest::prelude::*;
use spawn_math::{Mat4, MathErrorKind, Quat, Transform, Vec2, Vec3};

/// Maps Vulkan NDC `[-1, 1]` to framebuffer pixel coordinates for a standard positive-height
/// `VkViewport { x: 0, y: 0, width, height, minDepth: 0.0, maxDepth: 1.0 }`.
fn ndc_to_vulkan_framebuffer(ndc: Vec3, width: f32, height: f32) -> Vec2 {
    Vec2::new(
        (ndc.x + 1.0) * (0.5 * width),
        (ndc.y + 1.0) * (0.5 * height),
    )
}

/// Computes Vulkan 1.3 §26.11.1 polygon signed area:
/// `a = -1/2 * sum_{i=0..2} (x_i * y_{i+1} - x_{i+1} * y_i)`
/// `  = -1/2 * ((x1 - x0)(y2 - y0) - (x2 - x0)(y1 - y0))`.
///
/// In Vulkan, `VK_FRONT_FACE_COUNTER_CLOCKWISE` classifies a triangle as front-facing
/// iff `a > 0.0`.
fn vulkan_polygon_signed_area(fb0: Vec2, fb1: Vec2, fb2: Vec2) -> f32 {
    let e1 = fb1 - fb0;
    let e2 = fb2 - fb0;
    -0.5 * (e1.x * e2.y - e2.x * e1.y)
}

fn proptest_cfg() -> ProptestConfig {
    ProptestConfig {
        cases: 200,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

#[test]
fn f3d_t02_finite_reversed_z_maps_near_to_one_far_to_zero_and_flips_y_once() {
    let fov_y = FRAC_PI_3;
    let aspect = 16.0 / 9.0;
    let near = 0.1_f32;
    let far = 1000.0_f32;

    let proj = Mat4::perspective_reversed_z_rh(fov_y, aspect, near, far).unwrap();

    // Eye (0, 0, -near) -> NDC (0, 0, 1.0); Eye (0, 0, -far) -> NDC (0, 0, 0.0).
    let ndc_near = proj.project_point3(Vec3::new(0.0, 0.0, -near)).unwrap();
    let ndc_far = proj.project_point3(Vec3::new(0.0, 0.0, -far)).unwrap();
    assert!(
        ndc_near.abs_diff_eq(Vec3::new(0.0, 0.0, 1.0), 1.0e-6),
        "ndc_near = {ndc_near:?}"
    );
    assert!(
        ndc_far.abs_diff_eq(Vec3::new(0.0, 0.0, 0.0), 1.0e-6),
        "ndc_far = {ndc_far:?}"
    );

    // Strict depth monotonicity across [near, far]: closer objects have LARGER depth (`GREATER_OR_EQUAL`).
    let depths = [
        0.1_f32, 0.25, 1.0, 2.0, 5.0, 25.0, 100.0, 500.0, 999.9, 1000.0,
    ];
    for pair in depths.windows(2) {
        let z1 = proj
            .project_point3(Vec3::new(0.0, 0.0, -pair[0]))
            .unwrap()
            .z;
        let z2 = proj
            .project_point3(Vec3::new(0.0, 0.0, -pair[1]))
            .unwrap()
            .z;
        assert!(
            z1 > z2,
            "reversed-Z violated for d1={} (z={z1}) < d2={} (z={z2})",
            pair[0],
            pair[1]
        );
        assert!((0.0..=1.0 + 1.0e-6).contains(&z1));
    }

    // Single-site Vulkan NDC Y-flip (`SPN-INV-053`):
    // - View/model transforms do NOT flip +Y:
    let view = Mat4::try_look_to_rh(Vec3::ZERO, Vec3::FORWARD, Vec3::UP).unwrap();
    assert_eq!(view, Mat4::IDENTITY);
    assert_eq!(Transform::IDENTITY.up(), Vec3::Y);
    assert_eq!(Quat::IDENTITY.up(), Vec3::Y);
    // - Projection maps +X -> +X_ndc (right) and +Y -> -Y_ndc (top of Vulkan framebuffer):
    let right_pt = proj.project_point3(Vec3::new(1.0, 0.0, -5.0)).unwrap();
    let up_pt = proj.project_point3(Vec3::new(0.0, 1.0, -5.0)).unwrap();
    assert!(right_pt.x > 0.0 && right_pt.y == 0.0);
    assert!(up_pt.y < 0.0 && up_pt.x == 0.0);

    // Cross-check against glam's RH Vulkan [0, 1] Y-down projection with swapped near/far.
    let glam_rev_z = glam::camera::rh::proj::vulkan::perspective(fov_y, aspect, far, near);
    let expected = Mat4::from_cols_array(&glam_rev_z.to_cols_array());
    assert!(proj.abs_diff_eq(&expected, 1.0e-6));
}

#[test]
fn f3d_t02_infinite_reversed_z_maps_near_to_one_and_approaches_zero_monotonically() {
    let fov_y = FRAC_PI_2;
    let aspect = 1920.0 / 1080.0;
    let near = 0.05_f32;

    let proj = Mat4::perspective_infinite_reversed_z_rh(fov_y, aspect, near).unwrap();
    let ndc_near = proj.project_point3(Vec3::new(0.0, 0.0, -near)).unwrap();
    assert!(ndc_near.abs_diff_eq(Vec3::new(0.0, 0.0, 1.0), 1.0e-6));

    // Exact analytic depth `z_ndc = near / d` across 7 orders of magnitude!
    let distances = [
        0.05_f32, 0.5, 5.0, 50.0, 500.0, 5_000.0, 50_000.0, 500_000.0,
    ];
    for pair in distances.windows(2) {
        let d1 = pair[0];
        let d2 = pair[1];
        let z1 = proj.project_point3(Vec3::new(0.0, 0.0, -d1)).unwrap().z;
        let z2 = proj.project_point3(Vec3::new(0.0, 0.0, -d2)).unwrap().z;
        assert!(z1 > z2 && z2 > 0.0, "d1={d1} -> {z1}, d2={d2} -> {z2}");
        assert!((z1 - near / d1).abs() <= 1.0e-6);
    }

    // Cross-check directly against glam's dedicated Vulkan RH infinite reversed-Z projection!
    let glam_inf =
        glam::camera::rh::proj::vulkan::perspective_infinite_reverse(fov_y, aspect, near);
    let expected = Mat4::from_cols_array(&glam_inf.to_cols_array());
    assert!(proj.abs_diff_eq(&expected, 1.0e-6));
}

#[test]
fn f3d_t02_ccw_front_triangle_yields_positive_vulkan_signed_area() {
    let near = 0.1_f32;
    let proj_fin = Mat4::perspective_reversed_z_rh(FRAC_PI_3, 16.0 / 9.0, near, 1000.0).unwrap();
    let proj_inf = Mat4::perspective_infinite_reversed_z_rh(FRAC_PI_3, 16.0 / 9.0, near).unwrap();

    // Right-handed CCW triangle facing the camera at the origin (`(v1 - v0) × (v2 - v0) == +Z`).
    let v0 = Vec3::new(0.0, 0.0, -near);
    let v1 = Vec3::new(1.0, 0.0, -near);
    let v2 = Vec3::new(0.0, 1.0, -near);
    assert_eq!((v1 - v0).cross(v2 - v0), Vec3::Z);

    for proj in [proj_fin, proj_inf] {
        let fb0 = ndc_to_vulkan_framebuffer(proj.project_point3(v0).unwrap(), 1920.0, 1080.0);
        let fb1 = ndc_to_vulkan_framebuffer(proj.project_point3(v1).unwrap(), 1920.0, 1080.0);
        let fb2 = ndc_to_vulkan_framebuffer(proj.project_point3(v2).unwrap(), 1920.0, 1080.0);

        let area_ccw = vulkan_polygon_signed_area(fb0, fb1, fb2);
        assert!(
            area_ccw > 0.0,
            "CCW front triangle must have positive Vulkan signed area (`VK_FRONT_FACE_COUNTER_CLOCKWISE`), got {area_ccw}"
        );

        // Reversing winding to CW (back-facing) negates signed area (`a < 0`, culled by `VK_CULL_MODE_BACK_BIT`).
        let area_cw = vulkan_polygon_signed_area(fb0, fb2, fb1);
        assert!(area_cw < 0.0 && (area_ccw + area_cw).abs() <= 1.0e-3);
    }
}

#[test]
fn f3d_t02_invalid_projection_parameters_return_coded_math_errors() {
    for bad_fov in [0.0_f32, -1.0, PI, PI + 0.1, f32::NAN, f32::INFINITY] {
        let err = Mat4::perspective_reversed_z_rh(bad_fov, 1.5, 0.1, 100.0).unwrap_err();
        assert_eq!(err.kind(), MathErrorKind::InvalidFov);
        assert_eq!(err.code(), "MATH-0001");
    }
    for bad_aspect in [0.0_f32, -16.0 / 9.0, f32::NAN, f32::INFINITY] {
        let err = Mat4::perspective_infinite_reversed_z_rh(1.0, bad_aspect, 0.1).unwrap_err();
        assert_eq!(err.kind(), MathErrorKind::InvalidAspectRatio);
        assert_eq!(err.code(), "MATH-0002");
    }
    for (near, far) in [
        (0.0_f32, 100.0_f32),
        (-0.1, 100.0),
        (10.0, 10.0),
        (10.0, 5.0),
        (0.1, f32::INFINITY),
        (f32::NAN, 100.0),
    ] {
        let err = Mat4::perspective_reversed_z_rh(1.0, 1.5, near, far).unwrap_err();
        assert_eq!(err.kind(), MathErrorKind::InvalidClipPlanes);
        assert_eq!(err.code(), "MATH-0003");
    }
    for bad_near in [0.0_f32, -0.01, f32::NAN, f32::INFINITY] {
        let err = Mat4::perspective_infinite_reversed_z_rh(1.0, 1.5, bad_near).unwrap_err();
        assert_eq!(err.kind(), MathErrorKind::InvalidClipPlanes);
        assert_eq!(err.code(), "MATH-0003");
    }
}

proptest! {
    #![proptest_config(proptest_cfg())]

    #[test]
    fn f3d_t02_prop_reversed_z_monotonicity_and_ccw_winding(
        fov_y in 0.2_f32..2.6,
        aspect in 0.25_f32..4.0,
        near in 0.01_f32..2.0,
        span in 5.0_f32..5000.0,
        u1 in 0.0_f32..0.45,
        u2 in 0.55_f32..1.0,
    ) {
        let far = near + span;
        let d1 = near + span * u1;
        let d2 = near + span * u2;

        let fin = Mat4::perspective_reversed_z_rh(fov_y, aspect, near, far).unwrap();
        let inf = Mat4::perspective_infinite_reversed_z_rh(fov_y, aspect, near).unwrap();

        let z1_fin = fin.project_point3(Vec3::new(0.0, 0.0, -d1)).unwrap().z;
        let z2_fin = fin.project_point3(Vec3::new(0.0, 0.0, -d2)).unwrap().z;
        prop_assert!(z1_fin > z2_fin);

        let z1_inf = inf.project_point3(Vec3::new(0.0, 0.0, -d1)).unwrap().z;
        let z2_inf = inf.project_point3(Vec3::new(0.0, 0.0, -d2)).unwrap().z;
        prop_assert!(z1_inf > z2_inf && z2_inf > 0.0);

        // CCW triangle at -d1 keeps positive Vulkan signed area across all valid FOV/aspect!
        let v0 = Vec3::new(-0.2, -0.1, -d1);
        let v1 = Vec3::new(0.4, -0.1, -d1);
        let v2 = Vec3::new(0.1, 0.5, -d1);
        let fb0 = ndc_to_vulkan_framebuffer(fin.project_point3(v0).unwrap(), 1280.0, 720.0);
        let fb1 = ndc_to_vulkan_framebuffer(fin.project_point3(v1).unwrap(), 1280.0, 720.0);
        let fb2 = ndc_to_vulkan_framebuffer(fin.project_point3(v2).unwrap(), 1280.0, 720.0);
        prop_assert!(vulkan_polygon_signed_area(fb0, fb1, fb2) > 0.0);
    }
}
