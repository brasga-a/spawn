//! `F3D-T05` (`docs/first-3d/VERIFICATION.md` line 15; `F3D-FR-03, 09, 10`,
//! `F3D-INV-08, 09, 10`, `SPN-INV-040`..`SPN-INV-044`, `SPN-INV-050`..`SPN-INV-054`):
//! headless 60 Hz `f64` [`FreeCamera`] simulation and read-only `alpha`
//! presentation interpolation tests, split explicitly into:
//! - **`F3D-T05a`**: non-dropping cadence equivalence (`tick_duration / 4`,
//!   `tick_duration`, `4 * tick_duration`), right-handed look signs, `±(π/2 - ε)`
//!   pitch clamping, `±π` shortest-arc quaternion wrap, `±50 km` precision,
//!   focus-loss input reset, and immutable simulation state on presentation.
//! - **`F3D-T05b`**: backlog-drop clamping under stall deltas `> 4 * tick_duration`
//!   (e.g. `250 ms`).

#![forbid(unsafe_code)]

use std::time::Duration;

use proptest::prelude::*;
use spawn_core::{FixedStepClock, FixedStepConfig, Tick};
use spawn_engine::{
    CameraInputBindings, CameraTickInput, EngineErrorKind, FreeCamera, FreeCameraConfig,
    FreeCameraPose, MAX_CAMERA_MOVE_SPEED_MPS, MAX_LOOK_RADIANS_PER_COUNT, interpolation_alpha,
};
use spawn_math::{DVec3, Quat, Vec3, WorldPos, det};
use spawn_platform::{ElementState, KeyCode, KeyEvent, WindowEvent};
use spawn_render::MeshHandle;

/// Synthesizes a deterministic, rich per-tick input schedule across `0..48`
/// exercising single-axis & diagonal 3D movement, right-handed yaw/pitch look,
/// `±(π/2 - ε)` pole clamping, `±π` yaw wrapping, and unlocked-cursor gating.
fn build_scripted_tick_input(tick_index: u64) -> CameraTickInput {
    match tick_index {
        // Single-axis local translation along -Z, +X, +Y, +Z, -X, -Y.
        0 | 1 => CameraTickInput {
            move_forward_back: 1.0,
            ..CameraTickInput::IDLE
        },
        2 => CameraTickInput {
            move_right_left: 1.0,
            ..CameraTickInput::IDLE
        },
        3 => CameraTickInput {
            move_up_down: 1.0,
            ..CameraTickInput::IDLE
        },
        4 => CameraTickInput {
            move_forward_back: -1.0,
            ..CameraTickInput::IDLE
        },
        5 => CameraTickInput {
            move_right_left: -1.0,
            ..CameraTickInput::IDLE
        },
        6 | 7 => CameraTickInput {
            move_up_down: -1.0,
            ..CameraTickInput::IDLE
        },
        // 2-axis and 3-axis diagonal translation (must normalize to unit speed).
        8..=11 => CameraTickInput {
            move_forward_back: 1.0,
            move_right_left: 1.0,
            ..CameraTickInput::IDLE
        },
        12..=15 => CameraTickInput {
            move_forward_back: 1.0,
            move_right_left: -1.0,
            move_up_down: 1.0,
            ..CameraTickInput::IDLE
        },
        // Combined relative pointer look + movement while locked.
        16..=27 => {
            let phase = (tick_index - 16) as f64;
            CameraTickInput {
                move_forward_back: 1.0,
                move_right_left: if tick_index.is_multiple_of(2) {
                    1.0
                } else {
                    -1.0
                },
                move_up_down: if tick_index.is_multiple_of(3) {
                    1.0
                } else {
                    0.0
                },
                look_yaw_delta: 18.5 - phase * 3.25,
                look_pitch_delta: -12.0 + phase * 2.5,
                look_enabled: true,
            }
        }
        // Aggressive pitch sweep past both +pi/2 and -pi/2 poles.
        28..=31 => CameraTickInput {
            move_forward_back: 1.0,
            look_pitch_delta: -600.0, // pointer up -> pitches up against +pitch_limit
            look_enabled: true,
            ..CameraTickInput::IDLE
        },
        32..=35 => CameraTickInput {
            move_forward_back: -1.0,
            look_pitch_delta: 1_200.0, // pointer down -> pitches down against -pitch_limit
            look_enabled: true,
            ..CameraTickInput::IDLE
        },
        // Yaw sweep wrapping cleanly across +-pi.
        36..=43 => CameraTickInput {
            move_right_left: 1.0,
            look_yaw_delta: 420.0,
            look_pitch_delta: -150.0,
            look_enabled: true,
            ..CameraTickInput::IDLE
        },
        // Unlocked cursor (`look_enabled = false`): raw pointer counts must NOT rotate camera.
        _ => CameraTickInput {
            move_forward_back: 1.0,
            look_yaw_delta: 999.0,
            look_pitch_delta: -999.0,
            look_enabled: false,
            ..CameraTickInput::IDLE
        },
    }
}

