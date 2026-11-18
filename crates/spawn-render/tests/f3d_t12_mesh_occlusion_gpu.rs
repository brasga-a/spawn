//! Opt-in GPU verification for Milestone 2 Phase 4 (`docs/first-3d/VERIFICATION.md`:
//! `F3D-T12`, `F3D-FR-02`, `F3D-FR-06`, `F3D-FR-07`, `F3D-FR-08`, `F3D-FR-10`,
//! `F3D-FR-12`, `F3D-NFR-06`, `F3D-INV-06`, `F3D-INV-07`, `F3D-INV-13`,
//! `F3D-INV-14`, `F3D-INV-15`, `SPN-INV-034`, `SPN-INV-035`, `SPN-INV-052`,
//! `SPN-INV-053`, `SPN-INV-092`).
//!
//! Self-contained (synthesizes valid `SPNSHDR\0` SPIR-V 1.6 `mesh_3d` shaders in
//! memory so zero pre-existing `target/` files are required, plus exercises the
//! real `slangc 2026.22`-compiled `shaders/mesh_3d.slang` when available).
//! Run with:
//! `PATH="$HOME/.local/bin:$PATH" VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation VK_KHRONOS_VALIDATION_VALIDATE_SYNC=true cargo test -p spawn-render --features vulkan -- --ignored`

#![cfg(feature = "vulkan")]
#![forbid(unsafe_code)]

use std::f32::consts::{FRAC_PI_2, PI};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use spawn_math::{DVec3, Mat4, Quat, Vec3, Vec4, WorldPos};
use spawn_render::{
    CameraRelativeView, ClearColor, CookedShader, CullMode, FrameOutcome, GpuMemoryBudget,
    GraphicsPipelineDescriptor, MESH_PUSH_CONSTANT_RANGE, MESH_VERTEX_ATTRIBUTES, MeshDescriptor,
    MeshDraw, MeshHandle, MeshVertex, OffscreenReadback, PipelineHandle, PipelineTarget,
    PresentModePreference, PresentationState, PushConstantRange, RenderError, RenderErrorKind,
    Renderer, RendererDescriptor, SHADER_FORMAT_VERSION, SPIRV_MAGIC, ShaderHandle, ShaderStage,
    SurfaceSize, Validation,
};

