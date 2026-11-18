//! Gate G2 `cargo-fuzz` target for `spawn-render`'s untrusted cooked-shader
//! binary parser and pipeline-layout compatibility validator (`SPN-INV-034`, `F3D-T07`).

#![no_main]
#![forbid(unsafe_code)]

use arbitrary::Unstructured;
use libfuzzer_sys::fuzz_target;
use spawn_render::{
    CookedShader, DescriptorBinding, DescriptorKind, PipelineLayoutDesc, PushConstantRange,
    ShaderLayoutExpectation, ShaderStage, ShaderStageFlags, VertexFormat, VertexInputAttribute,
    validate_graphics_pipeline_shaders,
};

fuzz_target!(|data: &[u8]| {
    // 1. Raw arbitrary-byte untrusted decode: must never panic, invoke UB, or
    //    allocate unbounded memory regardless of header length fields.
    if let Ok(shader) = CookedShader::decode(data) {
        if let Ok(reencoded) = shader.encode() {
            let roundtrip = CookedShader::decode(&reencoded);
            assert!(roundtrip.is_ok());
        }
    }

    // 2. Structured mutation over a valid header frame via `arbitrary::Unstructured`
    //    so fuzzing reaches deep validation branches (SPIR-V instructions, stage/layout checks).
    let mut u = Unstructured::new(data);
    let Ok(stage_choice) = u.int_in_range::<u8>(0..=2) else {
        return;
    };
    let stage = match stage_choice {
        0 => ShaderStage::Vertex,
        1 => ShaderStage::Fragment,
        _ => ShaderStage::Compute,
    };
    let Ok(push_size) = u.int_in_range::<u32>(0..=300) else {
        return;
    };
    let Ok(push_bits) = u.arbitrary::<u32>() else {
        return;
    };
    let push_stages = ShaderStageFlags::from_bits(push_bits & 0b111).unwrap_or_default();
    let expected = ShaderLayoutExpectation {
        stage,
        vertex_inputs: &[VertexInputAttribute::new(0, VertexFormat::Float32x3)],
        push_constants: PushConstantRange::new(push_size, push_stages),
        descriptor_bindings: &[DescriptorBinding {
            set: 0,
            binding: 0,
            kind: DescriptorKind::UniformBuffer,
            count: 1,
            stages: ShaderStageFlags::ALL,
        }],
    };

    let rest = u.take_rest();
    if let Ok(shader) = CookedShader::decode_named("fuzz.spnshdr", rest) {
        let _ = shader.validate_against(&expected);
        let layout = PipelineLayoutDesc {
            vertex_inputs: expected.vertex_inputs,
            push_constants: expected.push_constants,
            descriptor_bindings: expected.descriptor_bindings,
        };
        let _ = validate_graphics_pipeline_shaders(&shader, &shader, &layout);
    }
});