/// Runs `FreeCamera` + `FixedStepClock` over `frame_deltas`, sampling
/// presentation on every frame and recording `(prev_pose, curr_pose)` per executed `Tick`.
fn run_cadence_trace(
    config: FixedStepConfig,
    cam_config: FreeCameraConfig,
    frame_deltas: &[Duration],
    input_for_tick: impl Fn(u64) -> CameraTickInput,
) -> (Vec<(Tick, FreeCameraPose, FreeCameraPose)>, Vec<f64>) {
    let mut clock = FixedStepClock::new(config);
    let cam_res = FreeCamera::new(cam_config);
    assert!(cam_res.is_ok(), "valid camera config failed: {cam_res:?}");
    let Ok(mut camera) = cam_res else {
        unreachable!();
    };
    let tick_duration = clock.config().tick_duration();

    let mut tick_poses = Vec::new();
    let mut frame_alphas = Vec::with_capacity(frame_deltas.len());

    for &delta in frame_deltas {
        let upd_res = clock.advance(delta);
        assert!(upd_res.is_ok(), "clock.advance failed: {upd_res:?}");
        let Ok(update) = upd_res else {
            unreachable!();
        };
        assert_eq!(
            update.dropped(),
            Duration::ZERO,
            "non-dropping cadence must never drop simulation backlog"
        );

        for tick in update.ticks() {
            let input = input_for_tick(tick.get());
            camera.step_tick(tick, tick_duration, input);
            tick_poses.push((tick, *camera.prev_pose(), *camera.curr_pose()));
        }

        let before_prev = *camera.prev_pose();
        let before_curr = *camera.curr_pose();
        let before_last_tick = camera.last_tick();

        let alpha = interpolation_alpha(&clock);
        assert!(
            alpha.is_finite() && (0.0..1.0).contains(&alpha),
            "alpha must lie in [0.0, 1.0), got {alpha}"
        );
        let pres_res = camera.sample_presentation(alpha, 16.0 / 9.0);
        assert!(pres_res.is_ok(), "sample_presentation failed: {pres_res:?}");
        let Ok(presentation) = pres_res else {
            unreachable!();
        };
        assert!(presentation.world_pos.is_finite());
        assert!(presentation.orientation.is_normalized());
        assert!(presentation.view.projection.is_finite());

        // SPN-INV-043 / F3D-INV-10: presentation extraction never mutates simulation state.
        assert!(camera.prev_pose().bitwise_eq(&before_prev));
        assert!(camera.curr_pose().bitwise_eq(&before_curr));
        assert_eq!(camera.last_tick(), before_last_tick);

        frame_alphas.push(alpha);
    }

    (tick_poses, frame_alphas)
}