struct SpirvBuilder {
    next_id: u32,
    capabilities: Vec<u32>,
    ext_imports: Vec<u32>,
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
            ext_imports: Vec::new(),
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
        words.extend_from_slice(&self.ext_imports);
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

/// Synthesizes a valid SPIR-V 1.6 cooked vertex shader (`SPNSHDR\0`) implementing
/// `vs_main` in `shaders/mesh_3d.slang` (`128`-byte `MeshPushConstants`, 3 vertex
/// input attributes at locations `0..=2`, column-major `mvp_camera_relative * vec4(pos, 1.0)`).
fn encode_mesh_3d_vs() -> Result<Vec<u8>, RenderError> {
    let mut b = SpirvBuilder::new();
    SpirvBuilder::push_inst(&mut b.capabilities, 17, &[1]); // OpCapability Shader
    let ext_glsl = b.id();
    let mut ext_ops = vec![ext_glsl];
    ext_ops.extend_from_slice(&pack_spirv_str("GLSL.std.450"));
    SpirvBuilder::push_inst(&mut b.ext_imports, 11, &ext_ops); // OpExtInstImport "GLSL.std.450"
    SpirvBuilder::push_inst(&mut b.memory_model, 14, &[0, 1]); // OpMemoryModel Logical GLSL450

    let t_void = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 19, &[t_void]);
    let t_fn = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 33, &[t_fn, t_void]);
    let t_u32 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 21, &[t_u32, 32, 0]);
    let t_f32 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 22, &[t_f32, 32]);
    let t_v3f = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 23, &[t_v3f, t_f32, 3]);
    let t_v4f = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 23, &[t_v4f, t_f32, 4]);
    let t_mat4 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 24, &[t_mat4, t_v4f, 4]);

    let t_pc_struct = b.id();
    SpirvBuilder::push_inst(
        &mut b.types_globals,
        30,
        &[t_pc_struct, t_mat4, t_v4f, t_v4f, t_v4f, t_v4f],
    );
    SpirvBuilder::push_inst(&mut b.decorations, 71, &[t_pc_struct, 2]); // Block
    SpirvBuilder::push_inst(&mut b.decorations, 72, &[t_pc_struct, 0, 35, 0]); // Offset 0
    SpirvBuilder::push_inst(&mut b.decorations, 72, &[t_pc_struct, 0, 5]); // ColMajor
    SpirvBuilder::push_inst(&mut b.decorations, 72, &[t_pc_struct, 0, 7, 16]); // MatrixStride 16
    SpirvBuilder::push_inst(&mut b.decorations, 72, &[t_pc_struct, 1, 35, 64]); // Offset 64
    SpirvBuilder::push_inst(&mut b.decorations, 72, &[t_pc_struct, 2, 35, 80]); // Offset 80
    SpirvBuilder::push_inst(&mut b.decorations, 72, &[t_pc_struct, 3, 35, 96]); // Offset 96
    SpirvBuilder::push_inst(&mut b.decorations, 72, &[t_pc_struct, 4, 35, 112]); // Offset 112

    let t_ptr_pc_struct = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 32, &[t_ptr_pc_struct, 9, t_pc_struct]);
    let t_ptr_pc_mat4 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 32, &[t_ptr_pc_mat4, 9, t_mat4]);
    let t_ptr_pc_v4f = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 32, &[t_ptr_pc_v4f, 9, t_v4f]);
    let t_ptr_in_v3f = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 32, &[t_ptr_in_v3f, 1, t_v3f]);
    let t_ptr_in_v4f = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 32, &[t_ptr_in_v4f, 1, t_v4f]);
    let t_ptr_out_v4f = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 32, &[t_ptr_out_v4f, 3, t_v4f]);
    let t_ptr_out_v3f = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 32, &[t_ptr_out_v3f, 3, t_v3f]);
    let t_ptr_out_f32 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 32, &[t_ptr_out_f32, 3, t_f32]);

    let v_pc = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 59, &[t_ptr_pc_struct, v_pc, 9]);

    let v_in_pos = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 59, &[t_ptr_in_v3f, v_in_pos, 1]);
    SpirvBuilder::push_inst(&mut b.decorations, 71, &[v_in_pos, 30, 0]);
    let v_in_norm = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 59, &[t_ptr_in_v3f, v_in_norm, 1]);
    SpirvBuilder::push_inst(&mut b.decorations, 71, &[v_in_norm, 30, 1]);
    let v_in_col = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 59, &[t_ptr_in_v4f, v_in_col, 1]);
    SpirvBuilder::push_inst(&mut b.decorations, 71, &[v_in_col, 30, 2]);

    let v_gl_pos = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 59, &[t_ptr_out_v4f, v_gl_pos, 3]);
    SpirvBuilder::push_inst(&mut b.decorations, 71, &[v_gl_pos, 11, 0]); // BuiltIn Position (0)
    let v_out_norm = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 59, &[t_ptr_out_v3f, v_out_norm, 3]);
    SpirvBuilder::push_inst(&mut b.decorations, 71, &[v_out_norm, 30, 0]);
    let v_out_col = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 59, &[t_ptr_out_v4f, v_out_col, 3]);
    SpirvBuilder::push_inst(&mut b.decorations, 71, &[v_out_col, 30, 1]);
    let v_out_shade = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 59, &[t_ptr_out_f32, v_out_shade, 3]);
    SpirvBuilder::push_inst(&mut b.decorations, 71, &[v_out_shade, 30, 2]);
    SpirvBuilder::push_inst(&mut b.decorations, 71, &[v_out_shade, 14]); // Flat

    let c_u0 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 43, &[t_u32, c_u0, 0]);
    let c_u1 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 43, &[t_u32, c_u1, 1]);
    let c_u2 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 43, &[t_u32, c_u2, 2]);
    let c_u3 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 43, &[t_u32, c_u3, 3]);
    let c_u4 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 43, &[t_u32, c_u4, 4]);

    let c_f0 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 43, &[t_f32, c_f0, 0.0_f32.to_bits()]);
    let c_f1 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 43, &[t_f32, c_f1, 1.0_f32.to_bits()]);

    let fn_main = b.id();
    let lbl_entry = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 54, &[t_void, fn_main, 0, t_fn]);
    SpirvBuilder::push_inst(&mut b.functions, 248, &[lbl_entry]);

    let pos = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 61, &[t_v3f, pos, v_in_pos]);
    let norm = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 61, &[t_v3f, norm, v_in_norm]);
    let in_col = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 61, &[t_v4f, in_col, v_in_col]);

    let px = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 81, &[t_f32, px, pos, 0]);
    let py = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 81, &[t_f32, py, pos, 1]);
    let pz = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 81, &[t_f32, pz, pos, 2]);
    let pos4 = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 80, &[t_v4f, pos4, px, py, pz, c_f1]);

    let ptr_mvp = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 65, &[t_ptr_pc_mat4, ptr_mvp, v_pc, c_u0]);
    let mvp = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 61, &[t_mat4, mvp, ptr_mvp]);
    let clip = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 145, &[t_v4f, clip, mvp, pos4]); // OpMatrixTimesVector
    SpirvBuilder::push_inst(&mut b.functions, 62, &[v_gl_pos, clip]);

    let ptr_nc0 = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 65, &[t_ptr_pc_v4f, ptr_nc0, v_pc, c_u1]);
    let nc0 = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 61, &[t_v4f, nc0, ptr_nc0]);
    let ptr_nc1 = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 65, &[t_ptr_pc_v4f, ptr_nc1, v_pc, c_u2]);
    let nc1 = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 61, &[t_v4f, nc1, ptr_nc1]);
    let ptr_nc2 = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 65, &[t_ptr_pc_v4f, ptr_nc2, v_pc, c_u3]);
    let nc2 = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 61, &[t_v4f, nc2, ptr_nc2]);
    let ptr_base = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 65, &[t_ptr_pc_v4f, ptr_base, v_pc, c_u4]);
    let base_col = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 61, &[t_v4f, base_col, ptr_base]);

    let nc0_xyz = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 79, &[t_v3f, nc0_xyz, nc0, nc0, 0, 1, 2]);
    let nc1_xyz = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 79, &[t_v3f, nc1_xyz, nc1, nc1, 0, 1, 2]);
    let nc2_xyz = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 79, &[t_v3f, nc2_xyz, nc2, nc2, 0, 1, 2]);

    let nx = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 81, &[t_f32, nx, norm, 0]);
    let ny = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 81, &[t_f32, ny, norm, 1]);
    let nz = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 81, &[t_f32, nz, norm, 2]);

    let term0 = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 142, &[t_v3f, term0, nc0_xyz, nx]);
    let term1 = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 142, &[t_v3f, term1, nc1_xyz, ny]);
    let term2 = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 142, &[t_v3f, term2, nc2_xyz, nz]);
    let sum01 = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 129, &[t_v3f, sum01, term0, term1]);
    let world_norm = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 129, &[t_v3f, world_norm, sum01, term2]);
    SpirvBuilder::push_inst(&mut b.functions, 62, &[v_out_norm, world_norm]);

    let out_col = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 133, &[t_v4f, out_col, in_col, base_col]); // OpFMul
    SpirvBuilder::push_inst(&mut b.functions, 62, &[v_out_col, out_col]);

    let shade_raw = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 81, &[t_f32, shade_raw, nc0, 3]);
    let shade = b.id();
    SpirvBuilder::push_inst(
        &mut b.functions,
        12,
        &[t_f32, shade, ext_glsl, 43, shade_raw, c_f0, c_f1], // FClamp
    );
    SpirvBuilder::push_inst(&mut b.functions, 62, &[v_out_shade, shade]);

    SpirvBuilder::push_inst(&mut b.functions, 253, &[]);
    SpirvBuilder::push_inst(&mut b.functions, 56, &[]);

    let mut ep_operands = vec![0, fn_main]; // ExecutionModel::Vertex = 0
    ep_operands.extend_from_slice(&pack_spirv_str("vs_main"));
    ep_operands.extend_from_slice(&[
        v_pc,
        v_in_pos,
        v_in_norm,
        v_in_col,
        v_gl_pos,
        v_out_norm,
        v_out_col,
        v_out_shade,
    ]);
    SpirvBuilder::push_inst(&mut b.entry_points, 15, &ep_operands);

    let cooked = CookedShader {
        version: SHADER_FORMAT_VERSION,
        stage: ShaderStage::Vertex,
        entry_point: "vs_main".to_owned(),
        push_constants: MESH_PUSH_CONSTANT_RANGE,
        descriptor_bindings: Vec::new(),
        vertex_inputs: MESH_VERTEX_ATTRIBUTES.to_vec(),
        spirv_words: b.finish(),
    };
    cooked.encode()
}

