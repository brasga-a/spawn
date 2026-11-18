//! Headless verification, `proptest` fuzzing, and `insta` error-report snapshot
//! tests for `spawn-render`'s cooked-shader binary parser and layout validator
//! (`docs/first-3d/VERIFICATION.md`: `F3D-T07`, `F3D-FR-05`, `F3D-FR-12`,
//! `F3D-INV-11`, `F3D-INV-12`, `F3D-INV-15`, Gate G2).

#![forbid(unsafe_code)]

use arbitrary::Unstructured;
use proptest::prelude::*;
use spawn_render::{
    CookedShader, DescriptorBinding, DescriptorKind, MAX_COOKED_SHADER_BYTES,
    MAX_DESCRIPTOR_BINDINGS, MAX_ENTRY_POINT_BYTES, MAX_PUSH_CONSTANT_BYTES, MAX_SPIRV_WORDS,
    MAX_VERTEX_ATTRIBUTES, PipelineLayoutDesc, PushConstantRange, RUNTIME_DESCRIPTOR_COUNT,
    RenderError, RenderErrorKind, SHADER_FORMAT_VERSION, SHADER_MAGIC, SPIRV_MAGIC,
    ShaderLayoutExpectation, ShaderStage, ShaderStageFlags, VertexFormat, VertexInputAttribute,
    validate_graphics_pipeline_shaders,
};