#[test]
fn f3d_t05a_non_dropping_cadence_equivalence_and_subtick_alpha_progression() {
    let step_cfg = FixedStepConfig::default(); // 60 Hz = 16_666_666 ns, max_ticks_per_update = 4
    let tick_dt = step_cfg.tick_duration();
    assert_eq!(tick_dt, Duration::from_nanos(16_666_666));
    assert_eq!(step_cfg.max_ticks_per_update(), 4);

    let cam_cfg = FreeCameraConfig::default();
    let total_ticks: usize = 48; // multiple of 4 (`12 * 4 = 48`)

    // 1. Exact 4-part sub-tick cadence (~240 Hz): `3 * 4_166_666 + 4_166_668 == 16_666_666 ns`
    //    so total elapsed nanoseconds equal `48 * tick_duration` identically!
    let q_base = tick_dt.as_nanos() as u64 / 4; // 4_166_666 ns
    let q_rem = tick_dt.as_nanos() as u64 - 3 * q_base; // 4_166_668 ns
    let mut deltas_240_exact = Vec::with_capacity(total_ticks * 4);
    for _ in 0..total_ticks {
        deltas_240_exact.push(Duration::from_nanos(q_base));
        deltas_240_exact.push(Duration::from_nanos(q_base));
        deltas_240_exact.push(Duration::from_nanos(q_base));
        deltas_240_exact.push(Duration::from_nanos(q_rem));
    }

    // 2. Uniform `tick_duration / 4` (`4_166_666 ns`) sub-tick cadence across 193 frames
    //    (`193 * 4_166_666 = 804_166_538 ns`, executing all 48 ticks `0..48` + sub-tick remainder).
    let deltas_240_uniform = vec![tick_dt / 4; total_ticks * 4 + 1];

    // 3. Single-tick 60 Hz cadence (`tick_duration` = 16_666_666 ns).
    let deltas_60 = vec![tick_dt; total_ticks];

    // 4. Multi-tick ~15 Hz catch-up cadence (`4 * tick_duration` = 66_666_664 ns, `dropped == 0`).
    let deltas_15 = vec![tick_dt * 4; total_ticks / 4];

    assert_eq!(
        deltas_240_exact.iter().copied().sum::<Duration>(),
        deltas_60.iter().copied().sum::<Duration>()
    );
    assert_eq!(
        deltas_60.iter().copied().sum::<Duration>(),
        deltas_15.iter().copied().sum::<Duration>()
    );

    let (poses_240_exact, alphas_240) = run_cadence_trace(
        step_cfg,
        cam_cfg,
        &deltas_240_exact,
        build_scripted_tick_input,
    );
    let (poses_240_uniform, _) = run_cadence_trace(
        step_cfg,
        cam_cfg,
        &deltas_240_uniform,
        build_scripted_tick_input,
    );
    let (poses_60, alphas_60) =
        run_cadence_trace(step_cfg, cam_cfg, &deltas_60, build_scripted_tick_input);
    let (poses_15, alphas_15) =
        run_cadence_trace(step_cfg, cam_cfg, &deltas_15, build_scripted_tick_input);

    assert_eq!(poses_240_exact.len(), total_ticks);
    assert_eq!(poses_240_uniform.len(), total_ticks);
    assert_eq!(poses_60.len(), total_ticks);
    assert_eq!(poses_15.len(), total_ticks);

    // Every executed Tick (0..48) must have BITWISE-IDENTICAL (`to_bits()`) `prev_pose`
    // and `curr_pose` across all four non-dropping cadences!
    for idx in 0..total_ticks {
        let expected_tick = Tick::new(idx as u64);
        let (t_a, prev_a, curr_a) = poses_240_exact[idx];
        let (t_u, prev_u, curr_u) = poses_240_uniform[idx];
        let (t_b, prev_b, curr_b) = poses_60[idx];
        let (t_c, prev_c, curr_c) = poses_15[idx];

        assert_eq!(t_a, expected_tick);
        assert_eq!(t_u, expected_tick);
        assert_eq!(t_b, expected_tick);
        assert_eq!(t_c, expected_tick);

        assert!(
            prev_a.bitwise_eq(&prev_b) && prev_b.bitwise_eq(&prev_c) && prev_c.bitwise_eq(&prev_u),
            "prev_pose bitwise mismatch at tick {idx}:\n  240={prev_a:?}\n   60={prev_b:?}\n   15={prev_c:?}"
        );
        assert!(
            curr_a.bitwise_eq(&curr_b) && curr_b.bitwise_eq(&curr_c) && curr_c.bitwise_eq(&curr_u),
            "curr_pose bitwise mismatch at tick {idx}:\n  240={curr_a:?}\n   60={curr_b:?}\n   15={curr_c:?}"
        );
    }

    // Verify 0-tick sub-frame `alpha` progression in the ~240 Hz run:
    // within each 4-frame group, frames 0, 1, 2 run 0 ticks with strictly increasing `alpha`,
    // and frame 3 completes the tick (`alpha == 0.0`).
    for chunk in alphas_240.as_chunks::<4>().0 {
        assert!(chunk[0] > 0.24 && chunk[0] < 0.26, "chunk={chunk:?}");
        assert!(chunk[1] > 0.49 && chunk[1] < 0.51, "chunk={chunk:?}");
        assert!(chunk[2] > 0.74 && chunk[2] < 0.76, "chunk={chunk:?}");
        assert_eq!(chunk[3], 0.0);
    }
    assert!(alphas_60.iter().all(|&a| a == 0.0));
    assert!(alphas_15.iter().all(|&a| a == 0.0));

    // Verify diagonal normalization: on Tick 8 (`W + D`) and Tick 12 (`W + D + Space`),
    // world-space distance traveled equals `move_speed_mps * dt_seconds` (never sqrt(2) or sqrt(3)).
    let dt_sec = (tick_dt.as_secs() as f64) + (f64::from(tick_dt.subsec_nanos()) / 1_000_000_000.0);
    let expected_step_dist = cam_cfg.move_speed_mps * dt_sec;
    for diag_tick in [0_usize, 8, 12] {
        let (_, prev, curr) = poses_60[diag_tick];
        let step_dist = curr.world_pos.distance(prev.world_pos);
        assert!(
            (step_dist - expected_step_dist).abs() < 1.0e-12,
            "tick {diag_tick}: expected step distance {expected_step_dist}, got {step_dist}"
        );
    }

    // Verify pitch clamping on Ticks 31 (+limit) and 35 (-limit) and unlocked look gating on Ticks 44..48.
    assert_eq!(poses_60[31].2.pitch_radians, cam_cfg.pitch_limit_radians);
    assert_eq!(poses_60[35].2.pitch_radians, -cam_cfg.pitch_limit_radians);
    for &(_, prev, curr) in &poses_60[44..48] {
        assert_eq!(curr.yaw_radians.to_bits(), prev.yaw_radians.to_bits());
        assert_eq!(curr.pitch_radians.to_bits(), prev.pitch_radians.to_bits());
        assert!(curr.world_pos.distance(prev.world_pos) > 0.0);
    }
}