/// Synthesizes a valid SPIR-V 1.6 cooked fragment shader (`SPNSHDR\0`) implementing
/// `fs_main` in `shaders/mesh_3d.slang` (`shade_weight = 0.0` flat color vs `1.0`
/// directional shading).
fn encode_mesh_3d_fs() -> Result<Vec<u8>, RenderError> {
    let mut b = SpirvBuilder::new();
    SpirvBuilder::push_inst(&mut b.capabilities, 17, &[1]); // OpCapability Shader
    let ext_glsl = b.id();
    let mut ext_ops = vec![ext_glsl];
    ext_ops.extend_from_slice(&pack_spirv_str("GLSL.std.450"));
    SpirvBuilder::push_inst(&mut b.ext_imports, 11, &ext_ops);
    SpirvBuilder::push_inst(&mut b.memory_model, 14, &[0, 1]);

    let t_void = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 19, &[t_void]);
    let t_fn = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 33, &[t_fn, t_void]);
    let t_bool = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 20, &[t_bool]);
    let t_bvec3 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 23, &[t_bvec3, t_bool, 3]);
    let t_f32 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 22, &[t_f32, 32]);
    let t_v3f = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 23, &[t_v3f, t_f32, 3]);
    let t_v4f = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 23, &[t_v4f, t_f32, 4]);

    let t_ptr_in_v3f = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 32, &[t_ptr_in_v3f, 1, t_v3f]);
    let t_ptr_in_v4f = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 32, &[t_ptr_in_v4f, 1, t_v4f]);
    let t_ptr_in_f32 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 32, &[t_ptr_in_f32, 1, t_f32]);
    let t_ptr_out_v4f = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 32, &[t_ptr_out_v4f, 3, t_v4f]);

    let v_in_norm = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 59, &[t_ptr_in_v3f, v_in_norm, 1]);
    SpirvBuilder::push_inst(&mut b.decorations, 71, &[v_in_norm, 30, 0]);
    let v_in_col = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 59, &[t_ptr_in_v4f, v_in_col, 1]);
    SpirvBuilder::push_inst(&mut b.decorations, 71, &[v_in_col, 30, 1]);
    let v_in_shade = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 59, &[t_ptr_in_f32, v_in_shade, 1]);
    SpirvBuilder::push_inst(&mut b.decorations, 71, &[v_in_shade, 30, 2]);
    SpirvBuilder::push_inst(&mut b.decorations, 71, &[v_in_shade, 14]); // Flat

    let v_out_color = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 59, &[t_ptr_out_v4f, v_out_color, 3]);
    SpirvBuilder::push_inst(&mut b.decorations, 71, &[v_out_color, 30, 0]);

    let c_f0 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 43, &[t_f32, c_f0, 0.0_f32.to_bits()]);
    let c_f1 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 43, &[t_f32, c_f1, 1.0_f32.to_bits()]);
    let c_f035 = b.id();
    SpirvBuilder::push_inst(
        &mut b.types_globals,
        43,
        &[t_f32, c_f035, 0.35_f32.to_bits()],
    );
    let c_eps = b.id();
    SpirvBuilder::push_inst(
        &mut b.types_globals,
        43,
        &[t_f32, c_eps, 1.0e-12_f32.to_bits()],
    );

    let light_unit = Vec3::new(0.45, 0.80, 0.39).normalize_or_zero();
    let c_lx = b.id();
    SpirvBuilder::push_inst(
        &mut b.types_globals,
        43,
        &[t_f32, c_lx, light_unit.x.to_bits()],
    );
    let c_ly = b.id();
    SpirvBuilder::push_inst(
        &mut b.types_globals,
        43,
        &[t_f32, c_ly, light_unit.y.to_bits()],
    );
    let c_lz = b.id();
    SpirvBuilder::push_inst(
        &mut b.types_globals,
        43,
        &[t_f32, c_lz, light_unit.z.to_bits()],
    );
    let c_light_dir = b.id();
    SpirvBuilder::push_inst(
        &mut b.types_globals,
        44,
        &[t_v3f, c_light_dir, c_lx, c_ly, c_lz],
    );
    let c_up = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 44, &[t_v3f, c_up, c_f0, c_f1, c_f0]);

    let fn_main = b.id();
    let lbl_entry = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 54, &[t_void, fn_main, 0, t_fn]);
    SpirvBuilder::push_inst(&mut b.functions, 248, &[lbl_entry]);

    let n = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 61, &[t_v3f, n, v_in_norm]);
    let col = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 61, &[t_v4f, col, v_in_col]);
    let shade_w = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 61, &[t_f32, shade_w, v_in_shade]);

    let len_sq = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 148, &[t_f32, len_sq, n, n]); // OpDot
    let gt = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 182, &[t_bool, gt, len_sq, c_eps]);
    let gt3 = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 80, &[t_bvec3, gt3, gt, gt, gt]);
    let safe_len_sq = b.id();
    SpirvBuilder::push_inst(
        &mut b.functions,
        169,
        &[t_f32, safe_len_sq, gt, len_sq, c_f1],
    );
    let inv_len = b.id();
    SpirvBuilder::push_inst(
        &mut b.functions,
        12,
        &[t_f32, inv_len, ext_glsl, 32, safe_len_sq],
    ); // InverseSqrt
    let scaled_n = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 142, &[t_v3f, scaled_n, n, inv_len]);
    let unit_n = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 169, &[t_v3f, unit_n, gt3, scaled_n, c_up]);

    let ndotl = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 148, &[t_f32, ndotl, unit_n, c_light_dir]);
    let sat_ndotl = b.id();
    SpirvBuilder::push_inst(
        &mut b.functions,
        12,
        &[t_f32, sat_ndotl, ext_glsl, 43, ndotl, c_f0, c_f1], // FClamp
    );
    let lit = b.id();
    SpirvBuilder::push_inst(
        &mut b.functions,
        12,
        &[t_f32, lit, ext_glsl, 46, c_f035, c_f1, sat_ndotl], // FMix
    );
    let shade = b.id();
    SpirvBuilder::push_inst(
        &mut b.functions,
        12,
        &[t_f32, shade, ext_glsl, 46, c_f1, lit, shade_w], // FMix
    );

    let col_rgb = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 79, &[t_v3f, col_rgb, col, col, 0, 1, 2]);
    let shaded_rgb = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 142, &[t_v3f, shaded_rgb, col_rgb, shade]);
    let rx = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 81, &[t_f32, rx, shaded_rgb, 0]);
    let ry = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 81, &[t_f32, ry, shaded_rgb, 1]);
    let rz = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 81, &[t_f32, rz, shaded_rgb, 2]);
    let ca = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 81, &[t_f32, ca, col, 3]);
    let out_rgba = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 80, &[t_v4f, out_rgba, rx, ry, rz, ca]);
    SpirvBuilder::push_inst(&mut b.functions, 62, &[v_out_color, out_rgba]);

    SpirvBuilder::push_inst(&mut b.functions, 253, &[]);
    SpirvBuilder::push_inst(&mut b.functions, 56, &[]);

    let mut ep_operands = vec![4, fn_main]; // ExecutionModel::Fragment = 4
    ep_operands.extend_from_slice(&pack_spirv_str("fs_main"));
    ep_operands.extend_from_slice(&[v_in_norm, v_in_col, v_in_shade, v_out_color]);
    SpirvBuilder::push_inst(&mut b.entry_points, 15, &ep_operands);
    SpirvBuilder::push_inst(&mut b.exec_modes, 16, &[fn_main, 7]); // OriginUpperLeft = 7

    let cooked = CookedShader {
        version: SHADER_FORMAT_VERSION,
        stage: ShaderStage::Fragment,
        entry_point: "fs_main".to_owned(),
        push_constants: PushConstantRange::NONE,
        descriptor_bindings: Vec::new(),
        vertex_inputs: Vec::new(),
        spirv_words: b.finish(),
    };
    cooked.encode()
}

