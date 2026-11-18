//! Verification tests for `spawn-cli` offline shader cooking and workspace
//! shader-toolchain invariants (`docs/first-3d/VERIFICATION.md`: `F3D-T06`,
//! Phase 2 slice of `F3D-T13`; `ADR-0014`, `SPN-INV-001/003/009/011/034/081/083/084`).

#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use rspirv::binary::Assemble;
use rspirv::dr::Builder;
use rspirv::spirv::{
    AddressingModel, BuiltIn, Capability, Decoration, Dim, ExecutionMode, ExecutionModel,
    FunctionControl, ImageFormat, MemoryModel, StorageClass,
};
use spawn_cli::{
    CliCommand, CliError, CliErrorKind, ProcessSlangCompiler, SLANGC_VERSION, ShaderBuildOptions,
    SlangCompileRequest, SlangCompiler, build_shaders, build_shaders_with_compiler, parse_cli_args,
    verify_slangc_version,
};
use spawn_render::{
    CookedShader, DescriptorBinding, DescriptorKind, PipelineLayoutDesc, PushConstantRange,
    RUNTIME_DESCRIPTOR_COUNT, ShaderLayoutExpectation, ShaderStage, ShaderStageFlags, VertexFormat,
    VertexInputAttribute, validate_graphics_pipeline_shaders,
};

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Unique RAII temporary directory under `std::env::temp_dir()` (no `tempfile` dependency).
struct TempTestDir {
    path: PathBuf,
}