#[test]
fn f3d_t05a_right_handed_look_signs_pi_wrap_shortest_slerp_and_50km_precision() {
    let tick_dt = FixedStepConfig::default().tick_duration();
    let cfg = FreeCameraConfig::default();

    // 1. At initial zero yaw & zero pitch: +X right, +Y up, -Z forward.
    let cam_ident = FreeCamera::new(cfg).unwrap();
    assert_eq!(cam_ident.curr_pose().orientation, Quat::IDENTITY);
    assert!(
        cam_ident
            .curr_pose()
            .orientation
            .right()
            .abs_diff_eq(Vec3::X, 1.0e-6)
    );
    assert!(
        cam_ident
            .curr_pose()
            .orientation
            .up()
            .abs_diff_eq(Vec3::Y, 1.0e-6)
    );
    assert!(
        cam_ident
            .curr_pose()
            .orientation
            .forward()
            .abs_diff_eq(Vec3::NEG_Z, 1.0e-6)
    );

    // 2. Right-handed pointer look signs:
    //    - `dx > 0` (pointer moved right) turns camera right (`forward.x > 0`).
    //    - `dy < 0` (pointer moved up) pitches camera up (`forward.y > 0`).
    //    - `dy > 0` (pointer moved down) pitches camera down (`forward.y < 0`).
    let mut cam_right = FreeCamera::new(cfg).unwrap();
    cam_right.step_tick(
        Tick::ZERO,
        tick_dt,
        CameraTickInput {
            look_yaw_delta: 25.0,
            look_enabled: true,
            ..CameraTickInput::IDLE
        },
    );
    let fwd_right = cam_right.curr_pose().orientation.forward();
    assert!(
        fwd_right.x > 0.05 && fwd_right.z < -0.99,
        "mouse-right (dx > 0) must rotate forward toward +X, got {fwd_right:?}"
    );

    let mut cam_up = FreeCamera::new(cfg).unwrap();
    cam_up.step_tick(
        Tick::ZERO,
        tick_dt,
        CameraTickInput {
            look_pitch_delta: -25.0,
            look_enabled: true,
            ..CameraTickInput::IDLE
        },
    );
    let fwd_up = cam_up.curr_pose().orientation.forward();
    assert!(
        fwd_up.y > 0.05 && fwd_up.z < -0.99,
        "mouse-up (dy < 0) must pitch forward toward +Y, got {fwd_up:?}"
    );

    let mut cam_down = FreeCamera::new(cfg).unwrap();
    cam_down.step_tick(
        Tick::ZERO,
        tick_dt,
        CameraTickInput {
            look_pitch_delta: 25.0,
            look_enabled: true,
            ..CameraTickInput::IDLE
        },
    );
    assert!(cam_down.curr_pose().orientation.forward().y < -0.05);

    // 3. Shortest-arc quaternion `slerp` across the +-pi yaw wrap boundary:
    //    Start at `yaw = +3.10` rad (`~177.6 deg`), rotate further left (`dx < 0`, `+dyaw = +0.0832` rad)
    //    so `curr_pose.yaw_radians` wraps to `-3.10` rad (`-177.6 deg`), flipping quaternion sign (`dot < 0`).
    //    At `alpha = 0.5`, `sample_presentation` MUST point straight back along `+Z` (`(0, 0, +1)`),
    //    never spinning 355 deg around `-Z`!
    let wrap_cfg = FreeCameraConfig {
        initial_yaw_radians: 3.10,
        look_radians_per_count: 0.01,
        ..cfg
    };
    let mut cam_wrap = FreeCamera::new(wrap_cfg).unwrap();
    let delta_across_pi = (2.0 * (std::f64::consts::PI - 3.10)) / 0.01;
    cam_wrap.step_tick(
        Tick::ZERO,
        tick_dt,
        CameraTickInput {
            look_yaw_delta: -delta_across_pi,
            look_enabled: true,
            ..CameraTickInput::IDLE
        },
    );
    assert!(cam_wrap.prev_pose().yaw_radians > 3.09);
    assert!(cam_wrap.curr_pose().yaw_radians < -3.09);
    assert!(
        cam_wrap
            .prev_pose()
            .orientation
            .dot(cam_wrap.curr_pose().orientation)
            < -0.99,
        "expected antipodal quaternion hemispheres across +-pi wrap"
    );

    let mid_pres = cam_wrap.sample_presentation(0.5, 16.0 / 9.0).unwrap();
    let mid_fwd = mid_pres.orientation.forward();
    assert!(
        mid_fwd.z > 0.999 && mid_fwd.x.abs() < 1.0e-4,
        "shortest-arc slerp across +-pi must point along +Z, got {mid_fwd:?}"
    );

    // 4. +-50 km camera-relative `alpha` interpolation precision (`SPN-INV-051`, `SPN-INV-052`):
    let far_origin = WorldPos::new(50_000.0, -50_000.0, 50_000.0);
    let mut cam_far = FreeCamera::new(cfg.with_initial_world_pos(far_origin)).unwrap();
    cam_far.step_tick(
        Tick::ZERO,
        Duration::from_secs(1),
        CameraTickInput {
            move_forward_back: 1.0, // moves -6.0 m along -Z in 1 s
            ..CameraTickInput::IDLE
        },
    );
    let pres_quarter = cam_far.sample_presentation(0.25, 16.0 / 9.0).unwrap();
    let expected_cam = far_origin + DVec3::new(0.0, 0.0, -1.5);
    assert!(pres_quarter.world_pos.abs_diff_eq(expected_cam, 1.0e-9));

    let mesh_world = expected_cam + DVec3::new(0.5, -0.25, -2.0);
    let draw = pres_quarter.extract_draw(MeshHandle::from_raw(0, 1), mesh_world);
    assert!(
        draw.camera_relative_position
            .abs_diff_eq(Vec3::new(0.5, -0.25, -2.0), 1.0e-5),
        "50 km camera-relative extraction lost precision: {:?}",
        draw.camera_relative_position
    );
}