/// Encodes a minimal unindexed procedural triangle shader (`0` vertex inputs, `0` push constants)
/// for testing pipeline-kind mismatch rejection (`REND-0014`) on `render_meshes`.
fn encode_minimal_unindexed_vs() -> Result<Vec<u8>, RenderError> {
    let mut b = SpirvBuilder::new();
    SpirvBuilder::push_inst(&mut b.capabilities, 17, &[1]);
    SpirvBuilder::push_inst(&mut b.memory_model, 14, &[0, 1]);
    let t_void = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 19, &[t_void]);
    let t_fn = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 33, &[t_fn, t_void]);
    let t_f32 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 22, &[t_f32, 32]);
    let t_v4f = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 23, &[t_v4f, t_f32, 4]);
    let t_ptr_out_v4f = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 32, &[t_ptr_out_v4f, 3, t_v4f]);
    let v_gl_pos = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 59, &[t_ptr_out_v4f, v_gl_pos, 3]);
    SpirvBuilder::push_inst(&mut b.decorations, 71, &[v_gl_pos, 11, 0]);
    let c_f0 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 43, &[t_f32, c_f0, 0.0_f32.to_bits()]);
    let c_f1 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 43, &[t_f32, c_f1, 1.0_f32.to_bits()]);
    let c_pos = b.id();
    SpirvBuilder::push_inst(
        &mut b.types_globals,
        44,
        &[t_v4f, c_pos, c_f0, c_f0, c_f0, c_f1],
    );
    let fn_main = b.id();
    let lbl = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 54, &[t_void, fn_main, 0, t_fn]);
    SpirvBuilder::push_inst(&mut b.functions, 248, &[lbl]);
    SpirvBuilder::push_inst(&mut b.functions, 62, &[v_gl_pos, c_pos]);
    SpirvBuilder::push_inst(&mut b.functions, 253, &[]);
    SpirvBuilder::push_inst(&mut b.functions, 56, &[]);
    let mut ep = vec![0, fn_main];
    ep.extend_from_slice(&pack_spirv_str("vs_main"));
    ep.extend_from_slice(&[v_gl_pos]);
    SpirvBuilder::push_inst(&mut b.entry_points, 15, &ep);
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

fn encode_minimal_unindexed_fs() -> Result<Vec<u8>, RenderError> {
    let mut b = SpirvBuilder::new();
    SpirvBuilder::push_inst(&mut b.capabilities, 17, &[1]);
    SpirvBuilder::push_inst(&mut b.memory_model, 14, &[0, 1]);
    let t_void = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 19, &[t_void]);
    let t_fn = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 33, &[t_fn, t_void]);
    let t_f32 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 22, &[t_f32, 32]);
    let t_v4f = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 23, &[t_v4f, t_f32, 4]);
    let t_ptr_out_v4f = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 32, &[t_ptr_out_v4f, 3, t_v4f]);
    let v_out = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 59, &[t_ptr_out_v4f, v_out, 3]);
    SpirvBuilder::push_inst(&mut b.decorations, 71, &[v_out, 30, 0]);
    let c_f1 = b.id();
    SpirvBuilder::push_inst(&mut b.types_globals, 43, &[t_f32, c_f1, 1.0_f32.to_bits()]);
    let c_col = b.id();
    SpirvBuilder::push_inst(
        &mut b.types_globals,
        44,
        &[t_v4f, c_col, c_f1, c_f1, c_f1, c_f1],
    );
    let fn_main = b.id();
    let lbl = b.id();
    SpirvBuilder::push_inst(&mut b.functions, 54, &[t_void, fn_main, 0, t_fn]);
    SpirvBuilder::push_inst(&mut b.functions, 248, &[lbl]);
    SpirvBuilder::push_inst(&mut b.functions, 62, &[v_out, c_col]);
    SpirvBuilder::push_inst(&mut b.functions, 253, &[]);
    SpirvBuilder::push_inst(&mut b.functions, 56, &[]);
    let mut ep = vec![4, fn_main];
    ep.extend_from_slice(&pack_spirv_str("fs_main"));
    ep.extend_from_slice(&[v_out]);
    SpirvBuilder::push_inst(&mut b.entry_points, 15, &ep);
    SpirvBuilder::push_inst(&mut b.exec_modes, 16, &[fn_main, 7]);
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

/// Right-handed CCW unit quad (`[-1, 1] x [-1, 1]` in `XY`, `z = 0`, outward normal `+Z`).
fn unit_quad_mesh() -> ([MeshVertex; 4], [u32; 6]) {
    let vertices = [
        MeshVertex::new(Vec3::new(-1.0, -1.0, 0.0), Vec3::Z, Vec4::ONE),
        MeshVertex::new(Vec3::new(1.0, -1.0, 0.0), Vec3::Z, Vec4::ONE),
        MeshVertex::new(Vec3::new(1.0, 1.0, 0.0), Vec3::Z, Vec4::ONE),
        MeshVertex::new(Vec3::new(-1.0, 1.0, 0.0), Vec3::Z, Vec4::ONE),
    ];
    let indices = [0_u32, 1, 2, 0, 2, 3];
    (vertices, indices)
}

