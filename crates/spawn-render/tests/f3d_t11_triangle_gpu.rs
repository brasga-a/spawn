//! Opt-in GPU verification for Milestone 2 Phase 3 (`docs/first-3d/VERIFICATION.md`:
//! `F3D-T11`, `F3D-FR-06`, `F3D-FR-07`, `F3D-FR-08`, `F3D-FR-12`, `F3D-NFR-06`,
//! `F3D-INV-13`, `F3D-INV-14`, `F3D-INV-15`, `SPN-INV-034`, `SPN-INV-035`, `SPN-INV-092`).
//!
//! Self-contained (synthesizes valid `SPNSHDR\0` SPIR-V 1.6 shaders in memory;
//! requires zero pre-cooked `target/` files). Run with:
//! `VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation VK_KHRONOS_VALIDATION_VALIDATE_SYNC=true cargo test -p spawn-render --features vulkan -- --ignored`

#![cfg(feature = "vulkan")]
#![forbid(unsafe_code)]

use spawn_render::{
    ClearColor, CookedShader, CullMode, FrameOutcome, GpuMemoryBudget, GraphicsPipelineDescriptor,
    OffscreenTargetHandle, PipelineHandle, PipelineTarget, PresentModePreference,
    PresentationState, PushConstantRange, RenderErrorKind, Renderer, RendererDescriptor,
    SHADER_FORMAT_VERSION, SPIRV_MAGIC, ShaderHandle, ShaderStage, ShaderStageFlags, SurfaceSize,
    Validation,
};

struct SpirvBuilder {
    next_id: u32,
    capabilities: Vec<u32>,
    memory_model: Vec<u32>,
    entry_points: Vec<u32>,
    exec_modes: Vec<u32>,
    decorations: Vec<u32>,
    types_globals: Vec<u32>,
    functions: Vec<u32>,
}

impl SpirvBuilder {
    fn new() -> Self {
        Self {
            next_id: 1,
            capabilities: Vec::new(),
            memory_model: Vec::new(),
            entry_points: Vec::new(),
            exec_modes: Vec::new(),
            decorations: Vec::new(),
            types_globals: Vec::new(),
            functions: Vec::new(),
        }
    }

    fn id(&mut self) -> u32 {
        let out = self.next_id;
        self.next_id += 1;
        out
    }

    fn push_inst(section: &mut Vec<u32>, opcode: u16, operands: &[u32]) {
        let word_count = (1 + operands.len()) as u32;
        section.push((word_count << 16) | u32::from(opcode));
        section.extend_from_slice(operands);
    }

    fn finish(self) -> Vec<u32> {
        let mut words = vec![
            SPIRV_MAGIC,
            0x0001_0600, // SPIR-V 1.6
            0,           // generator
            self.next_id,
            0, // schema
        ];
        words.extend_from_slice(&self.capabilities);
        words.extend_from_slice(&self.memory_model);
        words.extend_from_slice(&self.entry_points);
        words.extend_from_slice(&self.exec_modes);
        words.extend_from_slice(&self.decorations);
        words.extend_from_slice(&self.types_globals);
        words.extend_from_slice(&self.functions);
        words
    }
}

fn pack_spirv_str(s: &str) -> Vec<u32> {
    let mut bytes = s.as_bytes().to_vec();
    bytes.push(0);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .copied()
        .map(u32::from_le_bytes)
        .collect()
}

