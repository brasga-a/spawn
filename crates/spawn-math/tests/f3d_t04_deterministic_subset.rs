//! `F3D-T04` (`F3D-FR-03`, `F3D-INV-08`, `SPN-INV-044`): headless deterministic math
//! golden-vector `to_bits()` verification across `f32` and `f64` for `sqrt`, `sin`, `cos`,
//! `sin_cos`, `atan2`, pole-clamped `asin`/`acos`, `exp`, `ln`, explicit `mul_add` (FMA),
//! and deterministic `FreeCamera` yaw/pitch orientation & basis helpers.

use spawn_math::{DVec3, Quat, Vec3, det};

#[test]
fn f3d_t04_f32_golden_bit_vectors_match_exactly() {
    // Columns: (label, x_bits, sin_bits, cos_bits, sqrt(|x|)_bits, exp(clamp(x,-20,20))_bits, ln(max(|x|, min_sub))_bits)
    const GOLDEN_F32: &[(&str, u32, u32, u32, u32, u32, u32)] = &[
        (
            "pos_zero",
            0x0000_0000,
            0x0000_0000,
            0x3f80_0000,
            0x0000_0000,
            0x3f80_0000,
            0xc2ce_8ed0,
        ),
        (
            "neg_zero",
            0x8000_0000,
            0x8000_0000,
            0x3f80_0000,
            0x0000_0000,
            0x3f80_0000,
            0xc2ce_8ed0,
        ),
        (
            "min_subnormal",
            0x0000_0001,
            0x0000_0001,
            0x3f80_0000,
            0x1a35_04f3,
            0x3f80_0000,
            0xc2ce_8ed0,
        ),
        (
            "tiny_1e_30",
            0x0da2_4260,
            0x0da2_4260,
            0x3f80_0000,
            0x2690_1d7d,
            0x3f80_0000,
            0xc28a_27b5,
        ),
        (
            "quarter",
            0x3e80_0000,
            0x3e7d_5777,
            0x3f78_0aa5,
            0x3f00_0000,
            0x3fa4_5af2,
            0xbfb1_7218,
        ),
        (
            "half",
            0x3f00_0000,
            0x3ef5_7744,
            0x3f60_a940,
            0x3f35_04f3,
            0x3fd3_094c,
            0xbf31_7218,
        ),
        (
            "frac_pi_6",
            0x3f06_0a92,
            0x3f00_0000,
            0x3f5d_b3d7,
            0x3f39_3def,
            0x3fd8_1364,
            0xbf25_a3bb,
        ),
        (
            "frac_pi_4",
            0x3f49_0fdb,
            0x3f35_04f3,
            0x3f35_04f3,
            0x3f62_dfc5,
            0x400c_5eb4,
            0xbe77_5cac,
        ),
        (
            "frac_pi_2",
            0x3fc9_0fdb,
            0x3f80_0000,
            0xb33b_bd2e,
            0x3fa0_6c99,
            0x4099_ef6f,
            0x3ee7_35da,
        ),
        (
            "three_pi_4",
            0x4016_cbe4,
            0x3f35_04f3,
            0xbf35_04f3,
            0x3fc4_7a8e,
            0x4128_cfc4,
            0x3f5b_677c,
        ),
        (
            "pi",
            0x4049_0fdb,
            0xb3bb_bd2e,
            0xbf80_0000,
            0x3fe2_dfc5,
            0x41b9_2024,
            0x3f92_8682,
        ),
        (
            "neg_frac_pi_3",
            0xbf86_0a92,
            0xbf5d_b3d8,
            0x3eff_ffff,
            0x3f82_fc5f,
            0x3eb3_abc2,
            0x3d3c_e5d6,
        ),
        (
            "neg_pi",
            0xc049_0fdb,
            0x33bb_bd2e,
            0xbf80_0000,
            0x3fe2_dfc5,
            0x3d31_0113,
            0x3f92_8682,
        ),
        (
            "large_1000",
            0x447a_0000,
            0x3f53_ae61,
            0x3f0f_f813,
            0x41fc_fb72,
            0x4de7_5844,
            0x40dd_0c55,
        ),
    ];

    let min_sub = f32::from_bits(1);
    for &(label, x_bits, sin_bits, cos_bits, sqrt_bits, exp_bits, ln_bits) in GOLDEN_F32 {
        let x = f32::from_bits(x_bits);
        assert_eq!(
            det::sin_f32(x).to_bits(),
            sin_bits,
            "sin_f32 mismatch at {label}"
        );
        assert_eq!(
            det::cos_f32(x).to_bits(),
            cos_bits,
            "cos_f32 mismatch at {label}"
        );
        let (sc_s, sc_c) = det::sin_cos_f32(x);
        assert_eq!(
            sc_s.to_bits(),
            sin_bits,
            "sin_cos_f32.0 mismatch at {label}"
        );
        assert_eq!(
            sc_c.to_bits(),
            cos_bits,
            "sin_cos_f32.1 mismatch at {label}"
        );
        assert_eq!(
            det::sqrt_f32(x.abs()).to_bits(),
            sqrt_bits,
            "sqrt_f32 mismatch at {label}"
        );
        assert_eq!(
            det::exp_f32(x.clamp(-20.0, 20.0)).to_bits(),
            exp_bits,
            "exp_f32 mismatch at {label}"
        );
        assert_eq!(
            det::ln_f32(x.abs().max(min_sub)).to_bits(),
            ln_bits,
            "ln_f32 mismatch at {label}"
        );
    }

    // Preserve IEEE-754 negative-zero sign on sqrt(-0.0).
    assert_eq!(det::sqrt_f32(-0.0_f32).to_bits(), 0x8000_0000);
}