#[test]
fn f3d_t05a_focus_loss_resets_held_movement_and_prevents_stuck_motion() {
    let (mut input, bindings) = CameraInputBindings::build_default_input().unwrap();
    let tick_dt = FixedStepConfig::default().tick_duration();
    let mut camera = FreeCamera::new(FreeCameraConfig::default()).unwrap();

    // Gain focus, hold W + D + Space, and inject relative look motion.
    input.handle_window_event(&WindowEvent::Focused(true));
    for key in [KeyCode::W, KeyCode::D, KeyCode::Space] {
        input.handle_window_event(&WindowEvent::Key(KeyEvent {
            key,
            native_code: 0,
            state: ElementState::Pressed,
        }));
    }
    input.handle_window_event(&WindowEvent::PointerRelativeMotion {
        dx: 40.0,
        dy: -20.0,
    });

    let cmd0 = input.sample_tick(Tick::ZERO);
    camera.step_tick(
        Tick::ZERO,
        tick_dt,
        bindings.sample_camera_input(&cmd0, true),
    );
    let pose_after_tick0 = *camera.curr_pose();
    assert!(
        pose_after_tick0
            .world_pos
            .distance(WorldPos::new(0.0, 1.5, 0.0))
            > 0.05
    );
    assert_ne!(pose_after_tick0.yaw_radians, 0.0);

    // Inject raw motion BEFORE focus loss, then lose focus (`Focused(false)`),
    // releasing cursor lock (`look_enabled = false`) and all held movement keys.
    input.handle_window_event(&WindowEvent::PointerRelativeMotion {
        dx: 120.0,
        dy: -80.0,
    });
    input.handle_window_event(&WindowEvent::Focused(false));

    let cmd1 = input.sample_tick(Tick::new(1));
    assert_eq!(cmd1.axis(bindings.move_forward_back), 0.0);
    assert_eq!(cmd1.axis(bindings.move_right_left), 0.0);
    assert_eq!(cmd1.axis(bindings.move_up_down), 0.0);

    camera.step_tick(
        Tick::new(1),
        tick_dt,
        bindings.sample_camera_input(&cmd1, false),
    );
    assert!(
        camera.curr_pose().bitwise_eq(&pose_after_tick0),
        "focus loss must stop held movement and prevent phantom camera rotation"
    );
}