/// Encodes a self-contained cooked vertex shader matching `shaders/triangle.slang`
/// (`clockwise = false` -> Vulkan `COUNTER_CLOCKWISE` front face;
///  `clockwise = true`  -> Vulkan `CLOCKWISE` back face).
fn encode_triangle_vs(clockwise: bool, clip_z: f32) -> Result<Vec<u8>, spawn_render::RenderError> {
    let mut b = SpirvBuilder::new();
    SpirvBuilder::push_inst(&mut b.capabilities, 17, &[1]); // OpCapability Shader
    SpirvBuilder::push_inst(&mut b.memory_model, 14, &[0, 1]); // OpMemoryModel Logical GLSL450

    let t_void = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 19, &[t_void]);
    let t_fn = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 33, &[t_fn, t_void]);
    let t_bool = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 20, &[t_bool]);
    let t_bvec3 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 23, &[t_bvec3, t_bool, 3]);
    let t_bvec4 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 23, &[t_bvec4, t_bool, 4]);
    let t_u32 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 21, &[t_u32, 32, 0]);
    let t_f32 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 22, &[t_f32, 32]);
    let t_v3f = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 23, &[t_v3f, t_f32, 3]);
    let t_v4f = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 23, &[t_v4f, t_f32, 4]);

    let t_ptr_in_u32 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 32, &[t_ptr_in_u32, 1, t_u32]);
    let t_ptr_out_v4f = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 32, &[t_ptr_out_v4f, 3, t_v4f]);
    let t_ptr_out_v3f = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 32, &[t_ptr_out_v3f, 3, t_v3f]);

    let v_vertex_idx = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 59, &[t_ptr_in_u32, v_vertex_idx, 1]);
    SpirvBuilder::push_inst(&mut b.decorations, 71, &[v_vertex_idx, 11, 42]); // BuiltIn VertexIndex

    let v_gl_pos = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 59, &[t_ptr_out_v4f, v_gl_pos, 3]);
    SpirvBuilder::push_inst(&mut b.decorations, 71, &[v_gl_pos, 11, 0]); // BuiltIn Position (0)

    let v_out_col = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 59, &[t_ptr_out_v3f, v_out_col, 3]);
    SpirvBuilder::push_inst(&mut b.decorations, 71, &[v_out_col, 30, 0]); // Location 0

    let c_u0 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 43, &[t_u32, c_u0, 0]);
    let c_u1 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 43, &[t_u32, c_u1, 1]);

    let c_f0 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 43, &[t_f32, c_f0, 0.0_f32.to_bits()]);
    let c_fn_half = b.id();
    SpirvBuilder::push_inst(
        &mut b.types_globals,
        43,
        &[t_f32, c_fn_half, (-0.5_f32).to_bits()],
    );
    let c_fp_half = b.id();
    SpirvBuilder::push_inst(
        &mut b.types_globals,
        43,
        &[t_f32, c_fp_half, 0.5_f32.to_bits()],
    );
    let c_fz = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 43, &[t_f32, c_fz, clip_z.to_bits()]);
    let c_f1 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 43, &[t_f32, c_f1, 1.0_f32.to_bits()]);
    let c_f02 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 43, &[t_f32, c_f02, 0.2_f32.to_bits()]);
    let c_f04 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 43, &[t_f32, c_f04, 0.4_f32.to_bits()]);

    let pos_top = b.id();
    SpirvBuilder::push_inst(
        &mut b.types_globals,
        44,
        &[t_v4f, pos_top, c_f0, c_fn_half, c_fz, c_f1],
    );
    let pos_bl = b.id();
    SpirvBuilder::push_inst(
        &mut b.types_globals,
        44,
        &[t_v4f, pos_bl, c_fn_half, c_fp_half, c_fz, c_f1],
    );
    let pos_br = b.id();
    SpirvBuilder::push_inst(
        &mut b.types_globals,
        44,
        &[t_v4f, pos_br, c_fp_half, c_fp_half, c_fz, c_f1],
    );

    let col_top = b.id();
    SpirvBuilder::push_inst(
        &mut b.types_globals,
        44,
        &[t_v3f, col_top, c_f1, c_f02, c_f02],
    );
    let col_bl = b.id();
    SpirvBuilder::push_inst(
        &mut b.types_globals,
        44,
        &[t_v3f, col_bl, c_f02, c_f1, c_f02],
    );
    let col_br = b.id();
    SpirvBuilder::push_inst(
        &mut b.types_globals,
        44,
        &[t_v3f, col_br, c_f02, c_f04, c_f1],
    );

    let (p1, p2, col1, col2) = if clockwise {
        (pos_br, pos_bl, col_br, col_bl)
    } else {
        (pos_bl, pos_br, col_bl, col_br)
    };

    let fn_main = b.id();
    let lbl = b.id();
    let vid = b.id();
    let is_0 = b.id();
    let is_1 = b.id();
    let b4_0 = b.id();
    let b4_1 = b.id();
    let b3_0 = b.id();
    let b3_1 = b.id();
    let pos_12 = b.id();
    let pos_out = b.id();
    let col_12 = b.id();
    let col_out = b.id();

    SpirvBuilder::push_inst(&mut b.functions, 54, &[t_void, fn_main, 0, t_fn]);
    SpirvBuilder::push_inst(&mut b.functions, 248, &[lbl]);
    SpirvBuilder::push_inst(&mut b.functions, 61, &[t_u32, vid, v_vertex_idx]);
    SpirvBuilder::push_inst(&mut b.functions, 170, &[t_bool, is_0, vid, c_u0]);
    SpirvBuilder::push_inst(&mut b.functions, 170, &[t_bool, is_1, vid, c_u1]);
    SpirvBuilder::push_inst(
        &mut b.functions,
        80,
        &[t_bvec4, b4_0, is_0, is_0, is_0, is_0],
    );
    SpirvBuilder::push_inst(
        &mut b.functions,
        80,
        &[t_bvec4, b4_1, is_1, is_1, is_1, is_1],
    );
    SpirvBuilder::push_inst(&mut b.functions, 80, &[t_bvec3, b3_0, is_0, is_0, is_0]);
    SpirvBuilder::push_inst(&mut b.functions, 80, &[t_bvec3, b3_1, is_1, is_1, is_1]);
    SpirvBuilder::push_inst(&mut b.functions, 169, &[t_v4f, pos_12, b4_1, p1, p2]);
    SpirvBuilder::push_inst(
        &mut b.functions,
        169,
        &[t_v4f, pos_out, b4_0, pos_top, pos_12],
    );
    SpirvBuilder::push_inst(&mut b.functions, 169, &[t_v3f, col_12, b3_1, col1, col2]);
    SpirvBuilder::push_inst(
        &mut b.functions,
        169,
        &[t_v3f, col_out, b3_0, col_top, col_12],
    );
    SpirvBuilder::push_inst(&mut b.functions, 62, &[v_gl_pos, pos_out]);
    SpirvBuilder::push_inst(&mut b.functions, 62, &[v_out_col, col_out]);
    SpirvBuilder::push_inst(&mut b.functions, 253, &[]);
    SpirvBuilder::push_inst(&mut b.functions, 56, &[]);

    let mut ep_operands = vec![0, fn_main];
    ep_operands.extend_from_slice(&pack_spirv_str("vs_main"));
    ep_operands.extend_from_slice(&[v_vertex_idx, v_gl_pos, v_out_col]);
    SpirvBuilder::push_inst(&mut b.entry_points, 15, &ep_operands);

    CookedShader {
        version: SHADER_FORMAT_VERSION,
        stage: ShaderStage::Vertex,
        entry_point: "vs_main".to_owned(),
        push_constants: PushConstantRange::NONE,
        descriptor_bindings: Vec::new(),
        vertex_inputs: Vec::new(),
        spirv_words: b.finish(),
    }
    .encode()
}