#[test]
fn f3d_t04_f64_golden_bit_vectors_match_exactly() {
    const GOLDEN_F64: &[(&str, u64, u64, u64, u64, u64, u64)] = &[
        (
            "pos_zero",
            0x0000_0000_0000_0000,
            0x0000_0000_0000_0000,
            0x3ff0_0000_0000_0000,
            0x0000_0000_0000_0000,
            0x3ff0_0000_0000_0000,
            0xc087_4385_446d_71c3,
        ),
        (
            "neg_zero",
            0x8000_0000_0000_0000,
            0x8000_0000_0000_0000,
            0x3ff0_0000_0000_0000,
            0x0000_0000_0000_0000,
            0x3ff0_0000_0000_0000,
            0xc087_4385_446d_71c3,
        ),
        (
            "min_subnormal",
            0x0000_0000_0000_0001,
            0x0000_0000_0000_0001,
            0x3ff0_0000_0000_0000,
            0x1e60_0000_0000_0000,
            0x3ff0_0000_0000_0000,
            0xc087_4385_446d_71c3,
        ),
        (
            "tiny_1e_200",
            0x1668_7e92_154e_f7ac,
            0x1668_7e92_154e_f7ac,
            0x3ff0_0000_0000_0000,
            0x2b2b_ff2e_e48e_0530,
            0x3ff0_0000_0000_0000,
            0xc07c_c845_b54b_54f2,
        ),
        (
            "quarter",
            0x3fd0_0000_0000_0000,
            0x3fcf_aaee_d4f3_1577,
            0x3fef_0154_9f7d_eea1,
            0x3fe0_0000_0000_0000,
            0x3ff4_8b5e_3c3e_8186,
            0xbff6_2e42_fefa_39ef,
        ),
        (
            "half",
            0x3fe0_0000_0000_0000,
            0x3fde_aee8_744b_05f0,
            0x3fec_1528_065b_7d50,
            0x3fe6_a09e_667f_3bcd,
            0x3ffa_6129_8e1e_069c,
            0xbfe6_2e42_fefa_39ef,
        ),
        (
            "frac_pi_6",
            0x3fe0_c152_382d_7366,
            0x3fe0_0000_0000_0000,
            0x3feb_b67a_e858_4caa,
            0x3fe7_27bd_d175_83bc,
            0x3ffb_026c_8ac0_4b15,
            0xbfe4_b477_6284_fc89,
        ),
        (
            "frac_pi_4",
            0x3fe9_21fb_5444_2d18,
            0x3fe6_a09e_667f_3bcc,
            0x3fe6_a09e_667f_3bcd,
            0x3fec_5bf8_91b4_ef6a,
            0x4001_8bd6_6947_1caa,
            0xbfce_eb95_b094_c193,
        ),
        (
            "frac_pi_2",
            0x3ff9_21fb_5444_2d18,
            0x3ff0_0000_0000_0000,
            0x3c91_a626_3314_5c07,
            0x3ff4_0d93_1ff6_2705,
            0x4013_3ded_c855_935f,
            0x3fdc_e6bb_25aa_1315,
        ),
        (
            "three_pi_4",
            0x4002_d97c_7f33_21d2,
            0x3fe6_a09e_667f_3bcd,
            0xbfe6_a09e_667f_3bcc,
            0x3ff8_8f51_bd3a_1593,
            0x4025_19f8_817f_9939,
            0x3feb_6cef_8934_d5b0,
        ),
        (
            "pi",
            0x4009_21fb_5444_2d18,
            0x3ca1_a626_3314_5c07,
            0xbff0_0000_0000_0000,
            0x3ffc_5bf8_91b4_ef6a,
            0x4037_2404_6eb0_9339,
            0x3ff2_50d0_48e7_a1bd,
        ),
        (
            "neg_frac_pi_3",
            0xbff0_c152_382d_7366,
            0xbfeb_b67a_e858_4cab,
            0x3fdf_ffff_ffff_fffe,
            0x3ff0_5f8b_d37c_0e62,
            0x3fd6_7578_59d6_64c7,
            0x3fa7_9cb9_c753_d65d,
        ),
        (
            "neg_pi",
            0xc009_21fb_5444_2d18,
            0xbca1_a626_3314_5c07,
            0xbff0_0000_0000_0000,
            0x3ffc_5bf8_91b4_ef6a,
            0x3fa6_2022_7b59_8efa,
            0x3ff2_50d0_48e7_a1bd,
        ),
        (
            "large_1e6",
            0x412e_8480_0000_0000,
            0xbfd6_664b_2568_d867,
            0x3fed_f9df_9906_d32c,
            0x408f_4000_0000_0000,
            0x4471_9103_e408_0b45,
            0x402b_a18a_998f_ffa0,
        ),
    ];

    let min_sub = f64::from_bits(1);
    for &(label, x_bits, sin_bits, cos_bits, sqrt_bits, exp_bits, ln_bits) in GOLDEN_F64 {
        let x = f64::from_bits(x_bits);
        assert_eq!(
            det::sin_f64(x).to_bits(),
            sin_bits,
            "sin_f64 mismatch at {label}"
        );
        assert_eq!(
            det::cos_f64(x).to_bits(),
            cos_bits,
            "cos_f64 mismatch at {label}"
        );
        let (sc_s, sc_c) = det::sin_cos_f64(x);
        assert_eq!(
            sc_s.to_bits(),
            sin_bits,
            "sin_cos_f64.0 mismatch at {label}"
        );
        assert_eq!(
            sc_c.to_bits(),
            cos_bits,
            "sin_cos_f64.1 mismatch at {label}"
        );
        assert_eq!(
            det::sqrt_f64(x.abs()).to_bits(),
            sqrt_bits,
            "sqrt_f64 mismatch at {label}"
        );
        assert_eq!(
            det::exp_f64(x.clamp(-50.0, 50.0)).to_bits(),
            exp_bits,
            "exp_f64 mismatch at {label}"
        );
        assert_eq!(
            det::ln_f64(x.abs().max(min_sub)).to_bits(),
            ln_bits,
            "ln_f64 mismatch at {label}"
        );
    }

    assert_eq!(det::sqrt_f64(-0.0_f64).to_bits(), 0x8000_0000_0000_0000);
}

