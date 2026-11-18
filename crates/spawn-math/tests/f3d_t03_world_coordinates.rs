//! `F3D-T03` (`F3D-FR-01, 10`, `F3D-INV-06`, `SPN-INV-051, 052`): headless `f64`
//! world-position type separation and `±50_000.0 m` (`±50 km`) camera-relative `f32`
//! precision verification.
//!
//! Compile-time rejection of `WorldPos + Vec3`, `WorldPos + WorldPos`, and `Vec3::from(world_pos)`
//! is additionally enforced by `compile_fail` doctests on [`spawn_math::WorldPos`].

use std::any::TypeId;

use proptest::prelude::*;
use spawn_math::{DVec3, Mat4, Quat, Vec3, WorldPos, camera_relative_f32};

#[test]
fn f3d_t03_distinct_types_and_affine_world_algebra() {
    assert_ne!(TypeId::of::<WorldPos>(), TypeId::of::<DVec3>());
    assert_ne!(TypeId::of::<WorldPos>(), TypeId::of::<Vec3>());
    assert_ne!(TypeId::of::<DVec3>(), TypeId::of::<Vec3>());
    assert_eq!(std::mem::size_of::<WorldPos>(), 24);
    assert_eq!(std::mem::size_of::<DVec3>(), 24);
    assert_eq!(std::mem::size_of::<Vec3>(), 12);

    let mut pos = WorldPos::new(10_000.0, -250.5, -42_000.25);
    let step = DVec3::new(1.5, 0.5, -2.75);
    pos += step;
    assert_eq!(pos, WorldPos::new(10_001.5, -250.0, -42_003.0));
    pos -= step;
    assert_eq!(pos, WorldPos::new(10_000.0, -250.5, -42_000.25));

    let other = pos.translate_local_f32(Vec3::new(-2.0, 4.0, 1.0));
    let delta: DVec3 = other - pos;
    assert_eq!(delta, DVec3::new(-2.0, 4.0, 1.0));
    assert!((other.distance(pos) - (21.0_f64).sqrt()).abs() <= 1.0e-12);
    assert_eq!(WorldPos::from_le_bytes(pos.to_le_bytes()), pos);
}

#[test]
fn f3d_t03_fifty_kilometer_camera_relative_f32_retains_sub_ten_micrometer_precision() {
    // Test across all signs of ±50,000.0 m (50 km) from the world origin.
    for (cx, cy, cz) in [
        (50_000.0, 50_000.0, -50_000.0),
        (-50_000.0, 50_000.0, 50_000.0),
        (50_000.0, -50_000.0, 50_000.0),
        (-50_000.0, -50_000.0, -50_000.0),
    ] {
        let camera_world = WorldPos::new(cx, cy, cz);

        // 1.0 m offset along all axes + fine sub-centimeter fractional offset (1.001_234 m).
        let exact_offset = DVec3::new(1.0, -1.0, 1.001_234);
        let mesh_world = camera_world + exact_offset;

        let rel_f32 = camera_relative_f32(mesh_world, camera_world);
        assert_eq!(rel_f32, mesh_world.camera_relative_to(camera_world));

        let err_x = (rel_f32.x as f64 - exact_offset.x).abs();
        let err_y = (rel_f32.y as f64 - exact_offset.y).abs();
        let err_z = (rel_f32.z as f64 - exact_offset.z).abs();
        assert!(err_x < 1.0e-5, "err_x at 50 km = {err_x}");
        assert!(err_y < 1.0e-5, "err_y at 50 km = {err_y}");
        assert!(err_z < 1.0e-5, "err_z at 50 km = {err_z}");

        // Contrast with naive f32 world-space subtraction: at 50,000.0_f32, 1 ULP is
        // 2^-8 = 0.003_906_25 m (~3.9 mm), destroying millimeter offsets!
        let naive_f32_z = (mesh_world.z as f32) - (camera_world.z as f32);
        let naive_err_z = (naive_f32_z as f64 - exact_offset.z).abs();
        assert!(
            naive_err_z > 1.0e-3 && err_z < 1.0e-7,
            "expected f64 camera-relative err ({err_z}) << naive f32 err ({naive_err_z})"
        );

        // Camera-relative model matrix places the mesh identically to a scene at the origin!
        let rot = Quat::from_yaw_pitch(0.35, -0.2);
        let scale = Vec3::splat(1.0);
        let model_at_50km = Mat4::from_scale_rotation_translation(scale, rot, rel_f32);
        let model_at_origin = Mat4::from_scale_rotation_translation(
            scale,
            rot,
            camera_relative_f32(WorldPos::ORIGIN + exact_offset, WorldPos::ORIGIN),
        );
        assert!(model_at_50km.abs_diff_eq(&model_at_origin, 1.0e-6));
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn f3d_t03_prop_large_world_camera_relative_and_alpha_lerp_precision(
        cam_x in -100_000.0_f64..100_000.0,
        cam_y in -100_000.0_f64..100_000.0,
        cam_z in -100_000.0_f64..100_000.0,
        off_x in -10.0_f64..10.0,
        off_y in -10.0_f64..10.0,
        off_z in -10.0_f64..10.0,
        alpha in 0.0_f64..1.0,
    ) {
        let prev_cam = WorldPos::new(cam_x, cam_y, cam_z);
        let step = DVec3::new(0.25, -0.1, -0.5);
        let curr_cam = prev_cam + step;
        let interp_cam = prev_cam.lerp(curr_cam, alpha);

        let offset = DVec3::new(off_x, off_y, off_z);
        let mesh = interp_cam + offset;
        let rel = camera_relative_f32(mesh, interp_cam);

        let max_err = (rel.x as f64 - offset.x)
            .abs()
            .max((rel.y as f64 - offset.y).abs())
            .max((rel.z as f64 - offset.z).abs());
        // Within [-10 m, 10 m] of the camera anywhere inside ±100 km, f32 ULP is <= 2^-20 < 1e-6 m!
        prop_assert!(max_err < 1.0e-5, "max_err = {max_err} at cam = {interp_cam:?}, offset = {offset:?}");
    }
}