/// Encodes a self-contained cooked fragment shader matching `shaders/triangle.slang`.
fn encode_triangle_fs() -> Result<Vec<u8>, spawn_render::RenderError> {
    let mut b = SpirvBuilder::new();
    SpirvBuilder::push_inst(&mut b.capabilities, 17, &[1]); // OpCapability Shader
    SpirvBuilder::push_inst(&mut b.memory_model, 14, &[0, 1]); // OpMemoryModel Logical GLSL450

    let t_void = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 19, &[t_void]);
    let t_fn = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 33, &[t_fn, t_void]);
    let t_f32 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 22, &[t_f32, 32]);
    let t_v3f = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 23, &[t_v3f, t_f32, 3]);
    let t_v4f = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 23, &[t_v4f, t_f32, 4]);

    let t_ptr_in_v3f = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 32, &[t_ptr_in_v3f, 1, t_v3f]);
    let t_ptr_out_v4f = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 32, &[t_ptr_out_v4f, 3, t_v4f]);

    let v_in_col = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 59, &[t_ptr_in_v3f, v_in_col, 1]);
    SpirvBuilder::push_inst(&mut b.decorations, 71, &[v_in_col, 30, 0]); // Location 0

    let v_out_frag = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 59, &[t_ptr_out_v4f, v_out_frag, 3]);
    SpirvBuilder::push_inst(&mut b.decorations, 71, &[v_out_frag, 30, 0]); // Location 0

    let c_one = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 43, &[t_f32, c_one, 1.0_f32.to_bits()]);

    let fn_main = b.id();
    let lbl = b.id();
    let rgb = b.id();
    let r = b.id();
    let g = b.id();
    let ch_b = b.id();
    let rgba = b.id();

    SpirvBuilder::push_inst(&mut b.functions, 54, &[t_void, fn_main, 0, t_fn]);
    SpirvBuilder::push_inst(&mut b.functions, 248, &[lbl]);
    SpirvBuilder::push_inst(&mut b.functions, 61, &[t_v3f, rgb, v_in_col]);
    SpirvBuilder::push_inst(&mut b.functions, 81, &[t_f32, r, rgb, 0]);
    SpirvBuilder::push_inst(&mut b.functions, 81, &[t_f32, g, rgb, 1]);
    SpirvBuilder::push_inst(&mut b.functions, 81, &[t_f32, ch_b, rgb, 2]);
    SpirvBuilder::push_inst(&mut b.functions, 80, &[t_v4f, rgba, r, g, ch_b, c_one]);
    SpirvBuilder::push_inst(&mut b.functions, 62, &[v_out_frag, rgba]);
    SpirvBuilder::push_inst(&mut b.functions, 253, &[]);
    SpirvBuilder::push_inst(&mut b.functions, 56, &[]);

    let mut ep_operands = vec![4, fn_main];
    ep_operands.extend_from_slice(&pack_spirv_str("fs_main"));
    ep_operands.extend_from_slice(&[v_in_col, v_out_frag]);
    SpirvBuilder::push_inst(&mut b.entry_points, 15, &ep_operands);
    SpirvBuilder::push_inst(&mut b.exec_modes, 16, &[fn_main, 7]); // OriginUpperLeft

    CookedShader {
        version: SHADER_FORMAT_VERSION,
        stage: ShaderStage::Fragment,
        entry_point: "fs_main".to_owned(),
        push_constants: PushConstantRange::NONE,
        descriptor_bindings: Vec::new(),
        vertex_inputs: Vec::new(),
        spirv_words: b.finish(),
    }
    .encode()
}