#[test]
fn f3d_t04_atan2_pole_clamped_inverse_trig_and_explicit_fma_bits() {
    const ATAN2_CASES: &[(u32, u32, u32, u64)] = &[
        (0x0000_0000, 0x3f80_0000, 0x0000_0000, 0x0000_0000_0000_0000), // (+0, +1) -> +0
        (0x3f80_0000, 0x3f80_0000, 0x3f49_0fdb, 0x3fe9_21fb_5444_2d18), // (+1, +1) -> +pi/4
        (0x3f80_0000, 0x0000_0000, 0x3fc9_0fdb, 0x3ff9_21fb_5444_2d18), // (+1, +0) -> +pi/2
        (0x3f80_0000, 0xbf80_0000, 0x4016_cbe4, 0x4002_d97c_7f33_21d2), // (+1, -1) -> +3pi/4
        (0x0000_0000, 0xbf80_0000, 0x4049_0fdb, 0x4009_21fb_5444_2d18), // (+0, -1) -> +pi
        (0x8000_0000, 0xbf80_0000, 0xc049_0fdb, 0xc009_21fb_5444_2d18), // (-0, -1) -> -pi
        (0xbf80_0000, 0xbf80_0000, 0xc016_cbe4, 0xc002_d97c_7f33_21d2), // (-1, -1) -> -3pi/4
        (0xbf80_0000, 0x0000_0000, 0xbfc9_0fdb, 0xbff9_21fb_5444_2d18), // (-1, +0) -> -pi/2
    ];

    for &(y_bits, x_bits, exp_f32, exp_f64) in ATAN2_CASES {
        let y = f32::from_bits(y_bits);
        let x = f32::from_bits(x_bits);
        assert_eq!(det::atan2_f32(y, x).to_bits(), exp_f32);
        assert_eq!(det::atan2_f64(y as f64, x as f64).to_bits(), exp_f64);
    }

    // Pole-clamped asin/acos across in-domain, slightly out-of-domain (±1.00001), and ±Inf!
    const INV_TRIG_CASES: &[(u32, u32, u32, u64, u64)] = &[
        (
            0xc020_0000,
            0xbfc9_0fdb,
            0x4049_0fda,
            0xbff9_21fb_5444_2d18,
            0x4009_21fb_5444_2d18,
        ), // -2.5
        (
            0xbf80_0054,
            0xbfc9_0fdb,
            0x4049_0fda,
            0xbff9_21fb_5444_2d18,
            0x4009_21fb_5444_2d18,
        ), // -1.00001
        (
            0xbf80_0000,
            0xbfc9_0fdb,
            0x4049_0fda,
            0xbff9_21fb_5444_2d18,
            0x4009_21fb_5444_2d18,
        ), // -1.0
        (
            0xbf00_0000,
            0xbf06_0a92,
            0x4006_0a92,
            0xbfe0_c152_382d_7366,
            0x4000_c152_382d_7366,
        ), // -0.5
        (
            0x0000_0000,
            0x0000_0000,
            0x3fc9_0fda,
            0x0000_0000_0000_0000,
            0x3ff9_21fb_5444_2d18,
        ), // 0.0
        (
            0x3f00_0000,
            0x3f06_0a92,
            0x3f86_0a92,
            0x3fe0_c152_382d_7366,
            0x3ff0_c152_382d_7366,
        ), // +0.5
        (
            0x3f80_0000,
            0x3fc9_0fdb,
            0x0000_0000,
            0x3ff9_21fb_5444_2d18,
            0x0000_0000_0000_0000,
        ), // +1.0
        (
            0x3f80_0054,
            0x3fc9_0fdb,
            0x0000_0000,
            0x3ff9_21fb_5444_2d18,
            0x0000_0000_0000_0000,
        ), // +1.00001
        (
            0x7f80_0000,
            0x3fc9_0fdb,
            0x0000_0000,
            0x3ff9_21fb_5444_2d18,
            0x0000_0000_0000_0000,
        ), // +Inf
        (
            0xff80_0000,
            0xbfc9_0fdb,
            0x4049_0fda,
            0xbff9_21fb_5444_2d18,
            0x4009_21fb_5444_2d18,
        ), // -Inf
    ];

    for &(in_bits, asin32, acos32, asin64, acos64) in INV_TRIG_CASES {
        let v = f32::from_bits(in_bits);
        assert_eq!(det::asin_clamped_f32(v).to_bits(), asin32);
        assert_eq!(det::acos_clamped_f32(v).to_bits(), acos32);
        assert_eq!(det::asin_clamped_f64(v as f64).to_bits(), asin64);
        assert_eq!(det::acos_clamped_f64(v as f64).to_bits(), acos64);
    }

    // NaN propagates NaN (never silently masked by pole clamping), while unclamped rejects > 1.0.
    assert!(det::asin_clamped_f32(f32::NAN).is_nan());
    assert!(det::acos_clamped_f32(f32::NAN).is_nan());
    assert!(det::asin_clamped_f64(f64::NAN).is_nan());
    assert!(det::acos_clamped_f64(f64::NAN).is_nan());
    assert!(det::asin_f32(1.001).is_nan());
    assert!(det::acos_f64(-1.001).is_nan());

    // Explicit FMA (`mul_add_f32` / `mul_add_f64`) preserves the exact `-2^-26` / `-2^-54` residual
    // where unfused `(1 + eps) * (1 - eps) - 1` rounds intermediate `1 - eps^2` to `1.0` -> `0.0`!
    let eps32 = 2.0_f32.powi(-13);
    let a32 = 1.0 + eps32;
    let b32 = 1.0 - eps32;
    assert_eq!((a32 * b32 - 1.0).to_bits(), 0x0000_0000);
    assert_eq!(det::mul_add_f32(a32, b32, -1.0), -2.0_f32.powi(-26));

    let eps64 = 2.0_f64.powi(-27);
    let a64 = 1.0 + eps64;
    let b64 = 1.0 - eps64;
    assert_eq!((a64 * b64 - 1.0).to_bits(), 0x0000_0000_0000_0000);
    assert_eq!(det::mul_add_f64(a64, b64, -1.0), -2.0_f64.powi(-54));
}