fn pack_spirv_string(s: &str) -> Vec<u32> {
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

fn spirv_execution_model(stage: ShaderStage) -> u32 {
    match stage {
        ShaderStage::Vertex => 0,
        ShaderStage::Fragment => 4,
        ShaderStage::Compute => 5,
    }
}

fn synth_spirv(stage: ShaderStage, entry_point: &str) -> Vec<u32> {
    let name_words = pack_spirv_string(entry_point);
    let op_entry_len = (3 + name_words.len()) as u32;
    let mut words = vec![
        SPIRV_MAGIC,
        0x0001_0600,    // SPIR-V 1.6
        0x0000_0000,    // generator
        16,             // id bound
        0,              // schema
        (2 << 16) | 17, // OpCapability Shader
        1,
        (3 << 16) | 14, // OpMemoryModel Logical GLSL450
        0,
        1,
        (op_entry_len << 16) | 15, // OpEntryPoint
        spirv_execution_model(stage),
        4,
    ];
    words.extend_from_slice(&name_words);
    words
}

fn sample_mesh_vertex_shader() -> CookedShader {
    CookedShader {
        version: SHADER_FORMAT_VERSION,
        stage: ShaderStage::Vertex,
        entry_point: "vs_main".to_owned(),
        push_constants: PushConstantRange::new(128, ShaderStageFlags::VERTEX),
        descriptor_bindings: vec![DescriptorBinding {
            set: 0,
            binding: 0,
            kind: DescriptorKind::UniformBuffer,
            count: 1,
            stages: ShaderStageFlags::VERTEX,
        }],
        vertex_inputs: vec![
            VertexInputAttribute::new(0, VertexFormat::Float32x3),
            VertexInputAttribute::new(1, VertexFormat::Float32x3),
            VertexInputAttribute::new(2, VertexFormat::Float32x4),
        ],
        spirv_words: synth_spirv(ShaderStage::Vertex, "vs_main"),
    }
}

fn sample_mesh_fragment_shader() -> CookedShader {
    CookedShader {
        version: SHADER_FORMAT_VERSION,
        stage: ShaderStage::Fragment,
        entry_point: "fs_main".to_owned(),
        push_constants: PushConstantRange::NONE,
        descriptor_bindings: vec![
            DescriptorBinding {
                set: 0,
                binding: 0,
                kind: DescriptorKind::UniformBuffer,
                count: 1,
                stages: ShaderStageFlags::FRAGMENT,
            },
            DescriptorBinding {
                set: 1,
                binding: 0,
                kind: DescriptorKind::SampledImage,
                count: RUNTIME_DESCRIPTOR_COUNT,
                stages: ShaderStageFlags::FRAGMENT,
            },
        ],
        vertex_inputs: Vec::new(),
        spirv_words: synth_spirv(ShaderStage::Fragment, "fs_main"),
    }
}

/// Normalizes the caller line/column in `RenderError`'s `-->` header so snapshot
/// assertions verify the exact ADR-0009 report structure without breaking when
/// test lines below shift.
fn normalize_report_location(report: &str) -> String {
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
fn f3d_t07_valid_cooked_shaders_roundtrip_and_match_pipeline_layout() {
    let vs = sample_mesh_vertex_shader();
    let fs = sample_mesh_fragment_shader();

    let vs_bytes = vs.encode().unwrap();
    let fs_bytes = fs.encode().unwrap();
    assert_eq!(vs_bytes.len() % 4, 0);
    assert_eq!(fs_bytes.len() % 4, 0);

    let decoded_vs = CookedShader::decode_named("mesh_3d.vs_main.spnshdr", &vs_bytes).unwrap();
    let decoded_fs = CookedShader::decode_named("mesh_3d.fs_main.spnshdr", &fs_bytes).unwrap();
    assert_eq!(decoded_vs, vs);
    assert_eq!(decoded_fs, fs);

    let layout = PipelineLayoutDesc {
        vertex_inputs: &[
            VertexInputAttribute::new(2, VertexFormat::Float32x4),
            VertexInputAttribute::new(0, VertexFormat::Float32x3),
            VertexInputAttribute::new(1, VertexFormat::Float32x3),
        ],
        push_constants: PushConstantRange::new(128, ShaderStageFlags::VERTEX),
        descriptor_bindings: &[
            DescriptorBinding {
                set: 0,
                binding: 0,
                kind: DescriptorKind::UniformBuffer,
                count: 1,
                stages: ShaderStageFlags::ALL_GRAPHICS,
            },
            DescriptorBinding {
                set: 1,
                binding: 0,
                kind: DescriptorKind::SampledImage,
                count: RUNTIME_DESCRIPTOR_COUNT,
                stages: ShaderStageFlags::FRAGMENT,
            },
        ],
    };

    validate_graphics_pipeline_shaders(&decoded_vs, &decoded_fs, &layout).unwrap();

    // Shared `ALL_GRAPHICS` push-constant range (`fs_main` uses 0 B) must succeed in both
    // `validate_graphics_pipeline_shaders` AND per-stage `validate_against`.
    let shared_pc = PushConstantRange::new(128, ShaderStageFlags::ALL_GRAPHICS);
    let shared_layout = PipelineLayoutDesc {
        push_constants: shared_pc,
        ..layout
    };
    validate_graphics_pipeline_shaders(&decoded_vs, &decoded_fs, &shared_layout).unwrap();
    decoded_vs
        .validate_against(&ShaderLayoutExpectation {
            stage: ShaderStage::Vertex,
            vertex_inputs: shared_layout.vertex_inputs,
            push_constants: shared_pc,
            descriptor_bindings: shared_layout.descriptor_bindings,
        })
        .unwrap();
    decoded_fs
        .validate_against(&ShaderLayoutExpectation {
            stage: ShaderStage::Fragment,
            vertex_inputs: &[],
            push_constants: shared_pc,
            descriptor_bindings: shared_layout.descriptor_bindings,
        })
        .unwrap();

    // Tail-padded std430 / `#[repr(C)]` push-constant structs: e.g. `Mat4 + float3x3`
    // has SPIR-V `max_end = 108` B while CPU/std430 struct size is `112` B (`92` vs `96`
    // for `float3x3[2]`). Pipeline sizes `>= shader.size` succeed; `< shader.size` fail.
    let mut vs_108 = decoded_vs.clone();
    vs_108.push_constants = PushConstantRange::new(108, ShaderStageFlags::VERTEX);
    validate_graphics_pipeline_shaders(
        &vs_108,
        &decoded_fs,
        &PipelineLayoutDesc {
            push_constants: PushConstantRange::new(112, ShaderStageFlags::ALL_GRAPHICS),
            ..layout
        },
    )
    .unwrap();
    vs_108
        .validate_against(&ShaderLayoutExpectation {
            stage: ShaderStage::Vertex,
            vertex_inputs: layout.vertex_inputs,
            push_constants: PushConstantRange::new(112, ShaderStageFlags::VERTEX),
            descriptor_bindings: layout.descriptor_bindings,
        })
        .unwrap();
    let too_small_err = vs_108
        .validate_against(&ShaderLayoutExpectation {
            stage: ShaderStage::Vertex,
            vertex_inputs: layout.vertex_inputs,
            push_constants: PushConstantRange::new(104, ShaderStageFlags::VERTEX),
            descriptor_bindings: layout.descriptor_bindings,
        })
        .unwrap_err();
    assert_eq!(too_small_err.kind(), RenderErrorKind::ShaderLayoutMismatch);
}

#[test]
fn f3d_t07_rejects_corrupt_truncated_unaligned_and_overflow_headers() {
    let valid_bytes = sample_mesh_vertex_shader().encode().unwrap();

    // 1. Empty and truncated headers.
    let err = CookedShader::decode(&[]).unwrap_err();
    assert_eq!(err.kind(), RenderErrorKind::InvalidShaderArtifact);
    assert_eq!(err.code(), "REND-0010");

    let err = CookedShader::decode(&valid_bytes[..36]).unwrap_err();
    assert_eq!(err.kind(), RenderErrorKind::InvalidShaderArtifact);

    // 2. Non-4-byte-aligned byte lengths.
    for extra in 1..=3 {
        let mut unaligned = valid_bytes.clone();
        unaligned.extend(std::iter::repeat_n(0, extra));
        let err = CookedShader::decode(&unaligned).unwrap_err();
        assert_eq!(err.kind(), RenderErrorKind::InvalidShaderArtifact);
        assert!(err.detail().unwrap_or("").contains("4-byte aligned"));
    }

    // 3. Oversized payload exceeding MAX_COOKED_SHADER_BYTES rejected up front.
    let oversized = vec![0u8; MAX_COOKED_SHADER_BYTES + 4];
    let err = CookedShader::decode(&oversized).unwrap_err();
    assert_eq!(err.kind(), RenderErrorKind::InvalidShaderArtifact);
    assert!(err.detail().unwrap_or("").contains("exceeds maximum"));

    // 4. Bad header magic.
    let mut bad_magic = valid_bytes.clone();
    bad_magic[0..8].copy_from_slice(b"BADMAGIC");
    let err = CookedShader::decode(&bad_magic).unwrap_err();
    assert_eq!(err.kind(), RenderErrorKind::InvalidShaderArtifact);
    assert!(err.detail().unwrap_or("").contains("magic"));

    // 5. Unsupported format versions (0, 2, u32::MAX).
    for bad_ver in [0u32, 2, u32::MAX] {
        let mut buf = valid_bytes.clone();
        buf[8..12].copy_from_slice(&bad_ver.to_le_bytes());
        let err = CookedShader::decode(&buf).unwrap_err();
        assert_eq!(err.kind(), RenderErrorKind::InvalidShaderArtifact);
        assert!(err.detail().unwrap_or("").contains("version"));
    }

    // 6. Invalid stage discriminant.
    for bad_stage in [0u32, 4, u32::MAX] {
        let mut buf = valid_bytes.clone();
        buf[12..16].copy_from_slice(&bad_stage.to_le_bytes());
        let err = CookedShader::decode(&buf).unwrap_err();
        assert_eq!(err.kind(), RenderErrorKind::InvalidShaderArtifact);
        assert!(err.detail().unwrap_or("").contains("stage"));
    }

    // 7. Out-of-range or u32::MAX counts rejected BEFORE allocation (SPN-INV-034).
    for (offset, bad_count) in [
        (16usize, 0u32),
        (16, (MAX_ENTRY_POINT_BYTES as u32) + 1),
        (16, u32::MAX),
        (28, (MAX_DESCRIPTOR_BINDINGS as u32) + 1),
        (28, u32::MAX),
        (32, (MAX_VERTEX_ATTRIBUTES as u32) + 1),
        (32, u32::MAX),
        (36, 4),
        (36, (MAX_SPIRV_WORDS as u32) + 1),
        (36, u32::MAX),
        // Within MAX_SPIRV_WORDS cap, but buffer is tiny: must detect truncation without allocating 2 MiB!
        (36, 500_000),
    ] {
        let mut buf = valid_bytes.clone();
        buf[offset..offset + 4].copy_from_slice(&bad_count.to_le_bytes());
        let err = CookedShader::decode(&buf).unwrap_err();
        assert_eq!(err.kind(), RenderErrorKind::InvalidShaderArtifact);
    }

    // 8. Trailing 4-byte-aligned garbage rejected.
    let mut trailing = valid_bytes.clone();
    trailing.extend_from_slice(&[0, 0, 0, 0]);
    let err = CookedShader::decode(&trailing).unwrap_err();
    assert_eq!(err.kind(), RenderErrorKind::InvalidShaderArtifact);
    assert!(err.detail().unwrap_or("").contains("trailing bytes"));

    // 9. Non-zero entry-point padding byte (`"vs_main"` is 7 bytes + 1 padding byte at offset 47).
    let mut bad_pad = valid_bytes.clone();
    bad_pad[47] = 0xFF;
    let err = CookedShader::decode(&bad_pad).unwrap_err();
    assert_eq!(err.kind(), RenderErrorKind::InvalidShaderArtifact);
    assert!(err.detail().unwrap_or("").contains("padding"));

    // 10. Invalid UTF-8 in entry point.
    let mut bad_utf8 = valid_bytes;
    bad_utf8[40] = 0xFF;
    let err = CookedShader::decode(&bad_utf8).unwrap_err();
    assert_eq!(err.kind(), RenderErrorKind::InvalidShaderArtifact);
    assert!(err.detail().unwrap_or("").contains("UTF-8"));
}

#[test]
fn f3d_t07_rejects_corrupt_spirv_and_missing_entry_points() {
    // 1. Byte-swapped SPIR-V magic.
    {
        let mut shader = sample_mesh_vertex_shader();
        shader.spirv_words[0] = 0x0302_2307;
        let err = shader.validate().unwrap_err();
        assert_eq!(err.kind(), RenderErrorKind::InvalidShaderArtifact);
        assert!(err.detail().unwrap_or("").contains("byte-swapped"));
    }

    // 2. Bad SPIR-V version (1.7) and non-zero schema.
    {
        let mut shader = sample_mesh_vertex_shader();
        shader.spirv_words[1] = 0x0001_0700;
        assert_eq!(
            shader.validate().unwrap_err().kind(),
            RenderErrorKind::InvalidShaderArtifact
        );

        let mut shader2 = sample_mesh_vertex_shader();
        shader2.spirv_words[4] = 1;
        assert_eq!(
            shader2.validate().unwrap_err().kind(),
            RenderErrorKind::InvalidShaderArtifact
        );
    }

    // 3. Malformed instruction word count == 0 and truncated instruction.
    {
        let mut zero_wc = sample_mesh_vertex_shader();
        zero_wc.spirv_words.push(17); // word_count = 0, opcode = 17
        let err = zero_wc.validate().unwrap_err();
        assert_eq!(err.kind(), RenderErrorKind::InvalidShaderArtifact);
        assert!(err.detail().unwrap_or("").contains("word count 0"));

        let mut trunc = sample_mesh_vertex_shader();
        trunc.spirv_words.push((5 << 16) | 17); // claims 5 words, 1 remaining
        let err = trunc.validate().unwrap_err();
        assert_eq!(err.kind(), RenderErrorKind::InvalidShaderArtifact);
        assert!(err.detail().unwrap_or("").contains("truncated SPIR-V"));
    }

    // 4. Missing entry point in SPIR-V (header says `vs_main`, SPIR-V has `main` or wrong stage).
    {
        let mut wrong_name = sample_mesh_vertex_shader();
        wrong_name.spirv_words = synth_spirv(ShaderStage::Vertex, "main");
        let err = wrong_name.validate().unwrap_err();
        assert_eq!(err.kind(), RenderErrorKind::InvalidShaderArtifact);
        assert!(
            err.detail()
                .unwrap_or("")
                .contains("no `vertex` OpEntryPoint named `vs_main`")
        );

        let mut wrong_stage_spv = sample_mesh_vertex_shader();
        wrong_stage_spv.spirv_words = synth_spirv(ShaderStage::Fragment, "vs_main");
        let err = wrong_stage_spv.validate().unwrap_err();
        assert_eq!(err.kind(), RenderErrorKind::InvalidShaderArtifact);

        // Duplicate `OpEntryPoint` with identical `(ExecutionModel, Name)` is forbidden
        // by SPIR-V 1.6 §2.4 and rejected with `REND-0010`, while distinct stages sharing
        // a name (`Vertex "main"` + `Fragment "main"`) remain valid.
        let mut dup_ep = sample_mesh_vertex_shader();
        let name_words = pack_spirv_string("vs_main");
        let op_entry_len = (3 + name_words.len()) as u32;
        dup_ep.spirv_words.push((op_entry_len << 16) | 15);
        dup_ep
            .spirv_words
            .push(spirv_execution_model(ShaderStage::Vertex));
        dup_ep.spirv_words.push(4);
        dup_ep.spirv_words.extend_from_slice(&name_words);
        let dup_err = dup_ep.validate().unwrap_err();
        assert_eq!(dup_err.kind(), RenderErrorKind::InvalidShaderArtifact);
        assert!(
            dup_err
                .detail()
                .unwrap_or("")
                .contains("duplicate OpEntryPoint")
        );

        let mut multi_stage_spv = sample_mesh_vertex_shader();
        multi_stage_spv.entry_point = "main".to_owned();
        multi_stage_spv.spirv_words = synth_spirv(ShaderStage::Vertex, "main");
        let main_words = pack_spirv_string("main");
        let frag_ep_len = (3 + main_words.len()) as u32;
        multi_stage_spv.spirv_words.push((frag_ep_len << 16) | 15);
        multi_stage_spv
            .spirv_words
            .push(spirv_execution_model(ShaderStage::Fragment));
        multi_stage_spv.spirv_words.push(5);
        multi_stage_spv.spirv_words.extend_from_slice(&main_words);
        multi_stage_spv.validate().unwrap();
    }

    // 5. Non-vertex stage with vertex inputs, duplicate locations, duplicate bindings, unaligned push size.
    {
        let mut frag_with_vi = sample_mesh_fragment_shader();
        frag_with_vi
            .vertex_inputs
            .push(VertexInputAttribute::new(0, VertexFormat::Float32x3));
        assert_eq!(
            frag_with_vi.validate().unwrap_err().kind(),
            RenderErrorKind::InvalidShaderArtifact
        );

        let mut dup_vi = sample_mesh_vertex_shader();
        dup_vi
            .vertex_inputs
            .push(VertexInputAttribute::new(1, VertexFormat::Float32x2));
        assert_eq!(
            dup_vi.validate().unwrap_err().kind(),
            RenderErrorKind::InvalidShaderArtifact
        );

        let mut unaligned_pc = sample_mesh_vertex_shader();
        unaligned_pc.push_constants.size = 66;
        assert_eq!(
            unaligned_pc.validate().unwrap_err().kind(),
            RenderErrorKind::InvalidShaderArtifact
        );

        let mut oversized_pc = sample_mesh_vertex_shader();
        oversized_pc.push_constants.size = MAX_PUSH_CONSTANT_BYTES + 4;
        assert_eq!(
            oversized_pc.validate().unwrap_err().kind(),
            RenderErrorKind::InvalidShaderArtifact
        );
    }
}

#[test]
fn f3d_t07_rejects_pipeline_layout_mismatches_with_rend_0011() {
    let vs = sample_mesh_vertex_shader();
    let fs = sample_mesh_fragment_shader();

    let base_attrs = [
        VertexInputAttribute::new(0, VertexFormat::Float32x3),
        VertexInputAttribute::new(1, VertexFormat::Float32x3),
        VertexInputAttribute::new(2, VertexFormat::Float32x4),
    ];
    let base_bindings = [
        DescriptorBinding {
            set: 0,
            binding: 0,
            kind: DescriptorKind::UniformBuffer,
            count: 1,
            stages: ShaderStageFlags::ALL_GRAPHICS,
        },
        DescriptorBinding {
            set: 1,
            binding: 0,
            kind: DescriptorKind::SampledImage,
            count: RUNTIME_DESCRIPTOR_COUNT,
            stages: ShaderStageFlags::FRAGMENT,
        },
    ];

    // 1. Swapped stages.
    let err = validate_graphics_pipeline_shaders(
        &fs,
        &vs,
        &PipelineLayoutDesc {
            vertex_inputs: &base_attrs,
            push_constants: PushConstantRange::new(128, ShaderStageFlags::VERTEX),
            descriptor_bindings: &base_bindings,
        },
    )
    .unwrap_err();
    assert_eq!(err.kind(), RenderErrorKind::ShaderLayoutMismatch);
    assert_eq!(err.code(), "REND-0011");

    // 2. Missing vertex input location 2 in pipeline.
    let err = validate_graphics_pipeline_shaders(
        &vs,
        &fs,
        &PipelineLayoutDesc {
            vertex_inputs: &base_attrs[..2],
            push_constants: PushConstantRange::new(128, ShaderStageFlags::VERTEX),
            descriptor_bindings: &base_bindings,
        },
    )
    .unwrap_err();
    assert_eq!(err.kind(), RenderErrorKind::ShaderLayoutMismatch);
    assert!(err.detail().unwrap_or("").contains("location 2"));

    // 3. Wrong VertexFormat at location 1 (`Float32x4` instead of `Float32x3`).
    let wrong_fmt_attrs = [
        VertexInputAttribute::new(0, VertexFormat::Float32x3),
        VertexInputAttribute::new(1, VertexFormat::Float32x4),
        VertexInputAttribute::new(2, VertexFormat::Float32x4),
    ];
    let err = validate_graphics_pipeline_shaders(
        &vs,
        &fs,
        &PipelineLayoutDesc {
            vertex_inputs: &wrong_fmt_attrs,
            push_constants: PushConstantRange::new(128, ShaderStageFlags::VERTEX),
            descriptor_bindings: &base_bindings,
        },
    )
    .unwrap_err();
    assert_eq!(err.kind(), RenderErrorKind::ShaderLayoutMismatch);
    assert!(err.detail().unwrap_or("").contains("Float32x3"));

    // 4. Extra unused vertex attribute in pipeline (`location 3`).
    let extra_attrs = [
        VertexInputAttribute::new(0, VertexFormat::Float32x3),
        VertexInputAttribute::new(1, VertexFormat::Float32x3),
        VertexInputAttribute::new(2, VertexFormat::Float32x4),
        VertexInputAttribute::new(3, VertexFormat::Float32x2),
    ];
    let err = validate_graphics_pipeline_shaders(
        &vs,
        &fs,
        &PipelineLayoutDesc {
            vertex_inputs: &extra_attrs,
            push_constants: PushConstantRange::new(128, ShaderStageFlags::VERTEX),
            descriptor_bindings: &base_bindings,
        },
    )
    .unwrap_err();
    assert_eq!(err.kind(), RenderErrorKind::ShaderLayoutMismatch);
    assert!(err.detail().unwrap_or("").contains("location 3"));

    // 5. Push-constant byte size mismatch (`64` vs `128`, and `0` vs `128`).
    for pc in [
        PushConstantRange::new(64, ShaderStageFlags::VERTEX),
        PushConstantRange::NONE,
        PushConstantRange::new(128, ShaderStageFlags::FRAGMENT),
    ] {
        let err = validate_graphics_pipeline_shaders(
            &vs,
            &fs,
            &PipelineLayoutDesc {
                vertex_inputs: &base_attrs,
                push_constants: pc,
                descriptor_bindings: &base_bindings,
            },
        )
        .unwrap_err();
        assert_eq!(err.kind(), RenderErrorKind::ShaderLayoutMismatch);
    }

    // 6. Missing descriptor binding, wrong kind, wrong count, wrong stage visibility.
    let err = validate_graphics_pipeline_shaders(
        &vs,
        &fs,
        &PipelineLayoutDesc {
            vertex_inputs: &base_attrs,
            push_constants: PushConstantRange::new(128, ShaderStageFlags::VERTEX),
            descriptor_bindings: &base_bindings[..1],
        },
    )
    .unwrap_err();
    assert_eq!(err.kind(), RenderErrorKind::ShaderLayoutMismatch);
    assert!(err.detail().unwrap_or("").contains("set=1, binding=0"));

    let wrong_kind_bindings = [
        base_bindings[0],
        DescriptorBinding {
            kind: DescriptorKind::CombinedImageSampler,
            ..base_bindings[1]
        },
    ];
    let err = validate_graphics_pipeline_shaders(
        &vs,
        &fs,
        &PipelineLayoutDesc {
            vertex_inputs: &base_attrs,
            push_constants: PushConstantRange::new(128, ShaderStageFlags::VERTEX),
            descriptor_bindings: &wrong_kind_bindings,
        },
    )
    .unwrap_err();
    assert_eq!(err.kind(), RenderErrorKind::ShaderLayoutMismatch);

    let wrong_stage_bindings = [
        DescriptorBinding {
            stages: ShaderStageFlags::VERTEX, // missing FRAGMENT
            ..base_bindings[0]
        },
        base_bindings[1],
    ];
    let err = validate_graphics_pipeline_shaders(
        &vs,
        &fs,
        &PipelineLayoutDesc {
            vertex_inputs: &base_attrs,
            push_constants: PushConstantRange::new(128, ShaderStageFlags::VERTEX),
            descriptor_bindings: &wrong_stage_bindings,
        },
    )
    .unwrap_err();
    assert_eq!(err.kind(), RenderErrorKind::ShaderLayoutMismatch);
}

#[test]
fn f3d_t07_error_reports_match_adr_0009_snapshots() {
    let mut bad_magic = sample_mesh_vertex_shader().encode().unwrap();
    bad_magic[0..8].copy_from_slice(b"CORRUPT!");
    let decode_err: RenderError =
        CookedShader::decode_named("target/shaders/mesh_3d.vs_main.spnshdr", &bad_magic)
            .unwrap_err();
    insta::assert_snapshot!(
        normalize_report_location(&decode_err.to_string()),
        @r#"
error[REND-0010]: invalid cooked shader artifact
  --> crates/spawn-render/tests/shader_validation.rs:<LINE>:<COL>
   = detail: artifact `target/shaders/mesh_3d.vs_main.spnshdr`: field `magic`: expected `SPNSHDR\0`, found [43, 4f, 52, 52, 55, 50, 54, 21]
"#
    );

    let vs = sample_mesh_vertex_shader();
    let mismatch_err = vs
        .validate_against(&ShaderLayoutExpectation {
            stage: ShaderStage::Vertex,
            vertex_inputs: &[
                VertexInputAttribute::new(0, VertexFormat::Float32x3),
                VertexInputAttribute::new(1, VertexFormat::Float32x4),
                VertexInputAttribute::new(2, VertexFormat::Float32x4),
            ],
            push_constants: PushConstantRange::new(128, ShaderStageFlags::VERTEX),
            descriptor_bindings: &vs.descriptor_bindings,
        })
        .unwrap_err();
    insta::assert_snapshot!(
        normalize_report_location(&mismatch_err.to_string()),
        @r#"
error[REND-0011]: shader interface does not match pipeline layout
  --> crates/spawn-render/tests/shader_validation.rs:<LINE>:<COL>
   = detail: entry `vs_main`: vertex input location 1 has format `Float32x3` in shader, but pipeline provides `Float32x4`
"#
    );
}

fn arb_stage() -> impl Strategy<Value = ShaderStage> {
    prop_oneof![
        Just(ShaderStage::Vertex),
        Just(ShaderStage::Fragment),
        Just(ShaderStage::Compute),
    ]
}

fn arb_vertex_format() -> impl Strategy<Value = VertexFormat> {
    prop_oneof![
        Just(VertexFormat::Float32),
        Just(VertexFormat::Float32x2),
        Just(VertexFormat::Float32x3),
        Just(VertexFormat::Float32x4),
        Just(VertexFormat::Sint32),
        Just(VertexFormat::Sint32x2),
        Just(VertexFormat::Sint32x3),
        Just(VertexFormat::Sint32x4),
        Just(VertexFormat::Uint32),
        Just(VertexFormat::Uint32x2),
        Just(VertexFormat::Uint32x3),
        Just(VertexFormat::Uint32x4),
    ]
}

fn arb_descriptor_kind() -> impl Strategy<Value = DescriptorKind> {
    prop_oneof![
        Just(DescriptorKind::UniformBuffer),
        Just(DescriptorKind::StorageBuffer),
        Just(DescriptorKind::SampledImage),
        Just(DescriptorKind::StorageImage),
        Just(DescriptorKind::Sampler),
        Just(DescriptorKind::CombinedImageSampler),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn f3d_t07_proptest_roundtrip_arbitrary_valid_cooked_shaders(
        stage in arb_stage(),
        entry in "[a-zA-Z_][a-zA-Z0-9_]{0,31}",
        push_words in 0u32..=64u32,
        extra_stage_bits in 0u32..=7u32,
        raw_bindings in prop::collection::vec((0u32..4, 0u32..16, arb_descriptor_kind(), 1u32..=8), 0..12),
        raw_attrs in prop::collection::vec((0u32..16, arb_vertex_format()), 0..8),
    ) {
        let stage_flag = stage.stage_flag();
        let extra_flags = ShaderStageFlags::from_bits(extra_stage_bits).unwrap_or_default();
        let push_constants = if push_words == 0 {
            PushConstantRange::NONE
        } else {
            PushConstantRange::new(push_words * 4, stage_flag | extra_flags)
        };

        let mut binding_map = std::collections::BTreeMap::new();
        for (set, binding, kind, count) in raw_bindings {
            binding_map.insert((set, binding), DescriptorBinding {
                set,
                binding,
                kind,
                count,
                stages: stage_flag | extra_flags,
            });
        }
        let descriptor_bindings: Vec<_> = binding_map.into_values().collect();

        let vertex_inputs: Vec<_> = if stage == ShaderStage::Vertex {
            let mut attr_map = std::collections::BTreeMap::new();
            for (loc, fmt) in raw_attrs {
                attr_map.insert(loc, VertexInputAttribute::new(loc, fmt));
            }
            attr_map.into_values().collect()
        } else {
            Vec::new()
        };

        let shader = CookedShader {
            version: SHADER_FORMAT_VERSION,
            stage,
            entry_point: entry.clone(),
            push_constants,
            descriptor_bindings: descriptor_bindings.clone(),
            vertex_inputs: vertex_inputs.clone(),
            spirv_words: synth_spirv(stage, &entry),
        };

        let encoded = shader.encode().unwrap();
        prop_assert_eq!(&encoded[..8], &SHADER_MAGIC);
        prop_assert_eq!(encoded.len() % 4, 0);

        let decoded = CookedShader::decode(&encoded).unwrap();
        prop_assert_eq!(&decoded, &shader);

        decoded.validate_against(&ShaderLayoutExpectation {
            stage,
            vertex_inputs: &vertex_inputs,
            push_constants,
            descriptor_bindings: &descriptor_bindings,
        }).unwrap();
    }

    #[test]
    fn f3d_t07_proptest_arbitrary_bytes_and_mutations_never_panic(
        raw_bytes in prop::collection::vec(any::<u8>(), 0..2048),
        mutate_index in 0usize..4096,
        mutate_xor in 1u8..=255u8,
    ) {
        // 1. Completely arbitrary byte stream.
        if let Ok(decoded) = CookedShader::decode(&raw_bytes) {
            let reencoded = decoded.encode().unwrap();
            prop_assert!(CookedShader::decode(&reencoded).is_ok());
        }

        // 2. Structured `arbitrary::Unstructured` harness (same logic as `fuzz_targets/decode_cooked_shader.rs`).
        let mut u = Unstructured::new(&raw_bytes);
        if let (Ok(stage_idx), Ok(push_size)) = (u.int_in_range::<u8>(0..=2), u.int_in_range::<u32>(0..=300)) {
            let stage = match stage_idx {
                0 => ShaderStage::Vertex,
                1 => ShaderStage::Fragment,
                _ => ShaderStage::Compute,
            };
            let expected = ShaderLayoutExpectation {
                stage,
                vertex_inputs: &[],
                push_constants: PushConstantRange::new(push_size, stage.stage_flag()),
                descriptor_bindings: &[],
            };
            if let Ok(shader) = CookedShader::decode_named("fuzz_seed.spnshdr", u.take_rest()) {
                let _ = shader.validate_against(&expected);
            }
        }

        // 3. Single-byte bit-flip mutation on a valid cooked shader artifact.
        let mut mutated = sample_mesh_vertex_shader().encode().unwrap();
        let target = mutate_index % mutated.len();
        mutated[target] ^= mutate_xor;
        if let Ok(decoded) = CookedShader::decode(&mutated) {
            let _ = decoded.encode();
        }
    }
}
