//! Writes a small synthetic seed corpus for the `decode_cooked_shader` fuzz
//! target (Gate G2). Seeds are generated, never committed: cooked shader
//! binaries stay out of version control (`SPN-INV-084`, `.gitignore`).
//!
//! Usage: `cargo run --example seed_corpus -- <corpus-dir>`

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

use spawn_render::{
    CookedShader, DescriptorBinding, DescriptorKind, MESH_PUSH_CONSTANT_RANGE,
    MESH_VERTEX_ATTRIBUTES, PushConstantRange, SHADER_FORMAT_VERSION, SPIRV_MAGIC, ShaderStage,
    ShaderStageFlags,
};

/// Minimal structurally valid SPIR-V 1.6 module: header, `OpCapability Shader`,
/// `OpMemoryModel Logical GLSL450`, and one `OpEntryPoint` named `name`.
fn minimal_spirv(execution_model: u32, name: &[u8; 8]) -> Vec<u32> {
    let (lo, hi) = name.split_at(4);
    let word = |bytes: &[u8]| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    vec![
        SPIRV_MAGIC,
        0x0001_0600,
        0,
        8,
        0,
        (2 << 16) | 17, // OpCapability Shader
        1,
        (3 << 16) | 14, // OpMemoryModel Logical GLSL450
        0,
        1,
        (5 << 16) | 15, // OpEntryPoint <model> %4 "<name>"
        execution_model,
        4,
        word(lo),
        word(hi),
    ]
}

fn seeds() -> Vec<(&'static str, CookedShader)> {
    let uniform = DescriptorBinding {
        set: 0,
        binding: 0,
        kind: DescriptorKind::UniformBuffer,
        count: 1,
        stages: ShaderStageFlags::ALL,
    };
    vec![
        (
            "mesh_vertex",
            CookedShader {
                version: SHADER_FORMAT_VERSION,
                stage: ShaderStage::Vertex,
                entry_point: "vs_main".to_owned(),
                push_constants: MESH_PUSH_CONSTANT_RANGE,
                descriptor_bindings: Vec::new(),
                vertex_inputs: MESH_VERTEX_ATTRIBUTES.to_vec(),
                spirv_words: minimal_spirv(0, b"vs_main\0"),
            },
        ),
        (
            "mesh_fragment",
            CookedShader {
                version: SHADER_FORMAT_VERSION,
                stage: ShaderStage::Fragment,
                entry_point: "fs_main".to_owned(),
                push_constants: PushConstantRange::NONE,
                descriptor_bindings: Vec::new(),
                vertex_inputs: Vec::new(),
                spirv_words: minimal_spirv(4, b"fs_main\0"),
            },
        ),
        (
            "fragment_with_uniform",
            CookedShader {
                version: SHADER_FORMAT_VERSION,
                stage: ShaderStage::Fragment,
                entry_point: "fs_main".to_owned(),
                push_constants: PushConstantRange::NONE,
                descriptor_bindings: vec![uniform],
                vertex_inputs: Vec::new(),
                spirv_words: minimal_spirv(4, b"fs_main\0"),
            },
        ),
        (
            "compute",
            CookedShader {
                version: SHADER_FORMAT_VERSION,
                stage: ShaderStage::Compute,
                entry_point: "cs_main".to_owned(),
                push_constants: PushConstantRange::NONE,
                descriptor_bindings: vec![uniform],
                vertex_inputs: Vec::new(),
                spirv_words: minimal_spirv(5, b"cs_main\0"),
            },
        ),
    ]
}

fn main() -> ExitCode {
    let Some(dir) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: seed_corpus <corpus-dir>");
        return ExitCode::from(2);
    };
    if let Err(err) = std::fs::create_dir_all(&dir) {
        eprintln!("seed_corpus: cannot create {}: {err}", dir.display());
        return ExitCode::FAILURE;
    }
    for (name, shader) in seeds() {
        let bytes = match shader.encode() {
            Ok(bytes) => bytes,
            Err(err) => {
                eprintln!("seed_corpus: seed `{name}` failed to encode: {err}");
                return ExitCode::FAILURE;
            }
        };
        if let Err(err) = CookedShader::decode(&bytes) {
            eprintln!("seed_corpus: seed `{name}` does not round-trip: {err}");
            return ExitCode::FAILURE;
        }
        // Raw decode path seed, plus a structured-path seed prefixed with the
        // bytes the fuzz target consumes through `arbitrary::Unstructured`.
        let structured: Vec<u8> = [0u8, 128, 0, 0, 0, 1, 0, 0, 0]
            .into_iter()
            .chain(bytes.iter().copied())
            .collect();
        for (suffix, payload) in [("raw", &bytes), ("structured", &structured)] {
            let path = dir.join(format!("{name}.{suffix}.seed"));
            if let Err(err) = std::fs::write(&path, payload) {
                eprintln!("seed_corpus: cannot write {}: {err}", path.display());
                return ExitCode::FAILURE;
            }
        }
        println!("seed_corpus: wrote `{name}` ({} bytes)", bytes.len());
    }
    ExitCode::SUCCESS
}