#[test]
fn f3d_t04_deterministic_camera_yaw_pitch_basis_and_quat_helpers() {
    assert_eq!(det::quat_from_yaw_pitch(0.0, 0.0), Quat::IDENTITY);
    assert_eq!(det::quat_from_yaw_pitch_f64(0.0, 0.0), Quat::IDENTITY);
    assert_eq!(det::quat_from_yaw_pitch(f32::NAN, 0.0), Quat::IDENTITY);
    assert_eq!(
        det::quat_from_yaw_pitch_f64(0.0, f64::INFINITY),
        Quat::IDENTITY
    );

    // Pitch clamping bounds within ±(pi/2 - eps) and propagates NaN.
    let limit = std::f64::consts::FRAC_PI_2 - det::DEFAULT_PITCH_LIMIT_EPSILON_F64;
    assert_eq!(
        det::clamp_pitch_f64(10.0, det::DEFAULT_PITCH_LIMIT_EPSILON_F64).to_bits(),
        limit.to_bits()
    );
    assert_eq!(
        det::clamp_pitch_f64(-10.0, det::DEFAULT_PITCH_LIMIT_EPSILON_F64).to_bits(),
        (-limit).to_bits()
    );
    assert!(det::clamp_pitch_f32(f32::NAN, det::DEFAULT_PITCH_LIMIT_EPSILON_F32).is_nan());

    // Angle wrapping stays inside (-pi, pi].
    let wrapped = det::wrap_angle_pi_f64(25.0 * std::f64::consts::TAU + 0.75);
    assert!((wrapped - 0.75).abs() <= 1.0e-12);
    assert!(det::wrap_angle_pi_f32(f32::INFINITY).is_nan());

    // Basis vectors match quaternion rotation of (+X, +Y, -Z) and stay orthonormal in f32 and f64!
    for &(yaw, pitch) in &[
        (0.0_f64, 0.0_f64),
        (0.65, -0.42),
        (-2.35, 1.21),
        (std::f64::consts::PI, -0.85),
    ] {
        let q = det::quat_from_yaw_pitch_f64(yaw, pitch);
        let b32 = det::yaw_pitch_basis_f32(yaw as f32, pitch as f32);
        let b64 = det::yaw_pitch_basis_f64(yaw, pitch);

        assert!(b64.right.is_normalized());
        assert!(b64.up.is_normalized());
        assert!(b64.forward.is_normalized());
        assert_eq!(b64.right.y, 0.0);
        assert!(b64.right.dot(b64.up).abs() <= 1.0e-15);
        assert!(b64.right.dot(b64.forward).abs() <= 1.0e-15);
        assert!(b64.up.dot(b64.forward).abs() <= 1.0e-15);
        assert!(b64.right.cross(b64.up).abs_diff_eq(-b64.forward, 1.0e-15));

        assert!(b32.right.abs_diff_eq(q.right(), 1.0e-5));
        assert!(b32.up.abs_diff_eq(q.up(), 1.0e-5));
        assert!(b32.forward.abs_diff_eq(q.forward(), 1.0e-5));

        let local = b64.local_vector(2.0, 3.0, -4.0);
        let expected: DVec3 = b64.right * 2.0 + b64.up * 3.0 + b64.forward * -4.0;
        assert_eq!(local, expected);
    }

    // Deterministic slerp_shortest matches endpoint & antipodal double-cover invariants.
    let qa = det::quat_from_yaw_pitch(0.25, -0.15);
    let qb = det::quat_from_yaw_pitch(-1.1, 0.55);
    let s1 = det::slerp_shortest(qa, qb, 0.5);
    let s2 = det::slerp_shortest(qa, -qb, 0.5);
    assert_eq!(
        s1.to_array().map(f32::to_bits),
        s2.to_array().map(f32::to_bits)
    );
    assert!(
        det::quat_from_axis_angle(Vec3::Y, 1.0)
            .unwrap()
            .is_normalized()
    );
    assert!(det::tan_f32(0.5).is_finite() && det::tan_f64(0.5).is_finite());
}