/// Closed right-handed CCW 24-vertex (`36`-index) unit cube (`[-0.5, 0.5]^3`) with outward normals.
fn unit_cube_mesh() -> (Vec<MeshVertex>, Vec<u32>) {
    let faces: [([Vec3; 4], Vec3); 6] = [
        // +Z front
        (
            [
                Vec3::new(-0.5, -0.5, 0.5),
                Vec3::new(0.5, -0.5, 0.5),
                Vec3::new(0.5, 0.5, 0.5),
                Vec3::new(-0.5, 0.5, 0.5),
            ],
            Vec3::Z,
        ),
        // -Z back
        (
            [
                Vec3::new(0.5, -0.5, -0.5),
                Vec3::new(-0.5, -0.5, -0.5),
                Vec3::new(-0.5, 0.5, -0.5),
                Vec3::new(0.5, 0.5, -0.5),
            ],
            Vec3::NEG_Z,
        ),
        // +X right
        (
            [
                Vec3::new(0.5, -0.5, 0.5),
                Vec3::new(0.5, -0.5, -0.5),
                Vec3::new(0.5, 0.5, -0.5),
                Vec3::new(0.5, 0.5, 0.5),
            ],
            Vec3::X,
        ),
        // -X left
        (
            [
                Vec3::new(-0.5, -0.5, -0.5),
                Vec3::new(-0.5, -0.5, 0.5),
                Vec3::new(-0.5, 0.5, 0.5),
                Vec3::new(-0.5, 0.5, -0.5),
            ],
            Vec3::NEG_X,
        ),
        // +Y top
        (
            [
                Vec3::new(-0.5, 0.5, 0.5),
                Vec3::new(0.5, 0.5, 0.5),
                Vec3::new(0.5, 0.5, -0.5),
                Vec3::new(-0.5, 0.5, -0.5),
            ],
            Vec3::Y,
        ),
        // -Y bottom
        (
            [
                Vec3::new(-0.5, -0.5, -0.5),
                Vec3::new(0.5, -0.5, -0.5),
                Vec3::new(0.5, -0.5, 0.5),
                Vec3::new(-0.5, -0.5, 0.5),
            ],
            Vec3::NEG_Y,
        ),
    ];

    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (face_idx, (corners, normal)) in faces.into_iter().enumerate() {
        let base = (face_idx * 4) as u32;
        for pos in corners {
            vertices.push(MeshVertex::new(pos, normal, Vec4::ONE));
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (vertices, indices)
}

fn assert_rgba_approx(actual: [u8; 4], expected: [u8; 4], tol: u8, context: &str) {
    for ch in 0..4 {
        let diff = actual[ch].abs_diff(expected[ch]);
        assert!(
            diff <= tol,
            "{context}: channel {ch} expected {expected:?} (±{tol}), got {actual:?}"
        );
    }
}

fn assert_no_validation_errors(renderer: &mut Renderer<'_>) {
    let mut messages = Vec::new();
    let dropped = renderer.drain_validation_messages(&mut messages);
    for msg in &messages {
        eprintln!("validation {:?}: {}", msg.severity, msg.text);
    }
    assert_eq!(
        renderer.validation_error_count(),
        0,
        "expected 0 Vulkan validation errors (dropped={dropped})"
    );
}

/// Runs the core Phase 4 offscreen 3D verification checks (`far-after-near` reversed-Z
/// occlusion across finite & infinite projections, single-site `+Y` up NDC orientation,
/// `CullMode::Back` backface culling, closed directional-shaded 3D cube, and `±50 km`
/// camera-relative sub-`4 mm` precision) using the supplied `vs_bytes` and `fs_bytes`.
fn verify_3d_occlusion_culling_and_50km_precision(
    renderer: &mut Renderer<'_>,
    vs_bytes: &[u8],
    fs_bytes: &[u8],
    suite_label: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let vs: ShaderHandle = renderer.load_shader_named("mesh_3d.vs", vs_bytes)?;
    let fs: ShaderHandle = renderer.load_shader_named("mesh_3d.fs", fs_bytes)?;

    let mut offscreen_desc = GraphicsPipelineDescriptor::new_mesh_3d("mesh_3d_offscreen", vs, fs);
    offscreen_desc.target = PipelineTarget::OffscreenRgba8Unorm;
    let pipe_cull_back: PipelineHandle = renderer.create_graphics_pipeline(&offscreen_desc)?;

    let mut nocull_desc = offscreen_desc;
    nocull_desc.label = "mesh_3d_offscreen_nocull";
    nocull_desc.cull_mode = CullMode::None;
    let pipe_no_cull: PipelineHandle = renderer.create_graphics_pipeline(&nocull_desc)?;

    let target = renderer.create_offscreen_target(SurfaceSize::new(129, 129))?;

    let (quad_verts, quad_indices) = unit_quad_mesh();
    let quad_mesh = renderer.upload_mesh(&MeshDescriptor::new(
        "unit_quad",
        &quad_verts,
        &quad_indices,
    ))?;

    let (cube_verts, cube_indices) = unit_cube_mesh();
    let cube_mesh = renderer.upload_mesh(&MeshDescriptor::new(
        "unit_cube",
        &cube_verts,
        &cube_indices,
    ))?;

    let clear = ClearColor::new(0.05, 0.05, 0.08, 1.0);
    let expected_clear = [13_u8, 13, 20, 255];
    let near_green = Vec4::new(0.10, 0.85, 0.25, 1.0);
    let expected_green = [26_u8, 217, 64, 255];
    let far_red = Vec4::new(0.90, 0.15, 0.20, 1.0);
    let expected_red = [230_u8, 38, 51, 255];
    let top_blue = Vec4::new(0.15, 0.45, 0.95, 1.0);
    let expected_blue = [38_u8, 115, 242, 255];

    let proj_finite = Mat4::perspective_reversed_z_rh(FRAC_PI_2, 1.0, 0.1, 200.0)?;
    let proj_infinite = Mat4::perspective_infinite_reversed_z_rh(FRAC_PI_2, 1.0, 0.1)?;

    // 1. Far-after-near reversed-Z occlusion (`-2.0 m` green drawn FIRST, overlapping
    //    `-5.0 m` red drawn SECOND) + `+Y` world-up top marker (`SPN-INV-053`).
    for (proj_name, proj) in [("finite", proj_finite), ("infinite", proj_infinite)] {
        let view = CameraRelativeView::new(proj, Quat::IDENTITY);
        let draws = [
            // Near quad at -2.0 m (`x_ndc ∈ [-0.5, 0.5]`, `y_ndc ∈ [-0.5, 0.5]`) submitted FIRST.
            MeshDraw::new(quad_mesh, Vec3::new(0.0, 0.0, -2.0))
                .with_base_color(near_green)
                .with_shade_weight(0.0),
            // Far quad at -5.0 m (`x_ndc ∈ [-0.2, 0.8]`, `y_ndc ∈ [-0.5, 0.5]`) submitted SECOND!
            MeshDraw::new(quad_mesh, Vec3::new(1.5, 0.0, -5.0))
                .with_scale(Vec3::new(2.5, 2.5, 1.0))
                .with_base_color(far_red)
                .with_shade_weight(0.0),
            // `+Y` world-up marker at `(0.0, +1.5, -2.0)` -> `y_ndc = -0.75` (top of image, row ~16).
            MeshDraw::new(quad_mesh, Vec3::new(0.0, 1.5, -2.0))
                .with_scale(Vec3::new(0.2, 0.2, 1.0))
                .with_base_color(top_blue)
                .with_shade_weight(0.0),
        ];

        let rb: OffscreenReadback =
            renderer.render_offscreen_meshes(target, pipe_cull_back, clear, &view, &draws)?;

        assert_rgba_approx(
            rb.pixel_rgba(64, 64).ok_or("missing center")?,
            expected_green,
            2,
            &format!(
                "{suite_label}/{proj_name}: overlap center must keep -2.0 m near green over -5.0 m far red"
            ),
        );
        assert_rgba_approx(
            rb.pixel_rgba(108, 64).ok_or("missing right")?,
            expected_red,
            2,
            &format!("{suite_label}/{proj_name}: unoccluded right region must show -5.0 m far red"),
        );
        assert_rgba_approx(
            rb.pixel_rgba(40, 64).ok_or("missing left")?,
            expected_green,
            2,
            &format!(
                "{suite_label}/{proj_name}: unoccluded left region must show -2.0 m near green"
            ),
        );
        // Single-site Vulkan NDC Y-flip (`SPN-INV-053`): `+Y` world up lands at top (`y = 16`), not bottom (`y = 112`).
        assert_rgba_approx(
            rb.pixel_rgba(64, 16).ok_or("missing top marker")?,
            expected_blue,
            2,
            &format!(
                "{suite_label}/{proj_name}: +Y world marker must appear near top of framebuffer"
            ),
        );
        assert_rgba_approx(
            rb.pixel_rgba(64, 112).ok_or("missing bottom")?,
            expected_clear,
            2,
            &format!(
                "{suite_label}/{proj_name}: bottom mirror of +Y marker must remain clear color"
            ),
        );
        for (cx, cy) in [(0, 0), (128, 0), (0, 128), (128, 128)] {
            assert_rgba_approx(
                rb.pixel_rgba(cx, cy).ok_or("missing corner")?,
                expected_clear,
                2,
                &format!("{suite_label}/{proj_name}: corner ({cx}, {cy})"),
            );
        }
    }

    // 2. Backface culling (`CullMode::Back` vs `CullMode::None`) + closed 3D cube shading.
    let view = CameraRelativeView::new(proj_finite, Quat::IDENTITY);
    let backfacing_quad = [MeshDraw::new(quad_mesh, Vec3::new(0.0, 0.0, -2.0))
        .with_rotation(Quat::from_rotation_y(PI))
        .with_base_color(far_red)
        .with_shade_weight(0.0)];

    let rb_culled =
        renderer.render_offscreen_meshes(target, pipe_cull_back, clear, &view, &backfacing_quad)?;
    assert_rgba_approx(
        rb_culled.pixel_rgba(64, 64).ok_or("missing center")?,
        expected_clear,
        2,
        &format!(
            "{suite_label}: 180-deg rotated backfacing quad must be culled under CullMode::Back"
        ),
    );

    let rb_nocull =
        renderer.render_offscreen_meshes(target, pipe_no_cull, clear, &view, &backfacing_quad)?;
    assert_rgba_approx(
        rb_nocull.pixel_rgba(64, 64).ok_or("missing center")?,
        expected_red,
        2,
        &format!("{suite_label}: 180-deg rotated quad must render when CullMode::None is selected"),
    );

    let shaded_cube = [MeshDraw::new(cube_mesh, Vec3::new(0.0, 0.0, -2.2))
        .with_rotation(Quat::from_rotation_y(-0.55) * Quat::from_rotation_x(0.45))
        .with_base_color(Vec4::new(0.80, 0.70, 0.30, 1.0))
        .with_shade_weight(1.0)];
    let rb_cube =
        renderer.render_offscreen_meshes(target, pipe_cull_back, clear, &view, &shaded_cube)?;
    let cube_center = rb_cube.pixel_rgba(64, 64).ok_or("missing cube center")?;
    assert!(
        cube_center[0] > 70 && cube_center[1] > 60 && cube_center[3] == 255,
        "{suite_label}: shaded 3D cube center should show lit directional color, got {cube_center:?}"
    );
    assert_rgba_approx(
        rb_cube.pixel_rgba(0, 0).ok_or("missing corner")?,
        expected_clear,
        2,
        &format!("{suite_label}: cube corner"),
    );

    // 3. `±50 km` (`WorldPos`) camera-relative precision (`SPN-INV-052`, `F3D-INV-06`):
    //    Two quads separated by only `2 mm` (`0.002 m` < `~3.9 mm` f32 ULP at `50,000 m`!)
    //    submitted far-after-near at `WorldPos::ORIGIN` vs `WorldPos(50_000, -50_000, 50_000)`
    //    must both resolve the `2 mm` reversed-Z occlusion cleanly AND produce byte-identical
    //    `129 x 129` RGBA8 framebuffers!
    let tilted_view = CameraRelativeView::new(
        proj_infinite,
        Quat::from_rotation_y(0.12) * Quat::from_rotation_x(-0.08),
    );
    let near_offset = DVec3::new(-0.15, 0.10, -2.000);
    let far_offset_2mm = DVec3::new(-0.15, 0.10, -2.002);

    let draws_at_cam = |cam: WorldPos| {
        [
            MeshDraw::from_world(quad_mesh, cam + near_offset, cam)
                .with_scale(Vec3::new(0.9, 0.9, 1.0))
                .with_base_color(near_green)
                .with_shade_weight(0.0),
            MeshDraw::from_world(quad_mesh, cam + far_offset_2mm, cam)
                .with_scale(Vec3::new(0.9, 0.9, 1.0))
                .with_base_color(far_red)
                .with_shade_weight(0.0),
        ]
    };

    let rb_origin = renderer.render_offscreen_meshes(
        target,
        pipe_cull_back,
        clear,
        &tilted_view,
        &draws_at_cam(WorldPos::ORIGIN),
    )?;
    let cam_50km = WorldPos::new(50_000.0, -50_000.0, 50_000.0);
    let rb_50km = renderer.render_offscreen_meshes(
        target,
        pipe_cull_back,
        clear,
        &tilted_view,
        &draws_at_cam(cam_50km),
    )?;

    assert_rgba_approx(
        rb_50km.pixel_rgba(64, 64).ok_or("missing 50km center")?,
        expected_green,
        2,
        &format!(
            "{suite_label}: 2 mm separation at ±50 km must resolve near green without Z-fighting"
        ),
    );
    assert_eq!(
        rb_origin.rgba8_bytes(),
        rb_50km.rgba8_bytes(),
        "{suite_label}: ±50 km camera-relative render must match origin render byte-for-byte"
    );

    renderer.destroy_mesh(cube_mesh)?;
    renderer.destroy_mesh(quad_mesh)?;
    renderer.destroy_offscreen_target(target)?;
    renderer.destroy_pipeline(pipe_no_cull)?;
    renderer.destroy_pipeline(pipe_cull_back)?;
    renderer.destroy_shader(fs)?;
    renderer.destroy_shader(vs)?;

    assert_no_validation_errors(renderer);
    Ok(())
}

#[test]
#[ignore = "needs a Vulkan 1.3 GPU with VK_EXT_headless_surface"]
fn f3d_t12_mesh_staging_reversed_z_occlusion_50km_precision_and_swapchain_gpu() {
    let descriptor = RendererDescriptor {
        present_mode: PresentModePreference::Fifo,
        validation: Validation::Required,
    };
    let mut renderer = Renderer::new_headless(SurfaceSize::new(320, 240), &descriptor)
        .expect("Vulkan 1.3 headless renderer with required validation");
    assert_eq!(renderer.state(), PresentationState::Ready);
    assert!(
        renderer.info().validation_enabled,
        "VK_LAYER_KHRONOS_validation must be active for opt-in GPU verification"
    );

    let vs_bytes = encode_mesh_3d_vs().expect("valid synthesized mesh_3d VS");
    let fs_bytes = encode_mesh_3d_fs().expect("valid synthesized mesh_3d FS");

    // Scenarios 1, 2, 3: offscreen reversed-Z occlusion, culling, cube shading, ±50 km precision.
    verify_3d_occlusion_culling_and_50km_precision(
        &mut renderer,
        &vs_bytes,
        &fs_bytes,
        "synthesized_mesh_3d",
    )
    .expect("offscreen 3D mesh verification");

    // Scenario 4: `VK_EXT_headless_surface` swapchain (`COLOR_ATTACHMENT` only) + `D32_SFLOAT`
    // depth multi-mesh draws, resize (`320x240 -> 640x360 -> 257x193`), and `0x0` suspend/resume!
    let vs = renderer
        .load_shader_named("mesh_3d.swap.vs", &vs_bytes)
        .expect("load swapchain VS");
    let fs = renderer
        .load_shader_named("mesh_3d.swap.fs", &fs_bytes)
        .expect("load swapchain FS");
    let swap_pipe = renderer
        .create_graphics_pipeline(&GraphicsPipelineDescriptor::new_mesh_3d(
            "mesh_3d_swapchain",
            vs,
            fs,
        ))
        .expect("compile swapchain 3D pipeline");

    let (cube_verts, cube_indices) = unit_cube_mesh();
    let cube_mesh = renderer
        .upload_mesh(&MeshDescriptor::new(
            "swap_cube",
            &cube_verts,
            &cube_indices,
        ))
        .expect("upload swapchain cube");
    let (quad_verts, quad_indices) = unit_quad_mesh();
    let quad_mesh = renderer
        .upload_mesh(&MeshDescriptor::new(
            "swap_quad",
            &quad_verts,
            &quad_indices,
        ))
        .expect("upload swapchain quad");

    // Immediately after `upload_mesh`, the staging buffers are queued on the timeline
    // retirement list and reclaimed automatically once timeline submissions complete!
    let clear = ClearColor::new(0.04, 0.06, 0.10, 1.0);
    let make_view = |w: u32, h: u32| {
        let aspect = (w as f32) / (h.max(1) as f32);
        let proj = Mat4::perspective_reversed_z_rh(1.05, aspect, 0.1, 500.0)
            .expect("valid swapchain projection");
        CameraRelativeView::new(proj, Quat::IDENTITY)
    };
    let swap_draws = [
        MeshDraw::new(cube_mesh, Vec3::new(-0.8, 0.0, -3.0))
            .with_rotation(Quat::from_rotation_y(0.4))
            .with_base_color(Vec4::new(0.2, 0.8, 0.5, 1.0)),
        MeshDraw::new(quad_mesh, Vec3::new(0.6, 0.0, -5.5))
            .with_base_color(Vec4::new(0.9, 0.3, 0.2, 1.0)),
    ];

    for frame in 0..4 {
        let outcome = renderer
            .render_meshes(swap_pipe, clear, &make_view(320, 240), &swap_draws)
            .expect("swapchain mesh frame");
        assert_eq!(outcome, FrameOutcome::Presented, "frame {frame}");
    }

    // Resize -> recreates swapchain & `D32_SFLOAT` depth idempotently.
    for (w, h) in [(640, 360), (257, 193)] {
        renderer.resize(SurfaceSize::new(w, h));
        let outcome = renderer
            .render_meshes(swap_pipe, clear, &make_view(w, h), &swap_draws)
            .expect("resized swapchain mesh frame");
        assert_eq!(outcome, FrameOutcome::Presented);
    }

    // `0x0` suspend -> skips acquire and never allocates a `0x0` depth image.
    renderer.resize(SurfaceSize::new(0, 0));
    assert_eq!(
        renderer
            .render_meshes(swap_pipe, clear, &make_view(1, 1), &swap_draws)
            .expect("suspended mesh frame"),
        FrameOutcome::Suspended
    );
    assert_eq!(renderer.state(), PresentationState::Suspended);

    // Resume at `400x300`.
    renderer.resize(SurfaceSize::new(400, 300));
    assert_eq!(
        renderer
            .render_meshes(swap_pipe, clear, &make_view(400, 300), &swap_draws)
            .expect("resumed mesh frame"),
        FrameOutcome::Presented
    );
    assert_eq!(renderer.state(), PresentationState::Ready);

    // Scenario 5: Negative descriptor, handle, pipeline-kind mismatch, and OOM unwind checks!
    assert_eq!(
        renderer
            .upload_mesh(&MeshDescriptor::new("empty", &[], &[0, 1, 2]))
            .unwrap_err()
            .kind(),
        RenderErrorKind::InvalidResourceDescriptor
    );
    assert_eq!(
        renderer
            .upload_mesh(&MeshDescriptor::new("bad_tri", &quad_verts, &[0, 1, 2, 3]))
            .unwrap_err()
            .kind(),
        RenderErrorKind::InvalidResourceDescriptor
    );
    assert_eq!(
        renderer
            .upload_mesh(&MeshDescriptor::new("oob_idx", &quad_verts, &[0, 1, 99]))
            .unwrap_err()
            .kind(),
        RenderErrorKind::InvalidResourceDescriptor
    );
    let mut nan_verts = quad_verts;
    nan_verts[2].position.z = f32::INFINITY;
    assert_eq!(
        renderer
            .upload_mesh(&MeshDescriptor::new("inf_v", &nan_verts, &quad_indices))
            .unwrap_err()
            .kind(),
        RenderErrorKind::InvalidResourceDescriptor
    );

    // Passing a 3D mesh pipeline to `render_triangle`, or an unindexed pipeline to `render_meshes`,
    // returns `REND-0014` without stopping the renderer!
    assert_eq!(
        renderer
            .render_triangle(swap_pipe, clear, 3)
            .unwrap_err()
            .kind(),
        RenderErrorKind::InvalidResourceDescriptor
    );
    let unidx_vs = renderer
        .load_shader_named("unidx.vs", &encode_minimal_unindexed_vs().unwrap())
        .unwrap();
    let unidx_fs = renderer
        .load_shader_named("unidx.fs", &encode_minimal_unindexed_fs().unwrap())
        .unwrap();
    let unidx_pipe = renderer
        .create_graphics_pipeline(&GraphicsPipelineDescriptor::new(
            "unidx_swap",
            unidx_vs,
            unidx_fs,
        ))
        .unwrap();
    assert_eq!(
        renderer
            .render_meshes(unidx_pipe, clear, &make_view(400, 300), &swap_draws)
            .unwrap_err()
            .kind(),
        RenderErrorKind::InvalidResourceDescriptor
    );
    renderer.destroy_pipeline(unidx_pipe).unwrap();
    renderer.destroy_shader(unidx_fs).unwrap();
    renderer.destroy_shader(unidx_vs).unwrap();

    // Non-finite `MeshDraw` returns `REND-0014` without stopping the renderer.
    let nan_draws = [MeshDraw::new(cube_mesh, Vec3::NAN)];
    assert_eq!(
        renderer
            .render_meshes(swap_pipe, clear, &make_view(400, 300), &nan_draws)
            .unwrap_err()
            .kind(),
        RenderErrorKind::InvalidResourceDescriptor
    );

    // Destroy `quad_mesh` and verify stale + forged `MeshHandle`s return `REND-0013` (`InvalidHandle`).
    renderer.destroy_mesh(quad_mesh).expect("destroy quad_mesh");
    assert_eq!(
        renderer.destroy_mesh(quad_mesh).unwrap_err().kind(),
        RenderErrorKind::InvalidResourceHandle
    );
    assert_eq!(
        renderer
            .destroy_mesh(MeshHandle::from_raw(42, 7))
            .unwrap_err()
            .kind(),
        RenderErrorKind::InvalidResourceHandle
    );
    let stale_draws = [MeshDraw::new(quad_mesh, Vec3::new(0.0, 0.0, -3.0))];
    assert_eq!(
        renderer
            .render_meshes(swap_pipe, clear, &make_view(400, 300), &stale_draws)
            .unwrap_err()
            .kind(),
        RenderErrorKind::InvalidResourceHandle
    );

    // Renderer remains healthy after all typed `REND-0013` / `REND-0014` rejections!
    let remaining_draws = [MeshDraw::new(cube_mesh, Vec3::new(0.0, 0.0, -3.0))];
    assert_eq!(
        renderer
            .render_meshes(swap_pipe, clear, &make_view(400, 300), &remaining_draws)
            .expect("healthy after typed errors"),
        FrameOutcome::Presented
    );

    renderer.destroy_mesh(cube_mesh).expect("destroy cube_mesh");
    renderer
        .destroy_pipeline(swap_pipe)
        .expect("destroy swap_pipe");
    renderer.destroy_shader(fs).expect("destroy fs");
    renderer.destroy_shader(vs).expect("destroy vs");

    let reclaimed = renderer.wait_idle_and_reclaim().expect("reclaim idle");
    // Only the swapchain `D32_SFLOAT` depth image remains live before `close()`.
    assert_eq!(reclaimed.live_allocations, 1);
    assert_eq!(reclaimed.retired_allocations, 0);

    assert_no_validation_errors(&mut renderer);
    let shutdown = renderer.close().expect("clean close");
    assert_eq!(shutdown.block_count, 0);
    assert_eq!(shutdown.live_allocations, 0);
    assert_eq!(shutdown.retired_allocations, 0);

    // Also verify budget exhaustion (`REND-0012`) during `upload_mesh` unwinds staging + partial
    // buffers cleanly with zero leaks!
    let tiny_budget = GpuMemoryBudget {
        device_local_block_bytes: 4096,
        device_local_bytes: 4096,
        host_visible_block_bytes: 65_536,
        host_visible_bytes: 65_536,
    };
    let mut oom_renderer =
        Renderer::new_headless_with_budget(SurfaceSize::new(0, 0), &descriptor, tiny_budget)
            .expect("suspended renderer with tiny device_local budget");
    assert!(oom_renderer.info().validation_enabled);
    let big_verts = vec![MeshVertex::new(Vec3::ZERO, Vec3::Z, Vec4::ONE); 200]; // 8,000 B > 4,096 B budget
    let big_indices = [0_u32, 1, 2];
    let oom_err = oom_renderer
        .upload_mesh(&MeshDescriptor::new("oversized", &big_verts, &big_indices))
        .unwrap_err();
    assert_eq!(oom_err.kind(), RenderErrorKind::GpuMemoryExhausted);
    assert_eq!(oom_err.code(), "REND-0012");
    let oom_mid_stats = oom_renderer.allocator_stats();
    assert_eq!(oom_mid_stats.live_allocations, 0);
    assert_eq!(oom_mid_stats.live_allocated_bytes, 0);
    assert_eq!(oom_mid_stats.retired_allocations, 0);
    assert_eq!(oom_mid_stats.retired_bytes, 0);
    assert_no_validation_errors(&mut oom_renderer);
    let oom_shutdown = oom_renderer.close().expect("clean close after OOM");
    assert_eq!(oom_shutdown.block_count, 0);
    assert_eq!(oom_shutdown.live_allocations, 0);
    assert_eq!(oom_shutdown.retired_allocations, 0);
}

/// Executes the real authored `shaders/mesh_3d.slang` cooked via `spawn-cli` / `slangc 2026.22`
/// on the Vulkan 1.3 GPU, verifying end-to-end shader-cook -> reflection validation ->
/// `vkCreateShaderModule` -> `dynamicRendering` reversed-Z occlusion & `±50 km` precision.
/// Fails explicitly if neither pre-cooked `target/shaders/mesh_3d.*.spnshdr` nor pinned
/// `slangc` on `PATH` is present (never silently returns `Ok(())`).
#[test]
#[ignore = "needs a Vulkan 1.3 GPU with VK_EXT_headless_surface and cooked mesh_3d shaders (or slangc)"]
fn f3d_t12_cooked_mesh_3d_slang_executes_on_gpu() {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf();

    let (vs_bytes, fs_bytes) = load_or_cook_real_mesh_3d_shaders(&workspace_root)
        .expect("real mesh_3d.slang shaders must be pre-cooked or cooked via pinned slangc");

    let descriptor = RendererDescriptor {
        present_mode: PresentModePreference::Fifo,
        validation: Validation::Required,
    };
    let mut renderer = Renderer::new_headless(SurfaceSize::new(0, 0), &descriptor)
        .expect("Vulkan 1.3 headless renderer with required validation");
    assert!(
        renderer.info().validation_enabled,
        "VK_LAYER_KHRONOS_validation must be active for opt-in GPU verification"
    );
    verify_3d_occlusion_culling_and_50km_precision(
        &mut renderer,
        &vs_bytes,
        &fs_bytes,
        "slangc_cooked_mesh_3d",
    )
    .expect("real slangc-cooked mesh_3d.slang GPU execution");
    let reclaimed = renderer.wait_idle_and_reclaim().expect("reclaim idle");
    assert_eq!(reclaimed.live_allocations, 0);
    assert_eq!(reclaimed.retired_allocations, 0);
    assert_no_validation_errors(&mut renderer);
    let shutdown = renderer.close().expect("clean close");
    assert_eq!(shutdown.block_count, 0);
    assert_eq!(shutdown.live_allocations, 0);
    assert_eq!(shutdown.retired_allocations, 0);
}

fn load_or_cook_real_mesh_3d_shaders(workspace_root: &Path) -> Result<(Vec<u8>, Vec<u8>), String> {
    let pre_vs = workspace_root.join("target/shaders/mesh_3d.vs_main.spnshdr");
    let pre_fs = workspace_root.join("target/shaders/mesh_3d.fs_main.spnshdr");
    if pre_vs.is_file() && pre_fs.is_file() {
        let vs = fs::read(&pre_vs)
            .map_err(|err| format!("failed to read {}: {err}", pre_vs.display()))?;
        let fs = fs::read(&pre_fs)
            .map_err(|err| format!("failed to read {}: {err}", pre_fs.display()))?;
        return Ok((vs, fs));
    }

    // Cook into a temporary directory via `spawn-cli` (requires pinned `slangc` on `PATH`).
    let tmp_dir = std::env::temp_dir().join(format!("spawn_f3d_t12_cook_{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp_dir);
    fs::create_dir_all(&tmp_dir)
        .map_err(|err| format!("failed to create temp dir {}: {err}", tmp_dir.display()))?;

    let cook_result = (|| -> Result<(Vec<u8>, Vec<u8>), String> {
        let out = Command::new(env!("CARGO"))
            .current_dir(workspace_root)
            .args([
                "run",
                "--quiet",
                "-p",
                "spawn-cli",
                "--",
                "shaders",
                "build",
                "--out",
            ])
            .arg(&tmp_dir)
            .output()
            .map_err(|err| format!("failed to invoke `cargo run -p spawn-cli`: {err}"))?;
        if !out.status.success() {
            return Err(format!(
                "`cargo run -p spawn-cli -- shaders build` failed (exit {:?}); ensure pinned `slangc` is on PATH or pre-cook `target/shaders/`:\nstdout: {}\nstderr: {}",
                out.status.code(),
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr),
            ));
        }
        let vs_path = tmp_dir.join("mesh_3d.vs_main.spnshdr");
        let fs_path = tmp_dir.join("mesh_3d.fs_main.spnshdr");
        let vs = fs::read(&vs_path)
            .map_err(|err| format!("failed to read {}: {err}", vs_path.display()))?;
        let fs = fs::read(&fs_path)
            .map_err(|err| format!("failed to read {}: {err}", fs_path.display()))?;
        Ok((vs, fs))
    })();

    let _ = fs::remove_dir_all(&tmp_dir);
    cook_result
}