#[test]
fn f3d_t05a_invalid_camera_and_presentation_configs_fail_cleanly_without_panics() {
    let base = FreeCameraConfig::from_fov_degrees(75.0, 0.1, 500.0);
    assert!((base.fov_y_radians - 75.0_f32.to_radians()).abs() < 1.0e-6);
    base.validate().unwrap();

    let bad_configs = [
        FreeCameraConfig {
            fov_y_radians: 0.0,
            ..base
        },
        FreeCameraConfig {
            fov_y_radians: std::f32::consts::PI,
            ..base
        },
        FreeCameraConfig { near: 0.0, ..base },
        FreeCameraConfig {
            near: 10.0,
            far: 10.0,
            ..base
        },
        FreeCameraConfig {
            move_speed_mps: 0.0,
            ..base
        },
        FreeCameraConfig {
            move_speed_mps: MAX_CAMERA_MOVE_SPEED_MPS + 1.0,
            ..base
        },
        FreeCameraConfig {
            look_radians_per_count: -0.001,
            ..base
        },
        FreeCameraConfig {
            look_radians_per_count: MAX_LOOK_RADIANS_PER_COUNT + 0.1,
            ..base
        },
        // `pitch_limit_radians` must be STRICTLY `< FRAC_PI_2`!
        FreeCameraConfig {
            pitch_limit_radians: std::f64::consts::FRAC_PI_2,
            ..base
        },
        FreeCameraConfig {
            pitch_limit_radians: 0.0,
            ..base
        },
        FreeCameraConfig {
            initial_world_pos: WorldPos::new(f64::NAN, 0.0, 0.0),
            ..base
        },
        FreeCameraConfig {
            initial_yaw_radians: f64::INFINITY,
            ..base
        },
    ];

    for bad in bad_configs {
        let err = FreeCamera::new(bad).unwrap_err();
        assert_eq!(err.kind(), EngineErrorKind::Camera);
        assert_eq!(err.code(), "ENGN-0009");
    }

    let cam = FreeCamera::new(base).unwrap();
    for bad_alpha in [-0.01, 1.0, 1.5, f64::NAN, f64::INFINITY] {
        let err = cam.sample_presentation(bad_alpha, 1.0).unwrap_err();
        assert_eq!(err.kind(), EngineErrorKind::Camera);
        assert_eq!(err.code(), "ENGN-0009");
    }
    for bad_aspect in [0.0_f32, -1.0, f32::NAN, f32::INFINITY] {
        let err = cam.sample_presentation(0.25, bad_aspect).unwrap_err();
        assert_eq!(err.kind(), EngineErrorKind::Camera);
        assert_eq!(err.code(), "ENGN-0009");
    }
}