fn assert_rgba_approx(actual: [u8; 4], expected: [u8; 4], tol: u8, label: &str) {
    for c in 0..4 {
        let diff = actual[c].abs_diff(expected[c]);
        assert!(
            diff <= tol,
            "{label}: channel {c} mismatch: actual={actual:?}, expected={expected:?} (tol={tol})"
        );
    }
}

fn drain_and_assert_zero_validation_errors(renderer: &mut Renderer<'_>) {
    let mut messages = Vec::new();
    let dropped = renderer.drain_validation_messages(&mut messages);
    for msg in &messages {
        eprintln!("validation {:?}: {}", msg.severity, msg.text);
    }
    assert_eq!(
        renderer.validation_error_count(),
        0,
        "expected 0 validation errors (dropped={dropped})"
    );
}

/// F3D-T11:
/// 1. Offscreen `R8G8B8A8_UNORM` (`COLOR_ATTACHMENT | TRANSFER_SRC`) + `D32_SFLOAT` (`clear = 0.0`)
///    dynamic-rendering colored CCW triangle + `vkCmdCopyImageToBuffer2` readback:
///    verifies center pixel matches expected interpolated RGBA and all 4 corners match clear color.
/// 2. Verifies CW / back-facing triangle is culled (`VK_CULL_MODE_BACK_BIT` +
///    `VK_FRONT_FACE_COUNTER_CLOCKWISE`), while disabling culling draws it.
/// 3. Exercises untrusted shader/layout rejection (`REND-0010`, `REND-0011`), stale/forged
///    handle safety (`REND-0013`), invalid offscreen target rejection (`REND-0014`), and
///    budget exhaustion (`REND-0012`) without poisoning the renderer or emitting validation errors.
/// 4. Exercises `VK_EXT_headless_surface` swapchain (`COLOR_ATTACHMENT` only) + `D32_SFLOAT`
///    depth triangle draw/present + resize + `0x0` suspend + resume + leak-free close/drop.
#[test]
#[ignore = "needs a Vulkan 1.3 GPU with VK_EXT_headless_surface"]
fn f3d_t11_offscreen_triangle_readback_culling_swapchain_resize_and_leak_free_shutdown() {
    let descriptor = RendererDescriptor {
        present_mode: PresentModePreference::Fifo,
        validation: Validation::IfAvailable,
    };
    let mut renderer = Renderer::new_headless(SurfaceSize::new(320, 240), &descriptor)
        .expect("headless Vulkan 1.3 renderer initialization must succeed on reference GPU");
    eprintln!("F3D-T11 renderer: {:?}", renderer.info());
    if std::env::var_os("VK_INSTANCE_LAYERS").is_some() {
        assert!(
            renderer.info().validation_enabled,
            "Khronos validation layer was requested via VK_INSTANCE_LAYERS but is not active"
        );
    }

    let vs_ccw_bytes = encode_triangle_vs(false, 0.5).unwrap();
    let vs_cw_bytes = encode_triangle_vs(true, 0.5).unwrap();
    let vs_at_far_plane_bytes = encode_triangle_vs(false, 0.0).unwrap();
    let vs_outside_clip_bytes = encode_triangle_vs(false, -0.25).unwrap();
    let fs_bytes = encode_triangle_fs().unwrap();

    let vs_ccw = renderer
        .load_shader_named("triangle_ccw.vs_main.spnshdr", &vs_ccw_bytes)
        .unwrap();
    let vs_cw = renderer
        .load_shader_named("triangle_cw.vs_main.spnshdr", &vs_cw_bytes)
        .unwrap();
    let fs = renderer
        .load_shader_named("triangle.fs_main.spnshdr", &fs_bytes)
        .unwrap();

    // 1. Negative untrusted-input & handle checks BEFORE pipeline creation (`SPN-INV-034`, `035`).
    assert_eq!(
        renderer.load_shader(b"NOT_A_SHADER").unwrap_err().kind(),
        RenderErrorKind::InvalidShaderArtifact
    );
    assert_eq!(
        renderer
            .create_graphics_pipeline(
                &GraphicsPipelineDescriptor::new("bad_swapped", fs, vs_ccw)
                    .with_target(PipelineTarget::OffscreenRgba8Unorm),
            )
            .unwrap_err()
            .kind(),
        RenderErrorKind::ShaderLayoutMismatch
    );
    let mut mismatched_vs_shader = CookedShader::from_bytes(&vs_ccw_bytes).unwrap();
    mismatched_vs_shader.push_constants = PushConstantRange::new(64, ShaderStageFlags::VERTEX);
    let mismatched_vs = renderer
        .load_shader(&mismatched_vs_shader.encode().unwrap())
        .unwrap();
    assert_eq!(
        renderer
            .create_graphics_pipeline(
                &GraphicsPipelineDescriptor::new("bad_pc", mismatched_vs, fs)
                    .with_target(PipelineTarget::OffscreenRgba8Unorm),
            )
            .unwrap_err()
            .kind(),
        RenderErrorKind::ShaderLayoutMismatch
    );
    renderer.destroy_shader(mismatched_vs).unwrap();
    // Stale handle after destroy returns `REND-0013`!
    assert_eq!(
        renderer.destroy_shader(mismatched_vs).unwrap_err().kind(),
        RenderErrorKind::InvalidResourceHandle
    );
    assert_eq!(
        renderer
            .destroy_shader(ShaderHandle::from_raw(0, 1))
            .unwrap_err()
            .kind(),
        RenderErrorKind::InvalidResourceHandle
    );
    assert_eq!(
        renderer
            .create_offscreen_target(SurfaceSize::new(0, 64))
            .unwrap_err()
            .kind(),
        RenderErrorKind::InvalidResourceDescriptor
    );

    // 2. Offscreen CCW colored triangle draw + readback on odd-sized `65x65` target
    //    so center pixel `(32, 32)` samples exact NDC `(0.0, 0.0)`:
    //    barycentrics `(0.5, 0.25, 0.25)` -> `(0.60, 0.45, 0.40, 1.0)` -> `[153, 115, 102, 255]`.
    let offscreen = renderer
        .create_offscreen_target(SurfaceSize::new(65, 65))
        .unwrap();
    let pipe_ccw_offscreen = renderer
        .create_graphics_pipeline(
            &GraphicsPipelineDescriptor::new("triangle.offscreen.ccw", vs_ccw, fs)
                .with_target(PipelineTarget::OffscreenRgba8Unorm),
        )
        .unwrap();

    let clear = ClearColor::new(0.05, 0.10, 0.15, 1.0);
    let expected_clear = [13_u8, 26, 38, 255];
    let expected_center = [153_u8, 115, 102, 255];

    let readback_ccw = renderer
        .render_offscreen_triangle(offscreen, pipe_ccw_offscreen, clear, 3)
        .unwrap();
    assert_eq!(readback_ccw.size(), SurfaceSize::new(65, 65));
    assert_rgba_approx(
        readback_ccw.pixel_rgba(32, 32).unwrap(),
        expected_center,
        2,
        "CCW center pixel (32, 32)",
    );
    for &(cx, cy) in &[(0, 0), (64, 0), (0, 64), (64, 64)] {
        assert_rgba_approx(
            readback_ccw.pixel_rgba(cx, cy).unwrap(),
            expected_clear,
            1,
            &format!("CCW corner ({cx}, {cy})"),
        );
    }

    // 3. Backface culling verification: CW (back-facing) triangle with `CullMode::Back`
    //    is 100% culled (center pixel == clear color), while `CullMode::None` rasterizes it!
    let pipe_cw_culled = renderer
        .create_graphics_pipeline(
            &GraphicsPipelineDescriptor::new("triangle.offscreen.cw_culled", vs_cw, fs)
                .with_target(PipelineTarget::OffscreenRgba8Unorm)
                .with_cull_mode(CullMode::Back),
        )
        .unwrap();
    let readback_cw_culled = renderer
        .render_offscreen_triangle(offscreen, pipe_cw_culled, clear, 3)
        .unwrap();
    assert_rgba_approx(
        readback_cw_culled.pixel_rgba(32, 32).unwrap(),
        expected_clear,
        1,
        "CW back-facing center pixel must be culled to clear color",
    );
    for &(cx, cy) in &[(0, 0), (64, 0), (0, 64), (64, 64)] {
        assert_rgba_approx(
            readback_cw_culled.pixel_rgba(cx, cy).unwrap(),
            expected_clear,
            1,
            &format!("CW culled corner ({cx}, {cy})"),
        );
    }

    let pipe_cw_unculled = renderer
        .create_graphics_pipeline(
            &GraphicsPipelineDescriptor::new("triangle.offscreen.cw_none", vs_cw, fs)
                .with_target(PipelineTarget::OffscreenRgba8Unorm)
                .with_cull_mode(CullMode::None),
        )
        .unwrap();
    let readback_cw_unculled = renderer
        .render_offscreen_triangle(offscreen, pipe_cw_unculled, clear, 3)
        .unwrap();
    assert_rgba_approx(
        readback_cw_unculled.pixel_rgba(32, 32).unwrap(),
        expected_center,
        2,
        "CW center pixel with CullMode::None must rasterize",
    );

    // 3b. Reversed-Z `[0, 1]` clip depth + `GREATER_OR_EQUAL` against `clear = 0.0`:
    //     `clip_z = 0.0` (`0.0 >= 0.0`) passes, while `clip_z = -0.25` (`< 0.0`) is clipped!
    let vs_far = renderer.load_shader(&vs_at_far_plane_bytes).unwrap();
    let vs_clipped = renderer.load_shader(&vs_outside_clip_bytes).unwrap();
    let pipe_far = renderer
        .create_graphics_pipeline(
            &GraphicsPipelineDescriptor::new("triangle.offscreen.z0", vs_far, fs)
                .with_target(PipelineTarget::OffscreenRgba8Unorm),
        )
        .unwrap();
    let pipe_clipped = renderer
        .create_graphics_pipeline(
            &GraphicsPipelineDescriptor::new("triangle.offscreen.z_neg", vs_clipped, fs)
                .with_target(PipelineTarget::OffscreenRgba8Unorm),
        )
        .unwrap();
    let rb_far = renderer
        .render_offscreen_triangle(offscreen, pipe_far, clear, 3)
        .unwrap();
    assert_rgba_approx(
        rb_far.pixel_rgba(32, 32).unwrap(),
        expected_center,
        2,
        "z = 0.0 must pass GREATER_OR_EQUAL against depth clear 0.0",
    );
    let rb_clipped = renderer
        .render_offscreen_triangle(offscreen, pipe_clipped, clear, 3)
        .unwrap();
    assert_rgba_approx(
        rb_clipped.pixel_rgba(32, 32).unwrap(),
        expected_clear,
        1,
        "z = -0.25 outside Vulkan [0, 1] clip volume must be clipped",
    );
    renderer.destroy_pipeline(pipe_far).unwrap();
    renderer.destroy_pipeline(pipe_clipped).unwrap();
    renderer.destroy_shader(vs_far).unwrap();
    renderer.destroy_shader(vs_clipped).unwrap();

    // 4. Headless swapchain (`COLOR_ATTACHMENT` only) + `D32_SFLOAT` reversed-Z depth
    //    triangle draw/present across multiple frames, resizes, and `0x0` suspension!
    let pipe_swapchain = renderer
        .create_graphics_pipeline(
            &GraphicsPipelineDescriptor::new("triangle.swapchain.ccw", vs_ccw, fs)
                .with_target(PipelineTarget::Swapchain),
        )
        .unwrap();

    // Target-kind mismatch guard (`OffscreenRgba8Unorm` pipeline on swapchain and vice versa -> `REND-0014`).
    assert_eq!(
        renderer
            .render_triangle(pipe_ccw_offscreen, clear, 3)
            .unwrap_err()
            .kind(),
        RenderErrorKind::InvalidResourceDescriptor
    );
    assert_eq!(
        renderer
            .render_offscreen_triangle(offscreen, pipe_swapchain, clear, 3)
            .unwrap_err()
            .kind(),
        RenderErrorKind::InvalidResourceDescriptor
    );

    for frame in 0..8 {
        let outcome = renderer.render_triangle(pipe_swapchain, clear, 3);
        assert!(
            matches!(outcome, Ok(FrameOutcome::Presented)),
            "swapchain triangle frame {frame}: {outcome:?}"
        );
    }

    // Suspend at `0x0` (no acquire, no `0x0` depth allocation).
    renderer.resize(SurfaceSize::new(0, 0));
    for _ in 0..3 {
        assert_eq!(
            renderer.render_triangle(pipe_swapchain, clear, 3).unwrap(),
            FrameOutcome::Suspended
        );
    }
    assert_eq!(renderer.state(), PresentationState::Suspended);

    // Resize across multiple swapchain + depth extents and present cleanly.
    for (w, h) in [(640, 480), (801, 599), (1280, 720)] {
        renderer.resize(SurfaceSize::new(w, h));
        for _ in 0..2 {
            assert_eq!(
                renderer.render_triangle(pipe_swapchain, clear, 3).unwrap(),
                FrameOutcome::Presented
            );
        }
    }

    // 5. Destroy resources while frames may still be completing, verify stale handles fail,
    //    reclaim via `wait_idle_and_reclaim`, verify zero validation errors, and close cleanly!
    renderer.destroy_offscreen_target(offscreen).unwrap();
    assert_eq!(
        renderer
            .destroy_offscreen_target(OffscreenTargetHandle::from_raw(0, 1))
            .unwrap_err()
            .kind(),
        RenderErrorKind::InvalidResourceHandle
    );
    renderer.destroy_pipeline(pipe_ccw_offscreen).unwrap();
    renderer.destroy_pipeline(pipe_cw_culled).unwrap();
    renderer.destroy_pipeline(pipe_cw_unculled).unwrap();
    renderer.destroy_pipeline(pipe_swapchain).unwrap();
    assert_eq!(
        renderer
            .destroy_pipeline(pipe_swapchain)
            .unwrap_err()
            .kind(),
        RenderErrorKind::InvalidResourceHandle
    );
    assert_eq!(
        renderer
            .render_triangle(PipelineHandle::from_raw(0, 1), clear, 3)
            .unwrap_err()
            .kind(),
        RenderErrorKind::InvalidResourceHandle
    );
    renderer.destroy_shader(vs_ccw).unwrap();
    renderer.destroy_shader(vs_cw).unwrap();
    renderer.destroy_shader(fs).unwrap();

    let reclaimed_stats = renderer.wait_idle_and_reclaim().unwrap();
    assert_eq!(reclaimed_stats.retired_allocations, 0);
    assert_eq!(reclaimed_stats.retired_bytes, 0);
    // Only the 1280x720 swapchain `D32_SFLOAT` depth image remains live!
    assert_eq!(reclaimed_stats.live_allocations, 1);

    drain_and_assert_zero_validation_errors(&mut renderer);
    let closed_stats = renderer.close().unwrap();
    assert_eq!(closed_stats.block_count, 0);
    assert_eq!(closed_stats.live_allocations, 0);
    assert_eq!(closed_stats.retired_allocations, 0);

    // 6. Also verify GPU budget exhaustion (`REND-0012`) unwinds cleanly on a second
    //    renderer with a tight `DEVICE_LOCAL` budget, leaving zero validation errors on drop!
    {
        let tight_budget = GpuMemoryBudget {
            device_local_bytes: 512 * 1024, // 512 KiB
            host_visible_bytes: 512 * 1024,
            device_local_block_bytes: 512 * 1024,
            host_visible_block_bytes: 256 * 1024,
        };
        let mut tight_renderer =
            Renderer::new_headless_with_budget(SurfaceSize::new(0, 0), &descriptor, tight_budget)
                .unwrap();
        // A 512x512 RGBA8 + D32_SFLOAT offscreen target needs >= 2 MiB > 512 KiB -> REND-0012!
        let oom_err = tight_renderer
            .create_offscreen_target(SurfaceSize::new(512, 512))
            .unwrap_err();
        assert_eq!(oom_err.kind(), RenderErrorKind::GpuMemoryExhausted);
        assert_eq!(oom_err.code(), "REND-0012");
        drain_and_assert_zero_validation_errors(&mut tight_renderer);
        // Implicit `Drop` on `tight_renderer` exercises the automatic RAII teardown path too.
    }
}