impl TempTestDir {
    fn new(prefix: &str) -> TestResult<Self> {
        let pid = std::process::id();
        let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("spawn_cli_{prefix}_{pid}_{seq}"));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempTestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn words_to_le_bytes(words: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(words.len() * 4);
    for &w in words {
        out.extend_from_slice(&w.to_le_bytes());
    }
    out
}

/// Builds a valid SPIR-V 1.6 unindexed triangle vertex or fragment module via `rspirv`.
fn assemble_triangle_spirv(stage: ShaderStage, entry_name: &str) -> TestResult<Vec<u8>> {
    let mut b = Builder::new();
    b.set_version(1, 6);
    b.capability(Capability::Shader);
    b.memory_model(AddressingModel::Logical, MemoryModel::GLSL450);

    let void = b.type_void();
    let fn_ty = b.type_function(void, vec![]);
    let f32_ty = b.type_float(32, None);
    let u32_ty = b.type_int(32, 0);
    let v3f = b.type_vector(f32_ty, 3);
    let v4f = b.type_vector(f32_ty, 4);

    let mut iface = Vec::new();
    match stage {
        ShaderStage::Vertex => {
            let ptr_in_u32 = b.type_pointer(None, StorageClass::Input, u32_ty);
            let vert_idx = b.variable(ptr_in_u32, None, StorageClass::Input, None);
            b.decorate(
                vert_idx,
                Decoration::BuiltIn,
                [rspirv::dr::Operand::BuiltIn(BuiltIn::VertexIndex)],
            );

            let ptr_out_v4 = b.type_pointer(None, StorageClass::Output, v4f);
            let gl_pos = b.variable(ptr_out_v4, None, StorageClass::Output, None);
            b.decorate(
                gl_pos,
                Decoration::BuiltIn,
                [rspirv::dr::Operand::BuiltIn(BuiltIn::Position)],
            );

            let ptr_out_v3 = b.type_pointer(None, StorageClass::Output, v3f);
            let out_col = b.variable(ptr_out_v3, None, StorageClass::Output, None);
            b.decorate(
                out_col,
                Decoration::Location,
                [rspirv::dr::Operand::LiteralBit32(0)],
            );

            iface.extend([vert_idx, gl_pos, out_col]);
        }
        ShaderStage::Fragment | ShaderStage::Compute => {
            let ptr_in_v3 = b.type_pointer(None, StorageClass::Input, v3f);
            let in_col = b.variable(ptr_in_v3, None, StorageClass::Input, None);
            b.decorate(
                in_col,
                Decoration::Location,
                [rspirv::dr::Operand::LiteralBit32(0)],
            );

            let ptr_out_v4 = b.type_pointer(None, StorageClass::Output, v4f);
            let out_col = b.variable(ptr_out_v4, None, StorageClass::Output, None);
            b.decorate(
                out_col,
                Decoration::Location,
                [rspirv::dr::Operand::LiteralBit32(0)],
            );

            iface.extend([in_col, out_col]);
        }
    }

    let main_fn = b.begin_function(void, None, FunctionControl::NONE, fn_ty)?;
    let _ = b.begin_block(None)?;
    b.ret()?;
    b.end_function()?;

    let model = match stage {
        ShaderStage::Vertex => ExecutionModel::Vertex,
        ShaderStage::Fragment => ExecutionModel::Fragment,
        ShaderStage::Compute => ExecutionModel::GLCompute,
    };
    b.entry_point(model, main_fn, entry_name, iface);
    if stage == ShaderStage::Fragment {
        b.execution_mode(main_fn, ExecutionMode::OriginUpperLeft, []);
    }

    Ok(words_to_le_bytes(&b.module().assemble()))
}

/// Builds a valid SPIR-V 1.6 `mesh_3d` vertex or fragment module matching `shaders/mesh_3d.slang`
/// (`128`-byte push constants on `vs_main` + vertex locations `0, 1, 2`).
fn assemble_mesh_3d_spirv(stage: ShaderStage, entry_name: &str) -> TestResult<Vec<u8>> {
    let mut b = Builder::new();
    b.set_version(1, 6);
    b.capability(Capability::Shader);
    b.memory_model(AddressingModel::Logical, MemoryModel::GLSL450);

    let void = b.type_void();
    let fn_ty = b.type_function(void, vec![]);
    let f32_ty = b.type_float(32, None);
    let v3f = b.type_vector(f32_ty, 3);
    let v4f = b.type_vector(f32_ty, 4);
    let mat4 = b.type_matrix(v4f, 4);

    let mut iface = Vec::new();
    if stage == ShaderStage::Vertex {
        let pc_struct = b.type_struct([mat4, v4f, v4f, v4f, v4f]);
        b.decorate(pc_struct, Decoration::Block, []);
        b.member_decorate(
            pc_struct,
            0,
            Decoration::Offset,
            [rspirv::dr::Operand::LiteralBit32(0)],
        );
        b.member_decorate(pc_struct, 0, Decoration::RowMajor, []);
        b.member_decorate(
            pc_struct,
            0,
            Decoration::MatrixStride,
            [rspirv::dr::Operand::LiteralBit32(16)],
        );
        b.member_decorate(
            pc_struct,
            1,
            Decoration::Offset,
            [rspirv::dr::Operand::LiteralBit32(64)],
        );
        b.member_decorate(
            pc_struct,
            2,
            Decoration::Offset,
            [rspirv::dr::Operand::LiteralBit32(80)],
        );
        b.member_decorate(
            pc_struct,
            3,
            Decoration::Offset,
            [rspirv::dr::Operand::LiteralBit32(96)],
        );
        b.member_decorate(
            pc_struct,
            4,
            Decoration::Offset,
            [rspirv::dr::Operand::LiteralBit32(112)],
        );

        let ptr_pc = b.type_pointer(None, StorageClass::PushConstant, pc_struct);
        let pc_var = b.variable(ptr_pc, None, StorageClass::PushConstant, None);

        let ptr_in_v3 = b.type_pointer(None, StorageClass::Input, v3f);
        let ptr_in_v4 = b.type_pointer(None, StorageClass::Input, v4f);
        let in_pos = b.variable(ptr_in_v3, None, StorageClass::Input, None);
        let in_norm = b.variable(ptr_in_v3, None, StorageClass::Input, None);
        let in_col = b.variable(ptr_in_v4, None, StorageClass::Input, None);
        b.decorate(
            in_pos,
            Decoration::Location,
            [rspirv::dr::Operand::LiteralBit32(0)],
        );
        b.decorate(
            in_norm,
            Decoration::Location,
            [rspirv::dr::Operand::LiteralBit32(1)],
        );
        b.decorate(
            in_col,
            Decoration::Location,
            [rspirv::dr::Operand::LiteralBit32(2)],
        );

        let ptr_out_v4 = b.type_pointer(None, StorageClass::Output, v4f);
        let gl_pos = b.variable(ptr_out_v4, None, StorageClass::Output, None);
        b.decorate(
            gl_pos,
            Decoration::BuiltIn,
            [rspirv::dr::Operand::BuiltIn(BuiltIn::Position)],
        );

        iface.extend([pc_var, in_pos, in_norm, in_col, gl_pos]);
    } else {
        let ptr_out_v4 = b.type_pointer(None, StorageClass::Output, v4f);
        let out_col = b.variable(ptr_out_v4, None, StorageClass::Output, None);
        b.decorate(
            out_col,
            Decoration::Location,
            [rspirv::dr::Operand::LiteralBit32(0)],
        );
        iface.push(out_col);
    }

    let main_fn = b.begin_function(void, None, FunctionControl::NONE, fn_ty)?;
    let _ = b.begin_block(None)?;
    b.ret()?;
    b.end_function()?;

    let model = if stage == ShaderStage::Vertex {
        ExecutionModel::Vertex
    } else {
        ExecutionModel::Fragment
    };
    b.entry_point(model, main_fn, entry_name, iface);
    if stage == ShaderStage::Fragment {
        b.execution_mode(main_fn, ExecutionMode::OriginUpperLeft, []);
    }

    Ok(words_to_le_bytes(&b.module().assemble()))
}

/// Builds a SPIR-V 1.6 compute shader exercising all 6 `DescriptorKind`s plus fixed and runtime arrays.
fn assemble_all_descriptors_compute_spirv(entry_name: &str) -> TestResult<Vec<u8>> {
    let mut b = Builder::new();
    b.set_version(1, 6);
    b.capability(Capability::Shader);
    b.capability(Capability::RuntimeDescriptorArray);
    b.memory_model(AddressingModel::Logical, MemoryModel::GLSL450);

    let void = b.type_void();
    let fn_ty = b.type_function(void, vec![]);
    let f32_ty = b.type_float(32, None);
    let u32_ty = b.type_int(32, 0);
    let v4f = b.type_vector(f32_ty, 4);
    let const_4 = b.constant_bit32(u32_ty, 4);

    // 1. UniformBuffer (set 0, binding 0, count 1)
    let ubo_struct = b.type_struct([v4f]);
    b.decorate(ubo_struct, Decoration::Block, []);
    b.member_decorate(
        ubo_struct,
        0,
        Decoration::Offset,
        [rspirv::dr::Operand::LiteralBit32(0)],
    );
    let ptr_ubo = b.type_pointer(None, StorageClass::Uniform, ubo_struct);
    let var_ubo = b.variable(ptr_ubo, None, StorageClass::Uniform, None);
    b.decorate(
        var_ubo,
        Decoration::DescriptorSet,
        [rspirv::dr::Operand::LiteralBit32(0)],
    );
    b.decorate(
        var_ubo,
        Decoration::Binding,
        [rspirv::dr::Operand::LiteralBit32(0)],
    );

    // 2. StorageBuffer (set 0, binding 1, count 1)
    let ssbo_struct = b.type_struct([v4f]);
    b.decorate(ssbo_struct, Decoration::Block, []);
    b.member_decorate(
        ssbo_struct,
        0,
        Decoration::Offset,
        [rspirv::dr::Operand::LiteralBit32(0)],
    );
    let ptr_ssbo = b.type_pointer(None, StorageClass::StorageBuffer, ssbo_struct);
    let var_ssbo = b.variable(ptr_ssbo, None, StorageClass::StorageBuffer, None);
    b.decorate(
        var_ssbo,
        Decoration::DescriptorSet,
        [rspirv::dr::Operand::LiteralBit32(0)],
    );
    b.decorate(
        var_ssbo,
        Decoration::Binding,
        [rspirv::dr::Operand::LiteralBit32(1)],
    );

    // 3. SampledImage runtime array (set 1, binding 0, count RUNTIME_DESCRIPTOR_COUNT)
    let sampled_img = b.type_image(f32_ty, Dim::Dim2D, 0, 0, 0, 1, ImageFormat::Unknown, None);
    let rt_arr_img = b.type_runtime_array(sampled_img);
    let ptr_rt_img = b.type_pointer(None, StorageClass::UniformConstant, rt_arr_img);
    let var_rt_img = b.variable(ptr_rt_img, None, StorageClass::UniformConstant, None);
    b.decorate(
        var_rt_img,
        Decoration::DescriptorSet,
        [rspirv::dr::Operand::LiteralBit32(1)],
    );
    b.decorate(
        var_rt_img,
        Decoration::Binding,
        [rspirv::dr::Operand::LiteralBit32(0)],
    );

    // 4. Sampler fixed array [4] (set 1, binding 1, count 4)
    let sampler_ty = b.type_sampler();
    let arr_sampler = b.type_array(sampler_ty, const_4);
    let ptr_sampler = b.type_pointer(None, StorageClass::UniformConstant, arr_sampler);
    let var_sampler = b.variable(ptr_sampler, None, StorageClass::UniformConstant, None);
    b.decorate(
        var_sampler,
        Decoration::DescriptorSet,
        [rspirv::dr::Operand::LiteralBit32(1)],
    );
    b.decorate(
        var_sampler,
        Decoration::Binding,
        [rspirv::dr::Operand::LiteralBit32(1)],
    );

    // 5. CombinedImageSampler (set 1, binding 2, count 1)
    let combined_ty = b.type_sampled_image(sampled_img);
    let ptr_combined = b.type_pointer(None, StorageClass::UniformConstant, combined_ty);
    let var_combined = b.variable(ptr_combined, None, StorageClass::UniformConstant, None);
    b.decorate(
        var_combined,
        Decoration::DescriptorSet,
        [rspirv::dr::Operand::LiteralBit32(1)],
    );
    b.decorate(
        var_combined,
        Decoration::Binding,
        [rspirv::dr::Operand::LiteralBit32(2)],
    );

    // 6. StorageImage (set 1, binding 3, count 1)
    let storage_img = b.type_image(f32_ty, Dim::Dim2D, 0, 0, 0, 2, ImageFormat::Rgba8, None);
    let ptr_storage_img = b.type_pointer(None, StorageClass::UniformConstant, storage_img);
    let var_storage_img = b.variable(ptr_storage_img, None, StorageClass::UniformConstant, None);
    b.decorate(
        var_storage_img,
        Decoration::DescriptorSet,
        [rspirv::dr::Operand::LiteralBit32(1)],
    );
    b.decorate(
        var_storage_img,
        Decoration::Binding,
        [rspirv::dr::Operand::LiteralBit32(3)],
    );

    let main_fn = b.begin_function(void, None, FunctionControl::NONE, fn_ty)?;
    let _ = b.begin_block(None)?;
    b.ret()?;
    b.end_function()?;

    b.entry_point(
        ExecutionModel::GLCompute,
        main_fn,
        entry_name,
        [
            var_ubo,
            var_ssbo,
            var_rt_img,
            var_sampler,
            var_combined,
            var_storage_img,
        ],
    );
    b.execution_mode(main_fn, ExecutionMode::LocalSize, [8, 8, 1]);

    Ok(words_to_le_bytes(&b.module().assemble()))
}

struct MockSlangCompiler {
    version_result: Result<String, CliErrorKind>,
    recorded_calls: RefCell<Vec<(PathBuf, String, ShaderStage)>>,
    fail_on_entry: Option<(String, String)>,
    emit_corrupt_bytes: Option<Vec<u8>>,
}

impl MockSlangCompiler {
    fn pinned_ok() -> Self {
        Self {
            version_result: Ok(SLANGC_VERSION.to_owned()),
            recorded_calls: RefCell::new(Vec::new()),
            fail_on_entry: None,
            emit_corrupt_bytes: None,
        }
    }
}

impl SlangCompiler for MockSlangCompiler {
    fn query_version(&self) -> Result<String, CliError> {
        match &self.version_result {
            Ok(ver) => Ok(ver.clone()),
            Err(kind) => Err(CliError::new(*kind)
                .with_file("mock-slangc")
                .with_field("slangc")
                .with_detail("mock compiler missing")),
        }
    }