#[test]
fn f3d_t05b_backlog_drop_clamping_bounds_catchup_ticks_and_preserves_invariants() {
    let step_cfg = FixedStepConfig::default(); // 16_666_666 ns, max_ticks_per_update = 4
    let tick_dt = step_cfg.tick_duration();
    let mut clock = FixedStepClock::new(step_cfg);
    let mut camera = FreeCamera::new(FreeCameraConfig::default()).unwrap();
    let mut observed_ticks = Vec::new();

    // Frame 0: normal 60 Hz wakeup -> Tick 0.
    let u0 = clock.advance(tick_dt).unwrap();
    assert_eq!(u0.tick_count(), 1);
    assert_eq!(u0.dropped(), Duration::ZERO);
    for tick in u0.ticks() {
        observed_ticks.push(tick.get());
        camera.step_tick(tick, tick_dt, build_scripted_tick_input(tick.get()));
    }

    // Frame 1: large 250 ms stall (`> 4 * 16_666_666 ns`; 15 whole ticks + 10 ns remainder).
    let stall = Duration::from_millis(250);
    assert!(stall > tick_dt * 4);
    let u_stall = clock.advance(stall).unwrap();
    assert!(u_stall.tick_count() <= 4);
    assert_eq!(u_stall.tick_count(), 4);
    assert!(
        u_stall.dropped() > Duration::ZERO,
        "250 ms stall must report positive dropped backlog"
    );

    let mut poses_during_stall = Vec::new();
    for tick in u_stall.ticks() {
        observed_ticks.push(tick.get());
        camera.step_tick(tick, tick_dt, build_scripted_tick_input(tick.get()));
        poses_during_stall.push(*camera.curr_pose());
    }

    // After the 4 clamped catch-up ticks (Ticks 1..=4), `prev_pose` holds Tick 3
    // and `curr_pose` holds Tick 4, both finite, and `alpha` stays in `[0.0, 1.0)`.
    assert!(camera.prev_pose().bitwise_eq(&poses_during_stall[2]));
    assert!(camera.curr_pose().bitwise_eq(&poses_during_stall[3]));
    assert!(camera.prev_pose().is_finite());
    assert!(camera.curr_pose().is_finite());

    let alpha_after_stall = interpolation_alpha(&clock);
    assert!(
        alpha_after_stall.is_finite() && (0.0..1.0).contains(&alpha_after_stall),
        "alpha after clamped stall must stay in [0.0, 1.0), got {alpha_after_stall}"
    );
    let pres = camera
        .sample_presentation(alpha_after_stall, 16.0 / 9.0)
        .unwrap();
    assert!(pres.world_pos.is_finite());
    assert!(pres.orientation.is_normalized());

    // Frame 2: recovery frame (`tick_duration`) continues monotonically at Tick 5.
    let u_after = clock.advance(tick_dt).unwrap();
    assert_eq!(u_after.tick_count(), 1);
    assert_eq!(u_after.dropped(), Duration::ZERO);
    for tick in u_after.ticks() {
        observed_ticks.push(tick.get());
        camera.step_tick(tick, tick_dt, build_scripted_tick_input(tick.get()));
    }

    assert_eq!(observed_ticks, vec![0, 1, 2, 3, 4, 5]);
    assert!(camera.curr_pose().is_finite());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn f3d_t05a_prop_arbitrary_non_dropping_partitions_match_bitwise(
        tick_count in 2_usize..=16,
        substeps in prop::collection::vec(1_u32..=8, 8..=48),
        raw_inputs in prop::collection::vec(
            (-1.0_f64..=1.0, -1.0_f64..=1.0, -1.0_f64..=1.0, -250.0_f64..=250.0, -250.0_f64..=250.0, any::<bool>()),
            16
        ),
    ) {
        // Use a 12 ms tick (`12_000_000 ns`, divisible by 8) with max_ticks_per_update = 4.
        let tick_ms = 12_u64;
        let step_cfg = FixedStepConfig::new(Duration::from_millis(tick_ms), 4).unwrap();
        let tick_dt = step_cfg.tick_duration();
        let unit_ns = tick_dt.as_nanos() as u64 / 8; // 1.5 ms units; 1..=8 units <= 1 tick <= 4 ticks!
        let total_ns = (tick_count as u64) * (tick_dt.as_nanos() as u64);

        let mut partitioned = Vec::new();
        let mut acc_ns = 0_u64;
        let mut cursor = 0_usize;
        while acc_ns < total_ns {
            let units = u64::from(substeps[cursor % substeps.len()]);
            cursor += 1;
            let step_ns = (units * unit_ns).min(total_ns - acc_ns);
            partitioned.push(Duration::from_nanos(step_ns));
            acc_ns += step_ns;
        }

        let input_fn = |tick_idx: u64| {
            let (fb, rl, ud, dyaw, dpitch, look) = raw_inputs[(tick_idx as usize) % raw_inputs.len()];
            CameraTickInput {
                move_forward_back: fb,
                move_right_left: rl,
                move_up_down: ud,
                look_yaw_delta: dyaw,
                look_pitch_delta: dpitch,
                look_enabled: look,
            }
        };

        let cam_cfg = FreeCameraConfig {
            pitch_limit_radians: std::f64::consts::FRAC_PI_2 - det::DEFAULT_PITCH_LIMIT_EPSILON_F64,
            ..FreeCameraConfig::default()
        };
        let single_tick_deltas = vec![tick_dt; tick_count];

        let (poses_partitioned, _) = run_cadence_trace(step_cfg, cam_cfg, &partitioned, input_fn);
        let (poses_single, _) = run_cadence_trace(step_cfg, cam_cfg, &single_tick_deltas, input_fn);

        prop_assert_eq!(poses_partitioned.len(), tick_count);
        prop_assert_eq!(poses_single.len(), tick_count);
        for i in 0..tick_count {
            prop_assert!(poses_partitioned[i].1.bitwise_eq(&poses_single[i].1));
            prop_assert!(poses_partitioned[i].2.bitwise_eq(&poses_single[i].2));
        }
    }
}

/// Extends [`build_scripted_tick_input`] past tick `48` with an integer-derived,
/// platform-independent look/movement schedule so 600 ticks exercise many
/// `sin_cos` arguments, pitch clamping, and yaw wrapping.
fn golden_tick_input(tick_index: u64) -> CameraTickInput {
    if tick_index < 48 {
        return build_scripted_tick_input(tick_index);
    }
    let yaw_counts = ((tick_index * 37) % 101) as f64 - 50.0;
    let pitch_counts = ((tick_index * 53) % 71) as f64 - 35.0;
    let axis = |modulus: u64, offset: u64| match (tick_index + offset) % modulus {
        0 => -1.0,
        1 => 0.0,
        _ => 1.0,
    };
    CameraTickInput {
        move_forward_back: axis(3, 0),
        move_right_left: axis(4, 1),
        move_up_down: axis(5, 2),
        look_yaw_delta: yaw_counts,
        look_pitch_delta: pitch_counts,
        look_enabled: !tick_index.is_multiple_of(7),
    }
}

fn pose_bits(pose: &FreeCameraPose) -> [u64; 9] {
    [
        pose.world_pos.x.to_bits(),
        pose.world_pos.y.to_bits(),
        pose.world_pos.z.to_bits(),
        pose.yaw_radians.to_bits(),
        pose.pitch_radians.to_bits(),
        u64::from(pose.orientation.x.to_bits()),
        u64::from(pose.orientation.y.to_bits()),
        u64::from(pose.orientation.z.to_bits()),
        u64::from(pose.orientation.w.to_bits()),
    ]
}

/// `F3D-T05c` (`F3D-FR-03`, `F3D-INV-08`): a fixed `InputCommand`-equivalent
/// schedule stepped for 600 ticks at 60 Hz yields **recorded** IEEE-754 pose
/// bit patterns at ticks 1, 60, 300, and 600. Unlike the cadence-equivalence
/// tests (self-consistency within one process), this table is compared on
/// both Linux and Windows CI, proving cross-platform bit identity.
#[test]
fn f3d_t05c_cross_platform_golden_pose_bits() {
    const CHECKPOINTS: [u64; 4] = [1, 60, 300, 600];
    let tick_dt = FixedStepConfig::default().tick_duration();
    let mut camera = FreeCamera::new(FreeCameraConfig::default()).unwrap();
    let mut actual: Vec<(u64, [u64; 9])> = Vec::new();
    for tick_index in 0..600_u64 {
        camera.step_tick(
            Tick::new(tick_index),
            tick_dt,
            golden_tick_input(tick_index),
        );
        let executed = tick_index + 1;
        if CHECKPOINTS.contains(&executed) {
            let pose = camera.curr_pose();
            assert!(pose.is_finite(), "pose at tick {executed} must stay finite");
            actual.push((executed, pose_bits(pose)));
        }
    }

    let mut table = String::from("const GOLDEN_POSES: &[(u64, [u64; 9])] = &[\n");
    for (tick, bits) in &actual {
        let cols: Vec<String> = bits.iter().map(|b| format!("{b:#018x}")).collect();
        table.push_str(&format!("    ({tick}, [{}]),\n", cols.join(", ")));
    }
    table.push_str("];\n");
    assert_eq!(
        actual.as_slice(),
        GOLDEN_POSES,
        "FreeCamera pose bits changed; if intentional and reviewed, re-record:\n{table}"
    );
}

/// Columns: `world_pos.{x,y,z}`, `yaw`, `pitch` (`f64::to_bits`), then
/// `orientation.{x,y,z,w}` (`f32::to_bits` widened). Recorded on Linux x86_64
/// (Rust 1.99.1, pure-Rust `libm`); must match on every CI target.
const GOLDEN_POSES: &[(u64, [u64; 9])] = &[
    (
        1,
        [
            0x0000000000000000,
            0x3ff8000000000000,
            0xbfb99999886b8db2,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000000000000,
            0x0000000080000000,
            0x000000003f800000,
        ],
    ),
    (
        60,
        [
            0x3fd173377dedc081,
            0x4009016d4828865f,
            0xbff12d9b4f6734c5,
            0xbff1320808a31c1e,
            0x3ff3cc67cb45dbed,
            0x000000003eff191f,
            0x00000000bed57ee4,
            0x000000003e97ff9b,
            0x000000003f33277e,
        ],
    ),
    (
        300,
        [
            0xc00a7a58d31f5278,
            0x4016e49bc67addeb,
            0x4013a8b45f27efc6,
            0xbff2fd9a42cce399,
            0x3ff2d91a612fbd9d,
            0x000000003eebcc31,
            0x00000000beee142f,
            0x000000003e9f1026,
            0x000000003f307746,
        ],
    ),
    (
        600,
        [
            0xc01e3de287be9a8c,
            0x402204f88cab9c36,
            0x4029182a719adbef,
            0xbfefff4d190e8399,
            0x3ff0ce7415003a21,
            0x000000003ee14a36,
            0x00000000bed45db7,
            0x000000003e7620be,
            0x000000003f4262ef,
        ],
    ),
];