    fn compile_spirv(&self, request: &SlangCompileRequest<'_>) -> Result<(), CliError> {
        self.recorded_calls.borrow_mut().push((
            request.source_path.to_path_buf(),
            request.entry_point.to_owned(),
            request.stage,
        ));

        if let Some((bad_entry, diag)) = &self.fail_on_entry
            && request.entry_point == bad_entry
        {
            return Err(CliError::new(CliErrorKind::ShaderCompilationFailed)
                .with_file(request.source_path)
                .with_field(request.entry_point)
                .with_detail(diag.clone()));
        }

        if let Some(corrupt) = &self.emit_corrupt_bytes {
            fs::write(request.spv_output_path, corrupt).map_err(|err| {
                CliError::new(CliErrorKind::ShaderReflectionFailed).with_cause(&err)
            })?;
            return Ok(());
        }

        let stem = request
            .source_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let spv_res = match (stem, request.stage) {
            ("triangle", st) => assemble_triangle_spirv(st, request.entry_point),
            ("mesh_3d", st) => assemble_mesh_3d_spirv(st, request.entry_point),
            (_, ShaderStage::Compute) => {
                assemble_all_descriptors_compute_spirv(request.entry_point)
            }
            (_, st) => assemble_triangle_spirv(st, request.entry_point),
        };
        let spv = spv_res.map_err(|err| {
            CliError::new(CliErrorKind::ShaderReflectionFailed).with_detail(err.to_string())
        })?;
        fs::write(request.spv_output_path, spv)
            .map_err(|err| CliError::new(CliErrorKind::ShaderReflectionFailed).with_cause(&err))?;
        Ok(())
    }
}

fn normalize_cli_report_location(report: &str) -> String {
    report
        .lines()
        .map(|line| {
            if let Some(rest) = line.strip_prefix("  --> ") {
                let file_only = rest
                    .rsplit_once(':')
                    .and_then(|(prefix, _col)| prefix.rsplit_once(':'))
                    .map_or(rest, |(file, _line)| file)
                    .replace('\\', "/");
                format!("  --> {file_only}:<LINE>:<COL>")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn f3d_t06_mock_compiler_cooks_shaders_and_validates_against_spawn_render() {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf();
    let tmp = TempTestDir::new("mock_cook").unwrap();
    let src_dir = tmp.path().join("shaders");
    let out_dir = tmp.path().join("out");
    fs::create_dir_all(&src_dir).unwrap();

    // Copy real Milestone 2 `.slang` sources + add a compute descriptor test shader.
    fs::copy(
        workspace_root.join("shaders/triangle.slang"),
        src_dir.join("triangle.slang"),
    )
    .unwrap();
    fs::copy(
        workspace_root.join("shaders/mesh_3d.slang"),
        src_dir.join("mesh_3d.slang"),
    )
    .unwrap();
    fs::write(
        src_dir.join("z_compute.slang"),
        r#"
        [shader("compute")]
        [numthreads(8, 8, 1)]
        void cs_main(uint3 tid : SV_DispatchThreadID) {}
        "#,
    )
    .unwrap();

    let mock = MockSlangCompiler::pinned_ok();
    let options = ShaderBuildOptions {
        source_dir: src_dir.clone(),
        out_dir: out_dir.clone(),
        slangc_path: None,
    };

    let report = build_shaders_with_compiler(&options, Path::new("slangc"), &mock).unwrap();
    assert_eq!(report.compiler_version, SLANGC_VERSION);
    assert_eq!(report.artifacts.len(), 5);

    // Ensure zero temporary `.spv` files remain in `out_dir` (`SPN-INV-084`).
    for entry in fs::read_dir(&out_dir).unwrap() {
        let p = entry.unwrap().path();
        assert_ne!(
            p.extension().and_then(|e| e.to_str()),
            Some("spv"),
            "raw .spv file leaked into out_dir: {}",
            p.display()
        );
    }

    // 1. Validate cooked `triangle` vertex + fragment pair against Phase 3's unindexed pipeline layout.
    let tri_vs = CookedShader::decode_named(
        "triangle.vs_main.spnshdr",
        &fs::read(out_dir.join("triangle.vs_main.spnshdr")).unwrap(),
    )
    .unwrap();
    let tri_fs = CookedShader::decode_named(
        "triangle.fs_main.spnshdr",
        &fs::read(out_dir.join("triangle.fs_main.spnshdr")).unwrap(),
    )
    .unwrap();
    validate_graphics_pipeline_shaders(
        &tri_vs,
        &tri_fs,
        &PipelineLayoutDesc {
            vertex_inputs: &[],
            push_constants: PushConstantRange::NONE,
            descriptor_bindings: &[],
        },
    )
    .unwrap();

    // 2. Validate cooked `mesh_3d` vertex + fragment pair against Phase 4's camera-relative mesh layout.
    let mesh_vs = CookedShader::decode_named(
        "mesh_3d.vs_main.spnshdr",
        &fs::read(out_dir.join("mesh_3d.vs_main.spnshdr")).unwrap(),
    )
    .unwrap();
    let mesh_fs = CookedShader::decode_named(
        "mesh_3d.fs_main.spnshdr",
        &fs::read(out_dir.join("mesh_3d.fs_main.spnshdr")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        mesh_vs.push_constants,
        PushConstantRange::new(128, ShaderStageFlags::VERTEX)
    );
    validate_graphics_pipeline_shaders(
        &mesh_vs,
        &mesh_fs,
        &PipelineLayoutDesc {
            vertex_inputs: &[
                VertexInputAttribute::new(0, VertexFormat::Float32x3),
                VertexInputAttribute::new(1, VertexFormat::Float32x3),
                VertexInputAttribute::new(2, VertexFormat::Float32x4),
            ],
            push_constants: PushConstantRange::new(128, ShaderStageFlags::VERTEX),
            descriptor_bindings: &[],
        },
    )
    .unwrap();

    // 3. Validate cooked compute shader reflection covering all 6 DescriptorKinds + runtime array.
    let cs = CookedShader::decode_named(
        "z_compute.cs_main.spnshdr",
        &fs::read(out_dir.join("z_compute.cs_main.spnshdr")).unwrap(),
    )
    .unwrap();
    let expected_cs_bindings = [
        DescriptorBinding {
            set: 0,
            binding: 0,
            kind: DescriptorKind::UniformBuffer,
            count: 1,
            stages: ShaderStageFlags::COMPUTE,
        },
        DescriptorBinding {
            set: 0,
            binding: 1,
            kind: DescriptorKind::StorageBuffer,
            count: 1,
            stages: ShaderStageFlags::COMPUTE,
        },
        DescriptorBinding {
            set: 1,
            binding: 0,
            kind: DescriptorKind::SampledImage,
            count: RUNTIME_DESCRIPTOR_COUNT,
            stages: ShaderStageFlags::COMPUTE,
        },
        DescriptorBinding {
            set: 1,
            binding: 1,
            kind: DescriptorKind::Sampler,
            count: 4,
            stages: ShaderStageFlags::COMPUTE,
        },
        DescriptorBinding {
            set: 1,
            binding: 2,
            kind: DescriptorKind::CombinedImageSampler,
            count: 1,
            stages: ShaderStageFlags::COMPUTE,
        },
        DescriptorBinding {
            set: 1,
            binding: 3,
            kind: DescriptorKind::StorageImage,
            count: 1,
            stages: ShaderStageFlags::COMPUTE,
        },
    ];
    cs.validate_against(&ShaderLayoutExpectation {
        stage: ShaderStage::Compute,
        vertex_inputs: &[],
        push_constants: PushConstantRange::NONE,
        descriptor_bindings: &expected_cs_bindings,
    })
    .unwrap();
}

#[test]
fn f3d_t06_version_checks_flags_and_error_diagnostics() {
    // 1. Exact version and `v`-prefixed version succeed; wrong versions fail with CLI-0003.
    assert_eq!(
        verify_slangc_version(Path::new("slangc"), SLANGC_VERSION).unwrap(),
        SLANGC_VERSION
    );
    assert_eq!(
        verify_slangc_version(Path::new("slangc"), &format!("v{SLANGC_VERSION}\n")).unwrap(),
        SLANGC_VERSION
    );
    for bad in ["", "2026.21", "2026.22-rc1", "2025.4"] {
        let err = verify_slangc_version(Path::new("slangc"), bad).unwrap_err();
        assert_eq!(err.kind(), CliErrorKind::CompilerVersionMismatch);
        assert_eq!(err.code(), "CLI-0003");
        assert_eq!(err.exit_code(), 1);
    }

    // 2. Verify exact `slangc` CLI flags: Vulkan 1.3 SPIR-V 1.6, column-major matrices,
    //    preserved entry-point name, and NEVER `-fvk-invert-y` (`SPN-INV-053`).
    let args = ProcessSlangCompiler::build_compile_args(&SlangCompileRequest {
        source_path: Path::new("shaders/mesh_3d.slang"),
        entry_point: "vs_main",
        stage: ShaderStage::Vertex,
        spv_output_path: Path::new("target/shaders/.tmp.spv"),
    });
    let args_str: Vec<_> = args
        .iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    assert!(args_str.windows(2).any(|w| w == ["-target", "spirv"]));
    assert!(args_str.windows(2).any(|w| w == ["-profile", "spirv_1_6"]));
    assert!(args_str.iter().any(|a| a == "-matrix-layout-column-major"));
    assert!(args_str.iter().any(|a| a == "-fvk-use-entrypoint-name"));
    assert!(!args_str.iter().any(|a| a.contains("invert-y")));

    // 3. CLI argument parsing (`--manifest` alias + `--out` + `--slangc`).
    let parsed = parse_cli_args([
        "spawn",
        "shaders",
        "build",
        "--manifest",
        "custom_shaders",
        "--out",
        "custom_out",
        "--slangc",
        "/opt/slang/bin/slangc",
    ])
    .unwrap();
    assert_eq!(
        parsed,
        CliCommand::ShadersBuild(ShaderBuildOptions {
            source_dir: PathBuf::from("custom_shaders"),
            out_dir: PathBuf::from("custom_out"),
            slangc_path: Some(PathBuf::from("/opt/slang/bin/slangc")),
        })
    );

    // 4. Missing compiler, wrong compiler version, compiler syntax diagnostic forwarding,
    //    and corrupt SPIR-V output handling in temp workspace.
    let tmp = TempTestDir::new("errors").unwrap();
    let src_dir = tmp.path().join("shaders");
    let out_dir = tmp.path().join("out");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(
        src_dir.join("broken.slang"),
        "[shader(\"vertex\")] float4 vs_main() : SV_Position { return float4(0, 0, 0, 1); }",
    )
    .unwrap();

    let options = ShaderBuildOptions {
        source_dir: src_dir.clone(),
        out_dir: out_dir.clone(),
        slangc_path: Some(PathBuf::from("/nonexistent/slangc-missing-bin")),
    };

    // Real process probe against nonexistent path -> CLI-0002.
    let missing_err = build_shaders(&options).unwrap_err();
    assert_eq!(missing_err.kind(), CliErrorKind::CompilerUnavailable);
    assert_eq!(missing_err.code(), "CLI-0002");

    // Wrong version -> CLI-0003 (snapshot verified).
    let wrong_ver_mock = MockSlangCompiler {
        version_result: Ok("2025.99".to_owned()),
        ..MockSlangCompiler::pinned_ok()
    };
    let ver_err =
        build_shaders_with_compiler(&options, Path::new("slangc"), &wrong_ver_mock).unwrap_err();
    insta::assert_snapshot!(
        normalize_cli_report_location(&ver_err.to_string()),
        @r#"
error[CLI-0003]: installed `slangc` version does not match repository pin
  --> tools/spawn-cli/src/shaders.rs:<LINE>:<COL>
   = file: slangc
   = field: slangc_version
   = detail: expected pinned slangc `2026.22`, found `2025.99`
"#
    );
    assert!(wrong_ver_mock.recorded_calls.borrow().is_empty());

    // Malformed shader syntax error -> CLI-0005 forwarding file/line diagnostic.
    let mut diag_mock = MockSlangCompiler::pinned_ok();
    diag_mock.fail_on_entry = Some((
        "vs_main".to_owned(),
        "stage `vertex` failed (exit code 1):\nbroken.slang(1): error 30015: undefined identifier 'bad_ident'".to_owned(),
    ));
    let compile_err = build_shaders_with_compiler(
        &ShaderBuildOptions {
            source_dir: PathBuf::from(&src_dir),
            out_dir: PathBuf::from(&out_dir),
            slangc_path: None,
        },
        Path::new("slangc"),
        &diag_mock,
    )
    .unwrap_err();
    assert_eq!(compile_err.kind(), CliErrorKind::ShaderCompilationFailed);
    assert_eq!(compile_err.code(), "CLI-0005");
    assert_eq!(compile_err.field(), Some("vs_main"));
    assert!(
        compile_err
            .detail()
            .unwrap_or("")
            .contains("broken.slang(1): error 30015: undefined identifier 'bad_ident'")
    );

    // Corrupt SPIR-V bytes emitted by compiler -> CLI-0006 and temp `.spv` cleaned up.
    let mut corrupt_mock = MockSlangCompiler::pinned_ok();
    corrupt_mock.emit_corrupt_bytes = Some(vec![0xDE, 0xAD, 0xBE, 0xEF, 1, 2, 3, 4]);
    let refl_err =
        build_shaders_with_compiler(&options, Path::new("slangc"), &corrupt_mock).unwrap_err();
    assert_eq!(refl_err.kind(), CliErrorKind::ShaderReflectionFailed);
    assert_eq!(refl_err.code(), "CLI-0006");
    for entry in fs::read_dir(&out_dir).unwrap() {
        let p = entry.unwrap().path();
        assert_ne!(p.extension().and_then(|e| e.to_str()), Some("spv"));
    }

    // Subprocess exit-code checks on compiled `spawn` binary.
    let bin = env!("CARGO_BIN_EXE_spawn");
    let help_out = Command::new(bin).arg("--help").output().unwrap();
    assert!(help_out.status.success());

    let bad_arg_out = Command::new(bin).arg("unknown-subcmd").output().unwrap();
    assert_eq!(bad_arg_out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&bad_arg_out.stderr).contains("error[CLI-0001]"));

    let missing_bin_out = Command::new(bin)
        .args([
            "shaders",
            "build",
            "--source-dir",
            src_dir.to_str().unwrap(),
            "--out",
            out_dir.to_str().unwrap(),
            "--slangc",
            "/nonexistent/slangc-missing-bin",
        ])
        .output()
        .unwrap();
    assert_eq!(missing_bin_out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing_bin_out.stderr).contains("error[CLI-0002]"));
}

/// F3D-T06 live compiler integration: runs automatically when pinned `slangc` is
/// on `PATH`, and cleanly skips in headless CI environments where `slangc` is absent.
#[test]
fn f3d_t06_live_slangc_cooks_milestone_2_shaders_when_on_path() {
    let probe = ProcessSlangCompiler::default();
    match probe.query_version() {
        Err(err) if err.kind() == CliErrorKind::CompilerUnavailable => {
            // Headless CI / stripped-PATH run (`SPN-INV-009`): mock tests above cover full pipeline.
            return;
        }
        Err(err) => panic!("unexpected slangc version query failure: {err}"),
        Ok(ver) => {
            if verify_slangc_version(Path::new("slangc"), &ver).is_err() {
                return;
            }
        }
    }

    run_live_slangc_verification().unwrap();
}

/// Explicit opt-in live `slangc` verification (`cargo test -p spawn-cli -- --ignored`):
/// fails hard if pinned `slangc` is not installed.
#[test]
#[ignore = "requires pinned slangc 2026.22 installed on PATH"]
fn f3d_t06_opt_in_pinned_slangc_required() {
    run_live_slangc_verification().unwrap();
}

fn run_live_slangc_verification() -> TestResult {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .ok_or("missing workspace root")?
        .to_path_buf();
    let tmp = TempTestDir::new("live_slangc")?;
    let out_dir = tmp.path().join("cooked");

    let report = build_shaders(&ShaderBuildOptions {
        source_dir: workspace_root.join("shaders"),
        out_dir: out_dir.clone(),
        slangc_path: None,
    })?;
    assert_eq!(report.compiler_version, SLANGC_VERSION);
    assert_eq!(report.artifacts.len(), 4);

    let tri_vs = CookedShader::decode(&fs::read(out_dir.join("triangle.vs_main.spnshdr"))?)?;
    let tri_fs = CookedShader::decode(&fs::read(out_dir.join("triangle.fs_main.spnshdr"))?)?;
    validate_graphics_pipeline_shaders(
        &tri_vs,
        &tri_fs,
        &PipelineLayoutDesc {
            vertex_inputs: &[],
            push_constants: PushConstantRange::NONE,
            descriptor_bindings: &[],
        },
    )?;

    let mesh_vs = CookedShader::decode(&fs::read(out_dir.join("mesh_3d.vs_main.spnshdr"))?)?;
    let mesh_fs = CookedShader::decode(&fs::read(out_dir.join("mesh_3d.fs_main.spnshdr"))?)?;
    validate_graphics_pipeline_shaders(
        &mesh_vs,
        &mesh_fs,
        &PipelineLayoutDesc {
            vertex_inputs: &[
                VertexInputAttribute::new(0, VertexFormat::Float32x3),
                VertexInputAttribute::new(1, VertexFormat::Float32x3),
                VertexInputAttribute::new(2, VertexFormat::Float32x4),
            ],
            push_constants: PushConstantRange::new(128, ShaderStageFlags::VERTEX),
            descriptor_bindings: &[],
        },
    )?;

    // Also test real compiler diagnostic forwarding on a broken `.slang` file!
    let bad_src_dir = tmp.path().join("bad_shaders");
    fs::create_dir_all(&bad_src_dir)?;
    fs::write(
        bad_src_dir.join("syntax_err.slang"),
        "[shader(\"vertex\")]\nfloat4 vs_main() : SV_Position {\n    return undeclared_symbol;\n}\n",
    )?;
    let Err(err) = build_shaders(&ShaderBuildOptions {
        source_dir: bad_src_dir,
        out_dir: tmp.path().join("bad_out"),
        slangc_path: None,
    }) else {
        return Err("expected syntax_err.slang to fail compilation".into());
    };
    assert_eq!(err.kind(), CliErrorKind::ShaderCompilationFailed);
    assert!(err.detail().unwrap_or("").contains("syntax_err.slang"));
    assert!(err.detail().unwrap_or("").contains("undeclared_symbol"));

    // Live `slangc` cook of a rich shader exercising `float3x3 mats[2]` push constants
    // (`92` B SPIR-V static end vs `96` B std430/CPU layout), comma-separated compute
    // attributes (`[shader("compute"), numthreads(8, 8, 1)]`), `[[shader("vertex")]]`,
    // and all 6 descriptor kinds (including fixed `SamplerState[4]` and runtime `Texture2D[]`).
    let rich_src_dir = tmp.path().join("rich_shaders");
    let rich_out_dir = tmp.path().join("rich_out");
    fs::create_dir_all(&rich_src_dir)?;
    fs::write(
        rich_src_dir.join("descriptors_and_mat_array.slang"),
        r#"
struct PushMats {
    float3x3 mats[2];
};
[[vk::push_constant]] PushMats pc;

struct CamBlock {
    float4 tint;
};

[[vk::binding(0, 0)]] ConstantBuffer<CamBlock> u_cam;
[[vk::binding(1, 0)]] StructuredBuffer<float4> s_items;
[[vk::binding(2, 0)]] Texture2D<float4> t_tex;
[[vk::binding(3, 0)]] SamplerState s_samplers[4];
[[vk::binding(4, 0)]] Sampler2D t_combined;
[[vk::binding(5, 0)]] RWTexture2D<float4> u_storage_img;
[[vk::binding(0, 1)]] Texture2D<float4> t_bindless[];

[[shader("vertex")]]
float4 vs_mats(float3 pos : POSITION) : SV_Position {
    float3 p = mul(pc.mats[1], mul(pc.mats[0], pos));
    return float4(p, 1.0);
}

[shader("compute"), numthreads(8, 8, 1)]
void cs_resources(uint3 tid : SV_DispatchThreadID) {
    float4 a = u_cam.tint + s_items[tid.x];
    float4 b = t_tex.SampleLevel(s_samplers[1], float2(0.5, 0.5), 0.0);
    float4 c = t_combined.SampleLevel(float2(0.25, 0.25), 0.0);
    float4 d = t_bindless[tid.x].SampleLevel(s_samplers[0], float2(0.1, 0.1), 0.0);
    u_storage_img[tid.xy] = a + b + c + d;
}
"#,
    )?;

    let rich_report = build_shaders(&ShaderBuildOptions {
        source_dir: rich_src_dir,
        out_dir: rich_out_dir.clone(),
        slangc_path: None,
    })?;
    assert_eq!(rich_report.artifacts.len(), 2);

    let vs_mats = CookedShader::decode(&fs::read(
        rich_out_dir.join("descriptors_and_mat_array.vs_mats.spnshdr"),
    )?)?;
    assert_eq!(vs_mats.stage, ShaderStage::Vertex);
    assert_eq!(vs_mats.push_constants.size, 92);
    assert_eq!(vs_mats.push_constants.stages, ShaderStageFlags::VERTEX);
    vs_mats.validate_against(&ShaderLayoutExpectation {
        stage: ShaderStage::Vertex,
        vertex_inputs: &[VertexInputAttribute::new(0, VertexFormat::Float32x3)],
        push_constants: PushConstantRange::new(96, ShaderStageFlags::ALL_GRAPHICS),
        descriptor_bindings: &[],
    })?;

    let cs_res = CookedShader::decode(&fs::read(
        rich_out_dir.join("descriptors_and_mat_array.cs_resources.spnshdr"),
    )?)?;
    assert_eq!(cs_res.stage, ShaderStage::Compute);
    let expected_cs_bindings = [
        DescriptorBinding {
            set: 0,
            binding: 0,
            kind: DescriptorKind::UniformBuffer,
            count: 1,
            stages: ShaderStageFlags::COMPUTE,
        },
        DescriptorBinding {
            set: 0,
            binding: 1,
            kind: DescriptorKind::StorageBuffer,
            count: 1,
            stages: ShaderStageFlags::COMPUTE,
        },
        DescriptorBinding {
            set: 0,
            binding: 2,
            kind: DescriptorKind::SampledImage,
            count: 1,
            stages: ShaderStageFlags::COMPUTE,
        },
        DescriptorBinding {
            set: 0,
            binding: 3,
            kind: DescriptorKind::Sampler,
            count: 4,
            stages: ShaderStageFlags::COMPUTE,
        },
        DescriptorBinding {
            set: 0,
            binding: 4,
            kind: DescriptorKind::CombinedImageSampler,
            count: 1,
            stages: ShaderStageFlags::COMPUTE,
        },
        DescriptorBinding {
            set: 0,
            binding: 5,
            kind: DescriptorKind::StorageImage,
            count: 1,
            stages: ShaderStageFlags::COMPUTE,
        },
        DescriptorBinding {
            set: 1,
            binding: 0,
            kind: DescriptorKind::SampledImage,
            count: RUNTIME_DESCRIPTOR_COUNT,
            stages: ShaderStageFlags::COMPUTE,
        },
    ];
    assert_eq!(cs_res.descriptor_bindings, expected_cs_bindings);
    cs_res.validate_against(&ShaderLayoutExpectation {
        stage: ShaderStage::Compute,
        vertex_inputs: &[],
        push_constants: PushConstantRange::NONE,
        descriptor_bindings: &expected_cs_bindings,
    })?;

    Ok(())
}

/// Regression coverage for SPIR-V push-constant array-of-matrices (`float3x3[2]`),
/// negative signed constants, bad/overflowing `ArrayStride`/`MatrixStride`, and
/// duplicate `OpEntryPoint` rejection.
#[test]
fn f3d_t06_spirv_reflection_matrix_arrays_negative_constants_and_duplicate_entries() -> TestResult {
    struct SingleBlobCompiler(Vec<u8>);
    impl SlangCompiler for SingleBlobCompiler {
        fn query_version(&self) -> Result<String, CliError> {
            Ok(SLANGC_VERSION.to_owned())
        }
        fn compile_spirv(&self, req: &SlangCompileRequest<'_>) -> Result<(), CliError> {
            fs::write(req.spv_output_path, &self.0)
                .map_err(|e| CliError::new(CliErrorKind::ShaderCompilationFailed).with_cause(&e))
        }
    }

    let tmp = TempTestDir::new("refl_edge_cases")?;
    let src_dir = tmp.path().join("src");
    let out_dir = tmp.path().join("out");
    fs::create_dir_all(&src_dir)?;
    fs::write(
        src_dir.join("test.slang"),
        "[shader(\"vertex\")]\nfloat4 vs_main(float3 p : POSITION) : SV_Position { return float4(p, 1.0); }\n",
    )?;
    let opts = ShaderBuildOptions {
        source_dir: src_dir,
        out_dir: out_dir.clone(),
        slangc_path: None,
    };

    // 1. `struct PC { float3x3 mats[2]; }` (`ArrayStride 48`, member `RowMajor` + `MatrixStride 16`)
    //    must reflect `92` bytes (`48 + (2 * 16 + 12)`), NOT `84`.
    {
        let mut b = Builder::new();
        b.set_version(1, 6);
        b.capability(Capability::Shader);
        b.memory_model(AddressingModel::Logical, MemoryModel::GLSL450);
        let void = b.type_void();
        let fn_ty = b.type_function(void, vec![]);
        let f32_ty = b.type_float(32, None);
        let i32_ty = b.type_int(32, 1);
        let const_2 = b.constant_bit32(i32_ty, 2);
        let v3f = b.type_vector(f32_ty, 3);
        let v4f = b.type_vector(f32_ty, 4);
        let mat3 = b.type_matrix(v3f, 3);
        let arr_mat3 = b.type_array(mat3, const_2);
        b.decorate(
            arr_mat3,
            Decoration::ArrayStride,
            [rspirv::dr::Operand::LiteralBit32(48)],
        );
        let pc_struct = b.type_struct([arr_mat3]);
        b.decorate(pc_struct, Decoration::Block, []);
        b.member_decorate(
            pc_struct,
            0,
            Decoration::Offset,
            [rspirv::dr::Operand::LiteralBit32(0)],
        );
        b.member_decorate(pc_struct, 0, Decoration::RowMajor, []);
        b.member_decorate(
            pc_struct,
            0,
            Decoration::MatrixStride,
            [rspirv::dr::Operand::LiteralBit32(16)],
        );
        let ptr_pc = b.type_pointer(None, StorageClass::PushConstant, pc_struct);
        let pc_var = b.variable(ptr_pc, None, StorageClass::PushConstant, None);
        let ptr_out = b.type_pointer(None, StorageClass::Output, v4f);
        let gl_pos = b.variable(ptr_out, None, StorageClass::Output, None);
        b.decorate(
            gl_pos,
            Decoration::BuiltIn,
            [rspirv::dr::Operand::BuiltIn(BuiltIn::Position)],
        );
        let f = b.begin_function(void, None, FunctionControl::NONE, fn_ty)?;
        let _ = b.begin_block(None)?;
        b.ret()?;
        b.end_function()?;
        b.entry_point(ExecutionModel::Vertex, f, "vs_main", [pc_var, gl_pos]);

        let bytes = words_to_le_bytes(&b.module().assemble());
        build_shaders_with_compiler(&opts, Path::new("slangc"), &SingleBlobCompiler(bytes))?;
        let cooked = CookedShader::decode(&fs::read(out_dir.join("test.vs_main.spnshdr"))?)?;
        assert_eq!(cooked.push_constants.size, 92);
        cooked.validate_against(&ShaderLayoutExpectation {
            stage: ShaderStage::Vertex,
            vertex_inputs: &[],
            push_constants: PushConstantRange::new(96, ShaderStageFlags::VERTEX),
            descriptor_bindings: &[],
        })?;
    }

    // 2. Negative signed constant (`OpConstant %int -2`), shrunken `ArrayStride`,
    //    shrunken/overflowing `MatrixStride`, and duplicate `OpEntryPoint` all return `CLI-0006`.
    let build_bad_case = |case: u8| -> TestResult<Vec<u8>> {
        let mut b = Builder::new();
        b.set_version(1, 6);
        b.capability(Capability::Shader);
        b.memory_model(AddressingModel::Logical, MemoryModel::GLSL450);
        let void = b.type_void();
        let fn_ty = b.type_function(void, vec![]);
        let f32_ty = b.type_float(32, None);
        let i32_ty = b.type_int(32, 1);
        let v3f = b.type_vector(f32_ty, 3);
        let v4f = b.type_vector(f32_ty, 4);
        let mat3 = b.type_matrix(v3f, 3);

        let len_const = if case == 0 {
            b.constant_bit32(i32_ty, (-2i32) as u32)
        } else {
            b.constant_bit32(i32_ty, 2)
        };
        let arr_stride = if case == 1 { 32 } else { 48 };
        let mat_stride = match case {
            2 => 8,
            3 => u32::MAX,
            _ => 16,
        };

        let arr_mat3 = b.type_array(mat3, len_const);
        b.decorate(
            arr_mat3,
            Decoration::ArrayStride,
            [rspirv::dr::Operand::LiteralBit32(arr_stride)],
        );
        let pc_struct = b.type_struct([arr_mat3]);
        b.decorate(pc_struct, Decoration::Block, []);
        b.member_decorate(
            pc_struct,
            0,
            Decoration::Offset,
            [rspirv::dr::Operand::LiteralBit32(0)],
        );
        b.member_decorate(pc_struct, 0, Decoration::RowMajor, []);
        b.member_decorate(
            pc_struct,
            0,
            Decoration::MatrixStride,
            [rspirv::dr::Operand::LiteralBit32(mat_stride)],
        );
        let ptr_pc = b.type_pointer(None, StorageClass::PushConstant, pc_struct);
        let pc_var = b.variable(ptr_pc, None, StorageClass::PushConstant, None);
        let ptr_out = b.type_pointer(None, StorageClass::Output, v4f);
        let gl_pos = b.variable(ptr_out, None, StorageClass::Output, None);
        b.decorate(
            gl_pos,
            Decoration::BuiltIn,
            [rspirv::dr::Operand::BuiltIn(BuiltIn::Position)],
        );
        let f = b.begin_function(void, None, FunctionControl::NONE, fn_ty)?;
        let _ = b.begin_block(None)?;
        b.ret()?;
        b.end_function()?;
        b.entry_point(ExecutionModel::Vertex, f, "vs_main", [pc_var, gl_pos]);
        if case == 4 {
            b.entry_point(ExecutionModel::Vertex, f, "vs_main", [pc_var, gl_pos]);
        }
        Ok(words_to_le_bytes(&b.module().assemble()))
    };

    for (case, expected_substr) in [
        (0u8, "negative signed constant"),
        (1, "ArrayStride"),
        (2, "MatrixStride"),
        (3, "overflow"),
        (4, "duplicate OpEntryPoint"),
    ] {
        let bytes = build_bad_case(case)?;
        let Err(err) =
            build_shaders_with_compiler(&opts, Path::new("slangc"), &SingleBlobCompiler(bytes))
        else {
            return Err(format!("expected bad SPIR-V case {case} to fail reflection").into());
        };
        assert_eq!(err.kind(), CliErrorKind::ShaderReflectionFailed);
        assert!(
            err.detail().unwrap_or("").contains(expected_substr),
            "case {case}: expected detail containing `{expected_substr}`, got `{:?}`",
            err.detail()
        );
    }

    Ok(())
}

/// Phase 2 slice of `F3D-T13`: verifies `spawn-cli` headless dependency tree,
/// `#![forbid(unsafe_code)]`, zero `build.rs` shader compiler invocations, zero
/// git-tracked `.spv`/`.spnshdr` blobs, and genre/backend-clean public APIs.
#[test]
fn f3d_t13_shader_toolchain_headless_tree_build_rs_and_uncommitted_spv_audit() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf();

    // 1. `cargo tree -p spawn-cli -e normal` pulls zero window/GPU backends or unapproved crates.
    let tree_out = Command::new(env!("CARGO"))
        .current_dir(&root)
        .env("CARGO_TERM_COLOR", "never")
        .args([
            "tree",
            "--color",
            "never",
            "-p",
            "spawn-cli",
            "-e",
            "normal",
        ])
        .output()
        .unwrap();
    assert!(tree_out.status.success());
    let tree = String::from_utf8(tree_out.stdout).unwrap();
    assert!(
        !tree.contains('\x1b'),
        "cargo tree emitted unexpected ANSI escape sequences:\n{tree}"
    );
    let crate_names: std::collections::BTreeSet<String> = tree
        .lines()
        .filter_map(|line| {
            let trimmed =
                line.trim_start_matches([' ', '│', '├', '└', '─', '┬', '|', '+', '`', '\\', '-']);
            if trimmed.is_empty() || trimmed.starts_with('[') {
                return None;
            }
            trimmed.split_whitespace().next().map(str::to_owned)
        })
        .collect();
    for forbidden in [
        "ash",
        "bytemuck",
        "wayland-client",
        "wayland-protocols",
        "raw-window-handle",
        "rustix",
        "windows-sys",
        "x11rb",
        "spawn-platform",
        "spawn-input",
        "serde_json",
        "tempfile",
        "thiserror",
        "anyhow",
    ] {
        assert!(
            !crate_names.contains(forbidden),
            "`spawn-cli` normal dependency tree must not contain `{forbidden}`:\n{tree}"
        );
    }

    // 2. Zero `build.rs` files invoking `slangc` (`SPN-INV-084`).
    let mut candidate_build_scripts = vec![root.join("build.rs")];
    for group in ["crates", "tools"] {
        for entry in fs::read_dir(root.join(group)).unwrap() {
            candidate_build_scripts.push(entry.unwrap().path().join("build.rs"));
        }
    }
    for script in candidate_build_scripts {
        if script.is_file() {
            let content = fs::read_to_string(&script).unwrap();
            assert!(
                !content.contains("slangc"),
                "`{}` must never invoke `slangc` (SPN-INV-084)",
                script.display()
            );
        }
    }

    // 3. Zero `.spv` or `.spnshdr` files tracked in git (`SPN-INV-084`, `F3D-INV-11`).
    let ls_out = Command::new("git")
        .current_dir(&root)
        .args(["ls-files"])
        .output()
        .unwrap();
    assert!(ls_out.status.success());
    let tracked = String::from_utf8(ls_out.stdout).unwrap();
    for file in tracked.lines() {
        assert!(
            !file.ends_with(".spv") && !file.ends_with(".spnshdr"),
            "compiled shader artifact `{file}` must never be committed to git (ADR-0014)"
        );
    }

    // 4. `#![forbid(unsafe_code)]` enforced on `spawn-cli` (`F3D-NFR-05`).
    for rel in ["tools/spawn-cli/src/lib.rs", "tools/spawn-cli/src/main.rs"] {
        let src = fs::read_to_string(root.join(rel)).unwrap();
        assert!(
            src.contains("#![forbid(unsafe_code)]"),
            "`{rel}` must declare `#![forbid(unsafe_code)]`"
        );
    }

    // 5. Public API of `spawn-cli` leaks zero `clap::` / `rspirv::` types and zero genre terms.
    for entry in fs::read_dir(root.join("tools/spawn-cli/src")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let content = fs::read_to_string(&path).unwrap();
        for (idx, raw_line) in content.lines().enumerate() {
            let code = raw_line.split("//").next().unwrap_or("").trim();
            if code.starts_with("#[cfg(test)]") {
                break;
            }
            if code.starts_with("pub ") && !code.starts_with("pub mod ") {
                for leaked in ["clap::", "rspirv::", "ash::", "vk::", "bytemuck::"] {
                    assert!(
                        !code.contains(leaked),
                        "{}:{} leaks third-party type `{leaked}` in public item: `{code}`",
                        path.display(),
                        idx + 1
                    );
                }
            }
        }
    }
}
