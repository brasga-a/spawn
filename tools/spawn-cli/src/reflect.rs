//! SPIR-V inspection and reflection extraction via `rspirv` (`ADR-0014`, `SPN-INV-034`).
//!
//! Converts raw SPIR-V bytes emitted by `slangc` into a validated
//! [`spawn_render::CookedShader`] without exposing `rspirv` types in `spawn-cli`'s
//! public API.

#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use rspirv::dr::{Instruction, Module, Operand};
use rspirv::spirv::{Decoration, ExecutionModel, Op, StorageClass};
use spawn_render::{
    CookedShader, DescriptorBinding, DescriptorKind, MAX_COOKED_SHADER_BYTES,
    MAX_PUSH_CONSTANT_BYTES, PushConstantRange, RUNTIME_DESCRIPTOR_COUNT, SHADER_FORMAT_VERSION,
    ShaderStage, VertexFormat, VertexInputAttribute,
};

use crate::error::{CliError, CliErrorKind};

const MAX_TYPE_NESTING_DEPTH: usize = 32;

/// Parses `spirv_bytes` with `rspirv`, extracts reflection metadata for `(stage, entry_point)`,
/// and returns a validated [`CookedShader`].
#[track_caller]
pub(crate) fn extract_cooked_shader(
    source_file: &Path,
    stage: ShaderStage,
    entry_point: &str,
    spirv_bytes: &[u8],
) -> Result<CookedShader, CliError> {
    if spirv_bytes.is_empty() {
        return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
            .with_file(source_file)
            .with_field("spirv_bytes")
            .with_detail(format!(
                "entry `{entry_point}` ({stage}): compiler produced 0 bytes of SPIR-V"
            )));
    }
    if spirv_bytes.len() > MAX_COOKED_SHADER_BYTES {
        return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
            .with_file(source_file)
            .with_field("spirv_bytes")
            .with_detail(format!(
                "entry `{entry_point}` ({stage}): SPIR-V size {} bytes exceeds maximum {} bytes",
                spirv_bytes.len(),
                MAX_COOKED_SHADER_BYTES
            )));
    }
    if !spirv_bytes.len().is_multiple_of(4) {
        return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
            .with_file(source_file)
            .with_field("spirv_bytes")
            .with_detail(format!(
                "entry `{entry_point}` ({stage}): SPIR-V byte length {} is not a multiple of 4",
                spirv_bytes.len()
            )));
    }

    let module = rspirv::dr::load_bytes(spirv_bytes).map_err(|err| {
        CliError::new(CliErrorKind::ShaderReflectionFailed)
            .with_file(source_file)
            .with_field("spirv_module")
            .with_detail(format!(
                "entry `{entry_point}` ({stage}): failed to parse SPIR-V module"
            ))
            .with_cause(&err)
    })?;

    let spirv_words: Vec<u32> = spirv_bytes
        .as_chunks::<4>()
        .0
        .iter()
        .copied()
        .map(u32::from_le_bytes)
        .collect();

    let expected_model = match stage {
        ShaderStage::Vertex => ExecutionModel::Vertex,
        ShaderStage::Fragment => ExecutionModel::Fragment,
        ShaderStage::Compute => ExecutionModel::GLCompute,
    };

    let (target_fn_id, interface_ids) =
        find_entry_point(&module, source_file, expected_model, stage, entry_point)?;
    let reachable_used_ids = collect_reachable_ids(&module, target_fn_id);

    let annotations = collect_annotations(&module);

    let mut defs: BTreeMap<u32, &Instruction> = BTreeMap::new();
    let mut global_vars: Vec<&Instruction> = Vec::new();
    for inst in &module.types_global_values {
        if let Some(id) = inst.result_id {
            defs.insert(id, inst);
        }
        if inst.class.opcode == Op::Variable {
            global_vars.push(inst);
        }
    }

    let vertex_inputs = if stage == ShaderStage::Vertex {
        extract_vertex_inputs(
            source_file,
            entry_point,
            &global_vars,
            &interface_ids,
            &annotations,
            &defs,
        )?
    } else {
        Vec::new()
    };

    let push_constants = extract_push_constants(
        source_file,
        stage,
        entry_point,
        &global_vars,
        &interface_ids,
        &reachable_used_ids,
        &annotations,
        &defs,
    )?;

    let descriptor_bindings = extract_descriptor_bindings(
        source_file,
        stage,
        entry_point,
        &global_vars,
        &interface_ids,
        &reachable_used_ids,
        &annotations,
        &defs,
    )?;

    let cooked = CookedShader {
        version: SHADER_FORMAT_VERSION,
        stage,
        entry_point: entry_point.to_owned(),
        push_constants,
        descriptor_bindings,
        vertex_inputs,
        spirv_words,
    };

    cooked.validate().map_err(|err| {
        CliError::new(CliErrorKind::ShaderReflectionFailed)
            .with_file(source_file)
            .with_field("cooked_shader")
            .with_detail(format!(
                "entry `{entry_point}` ({stage}): cooked artifact failed validation"
            ))
            .with_cause(&err)
    })?;

    Ok(cooked)
}

#[derive(Default)]
struct AnnotationTables {
    locations: BTreeMap<u32, u32>,
    descriptor_sets: BTreeMap<u32, u32>,
    bindings: BTreeMap<u32, u32>,
    builtins: BTreeSet<u32>,
    buffer_blocks: BTreeSet<u32>,
    array_strides: BTreeMap<u32, u32>,
    member_offsets: BTreeMap<(u32, u32), u32>,
    member_matrix_strides: BTreeMap<(u32, u32), u32>,
    member_row_major: BTreeSet<(u32, u32)>,
    struct_has_builtin: BTreeSet<u32>,
}

fn collect_annotations(module: &Module) -> AnnotationTables {
    let mut tables = AnnotationTables::default();
    let mut groups: BTreeMap<u32, Vec<(Decoration, Option<u32>)>> = BTreeMap::new();

    for inst in &module.annotations {
        match inst.class.opcode {
            Op::Decorate => {
                if let (Some(&Operand::IdRef(target)), Some(&Operand::Decoration(dec))) =
                    (inst.operands.first(), inst.operands.get(1))
                {
                    let lit = operand_u32(inst.operands.get(2));
                    apply_decoration(&mut tables, target, dec, lit);
                    groups.entry(target).or_default().push((dec, lit));
                }
            }
            Op::MemberDecorate => {
                if let (
                    Some(&Operand::IdRef(struct_id)),
                    Some(&Operand::LiteralBit32(member_idx)),
                    Some(&Operand::Decoration(dec)),
                ) = (
                    inst.operands.first(),
                    inst.operands.get(1),
                    inst.operands.get(2),
                ) {
                    let lit = operand_u32(inst.operands.get(3));
                    apply_member_decoration(&mut tables, struct_id, member_idx, dec, lit);
                }
            }
            Op::GroupDecorate => {
                if let Some(&Operand::IdRef(group_id)) = inst.operands.first()
                    && let Some(group_decs) = groups.get(&group_id).cloned()
                    && let Some(targets) = inst.operands.get(1..)
                {
                    for op in targets {
                        if let Operand::IdRef(target) = *op {
                            for &(dec, lit) in &group_decs {
                                apply_decoration(&mut tables, target, dec, lit);
                            }
                        }
                    }
                }
            }
            Op::GroupMemberDecorate => {
                if let Some(&Operand::IdRef(group_id)) = inst.operands.first()
                    && let Some(group_decs) = groups.get(&group_id).cloned()
                    && let Some(pairs) = inst.operands.get(1..)
                {
                    for pair in pairs.as_chunks::<2>().0 {
                        if let [Operand::IdRef(struct_id), Operand::LiteralBit32(member_idx)] =
                            *pair
                        {
                            for &(dec, lit) in &group_decs {
                                apply_member_decoration(
                                    &mut tables,
                                    struct_id,
                                    member_idx,
                                    dec,
                                    lit,
                                );
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    tables
}

fn apply_decoration(tables: &mut AnnotationTables, target: u32, dec: Decoration, lit: Option<u32>) {
    match dec {
        Decoration::Location => {
            if let Some(loc) = lit {
                tables.locations.insert(target, loc);
            }
        }
        Decoration::DescriptorSet => {
            if let Some(set) = lit {
                tables.descriptor_sets.insert(target, set);
            }
        }
        Decoration::Binding => {
            if let Some(binding) = lit {
                tables.bindings.insert(target, binding);
            }
        }
        Decoration::BuiltIn => {
            tables.builtins.insert(target);
        }
        Decoration::BufferBlock => {
            tables.buffer_blocks.insert(target);
        }
        Decoration::ArrayStride => {
            if let Some(stride) = lit {
                tables.array_strides.insert(target, stride);
            }
        }
        _ => {}
    }
}

fn apply_member_decoration(
    tables: &mut AnnotationTables,
    struct_id: u32,
    member_idx: u32,
    dec: Decoration,
    lit: Option<u32>,
) {
    match dec {
        Decoration::Offset => {
            if let Some(offset) = lit {
                tables
                    .member_offsets
                    .insert((struct_id, member_idx), offset);
            }
        }
        Decoration::MatrixStride => {
            if let Some(stride) = lit {
                tables
                    .member_matrix_strides
                    .insert((struct_id, member_idx), stride);
            }
        }
        Decoration::RowMajor => {
            tables.member_row_major.insert((struct_id, member_idx));
        }
        Decoration::BuiltIn => {
            tables.struct_has_builtin.insert(struct_id);
        }
        _ => {}
    }
}

#[track_caller]
fn find_entry_point(
    module: &Module,
    source_file: &Path,
    expected_model: ExecutionModel,
    stage: ShaderStage,
    entry_point: &str,
) -> Result<(u32, BTreeSet<u32>), CliError> {
    let mut seen_entries: BTreeSet<(u32, &str)> = BTreeSet::new();
    let mut matched: Option<(u32, BTreeSet<u32>)> = None;

    for ep in &module.entry_points {
        if let (
            Some(&Operand::ExecutionModel(model)),
            Some(&Operand::IdRef(fn_id)),
            Some(Operand::LiteralString(name)),
        ) = (ep.operands.first(), ep.operands.get(1), ep.operands.get(2))
        {
            if !seen_entries.insert((model as u32, name.as_str())) {
                return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_file)
                    .with_field("entry_point")
                    .with_detail(format!(
                        "SPIR-V module declares duplicate OpEntryPoint `({model:?}, {name})`"
                    )));
            }
            if model == expected_model && name == entry_point {
                let mut iface = BTreeSet::new();
                if let Some(rest) = ep.operands.get(3..) {
                    for op in rest {
                        if let Operand::IdRef(id) = *op {
                            iface.insert(id);
                        }
                    }
                }
                matched = Some((fn_id, iface));
            }
        }
    }

    matched.ok_or_else(|| {
        CliError::new(CliErrorKind::ShaderReflectionFailed)
            .with_file(source_file)
            .with_field("entry_point")
            .with_detail(format!(
                "SPIR-V module does not contain `{stage}` entry point `{entry_point}`"
            ))
    })
}

fn collect_reachable_ids(module: &Module, root_fn_id: u32) -> BTreeSet<u32> {
    let mut fn_map = BTreeMap::new();
    for func in &module.functions {
        if let Some(id) = func.def.as_ref().and_then(|def| def.result_id) {
            fn_map.insert(id, func);
        }
    }

    let mut visited_fns = BTreeSet::new();
    let mut queue = VecDeque::new();
    let mut used_ids = BTreeSet::new();
    queue.push_back(root_fn_id);

    while let Some(fn_id) = queue.pop_front() {
        if !visited_fns.insert(fn_id) {
            continue;
        }
        let Some(func) = fn_map.get(&fn_id) else {
            continue;
        };
        for block in &func.blocks {
            for inst in &block.instructions {
                if inst.class.opcode == Op::FunctionCall
                    && let Some(&Operand::IdRef(callee)) = inst.operands.first()
                {
                    queue.push_back(callee);
                }
                for op in &inst.operands {
                    if let Operand::IdRef(id) = *op {
                        used_ids.insert(id);
                    }
                }
            }
        }
    }

    used_ids
}

#[track_caller]
fn extract_vertex_inputs(
    source_file: &Path,
    entry_point: &str,
    global_vars: &[&Instruction],
    interface_ids: &BTreeSet<u32>,
    annotations: &AnnotationTables,
    defs: &BTreeMap<u32, &Instruction>,
) -> Result<Vec<VertexInputAttribute>, CliError> {
    let mut by_location: BTreeMap<u32, VertexFormat> = BTreeMap::new();

    for var in global_vars {
        let Some(var_id) = var.result_id else {
            continue;
        };
        if !interface_ids.contains(&var_id) {
            continue;
        }
        let Some(&Operand::StorageClass(StorageClass::Input)) = var.operands.first() else {
            continue;
        };
        if annotations.builtins.contains(&var_id) {
            continue;
        }

        let pointee_id = resolve_pointer_pointee(source_file, var, defs)?;
        if annotations.builtins.contains(&pointee_id)
            || annotations.struct_has_builtin.contains(&pointee_id)
        {
            continue;
        }

        let Some(&location) = annotations.locations.get(&var_id) else {
            return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                .with_file(source_file)
                .with_field("vertex_input")
                .with_detail(format!(
                    "entry `{entry_point}`: non-builtin vertex input `%id={var_id}` is missing `Location` decoration"
                )));
        };

        let format = map_vertex_format(source_file, entry_point, location, pointee_id, defs)?;
        if by_location.insert(location, format).is_some() {
            return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                .with_file(source_file)
                .with_field("vertex_input")
                .with_detail(format!(
                    "entry `{entry_point}`: duplicate vertex input location {location}"
                )));
        }
    }

    Ok(by_location
        .into_iter()
        .map(|(location, format)| VertexInputAttribute { location, format })
        .collect())
}

#[track_caller]
fn map_vertex_format(
    source_file: &Path,
    entry_point: &str,
    location: u32,
    type_id: u32,
    defs: &BTreeMap<u32, &Instruction>,
) -> Result<VertexFormat, CliError> {
    let type_inst = defs.get(&type_id).copied().ok_or_else(|| {
        CliError::new(CliErrorKind::ShaderReflectionFailed)
            .with_file(source_file)
            .with_field("vertex_input")
            .with_detail(format!(
                "entry `{entry_point}`: unknown SPIR-V type id {type_id} at location {location}"
            ))
    })?;

    match type_inst.class.opcode {
        Op::TypeFloat => {
            let width = operand_u32(type_inst.operands.first()).unwrap_or(0);
            if width == 32 {
                Ok(VertexFormat::Float32)
            } else {
                Err(unsupported_vertex_type(
                    source_file,
                    entry_point,
                    location,
                    &format!("float{width}"),
                ))
            }
        }
        Op::TypeInt => {
            let width = operand_u32(type_inst.operands.first()).unwrap_or(0);
            let signed = operand_u32(type_inst.operands.get(1)).unwrap_or(0);
            if width != 32 {
                return Err(unsupported_vertex_type(
                    source_file,
                    entry_point,
                    location,
                    &format!("int{width}"),
                ));
            }
            if signed == 1 {
                Ok(VertexFormat::Sint32)
            } else {
                Ok(VertexFormat::Uint32)
            }
        }
        Op::TypeVector => {
            let Some(&Operand::IdRef(comp_id)) = type_inst.operands.first() else {
                return Err(unsupported_vertex_type(
                    source_file,
                    entry_point,
                    location,
                    "malformed vector",
                ));
            };
            let count = operand_u32(type_inst.operands.get(1)).unwrap_or(0);
            let scalar = map_vertex_format(source_file, entry_point, location, comp_id, defs)?;
            match (scalar, count) {
                (VertexFormat::Float32, 2) => Ok(VertexFormat::Float32x2),
                (VertexFormat::Float32, 3) => Ok(VertexFormat::Float32x3),
                (VertexFormat::Float32, 4) => Ok(VertexFormat::Float32x4),
                (VertexFormat::Sint32, 2) => Ok(VertexFormat::Sint32x2),
                (VertexFormat::Sint32, 3) => Ok(VertexFormat::Sint32x3),
                (VertexFormat::Sint32, 4) => Ok(VertexFormat::Sint32x4),
                (VertexFormat::Uint32, 2) => Ok(VertexFormat::Uint32x2),
                (VertexFormat::Uint32, 3) => Ok(VertexFormat::Uint32x3),
                (VertexFormat::Uint32, 4) => Ok(VertexFormat::Uint32x4),
                _ => Err(unsupported_vertex_type(
                    source_file,
                    entry_point,
                    location,
                    &format!("{scalar}x{count}"),
                )),
            }
        }
        other => Err(unsupported_vertex_type(
            source_file,
            entry_point,
            location,
            &format!("{other:?}"),
        )),
    }
}

#[track_caller]
fn unsupported_vertex_type(
    source_file: &Path,
    entry_point: &str,
    location: u32,
    desc: &str,
) -> CliError {
    CliError::new(CliErrorKind::ShaderReflectionFailed)
        .with_file(source_file)
        .with_field("vertex_input")
        .with_detail(format!(
            "entry `{entry_point}`: unsupported vertex attribute type `{desc}` at location {location}"
        ))
}

#[track_caller]
#[allow(clippy::too_many_arguments)]
fn extract_push_constants(
    source_file: &Path,
    stage: ShaderStage,
    entry_point: &str,
    global_vars: &[&Instruction],
    interface_ids: &BTreeSet<u32>,
    reachable_used_ids: &BTreeSet<u32>,
    annotations: &AnnotationTables,
    defs: &BTreeMap<u32, &Instruction>,
) -> Result<PushConstantRange, CliError> {
    let mut found_range: Option<PushConstantRange> = None;

    for var in global_vars {
        let Some(var_id) = var.result_id else {
            continue;
        };
        let Some(&Operand::StorageClass(StorageClass::PushConstant)) = var.operands.first() else {
            continue;
        };
        if !interface_ids.contains(&var_id) && !reachable_used_ids.contains(&var_id) {
            continue;
        }

        if found_range.is_some() {
            return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                .with_file(source_file)
                .with_field("push_constants")
                .with_detail(format!(
                    "entry `{entry_point}` ({stage}): multiple push-constant blocks declared"
                )));
        }

        let struct_id = resolve_pointer_pointee(source_file, var, defs)?;
        let raw_size = compute_type_byte_size(
            source_file,
            entry_point,
            struct_id,
            None,
            annotations,
            defs,
            0,
        )?;
        if raw_size == 0 {
            return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                .with_file(source_file)
                .with_field("push_constants")
                .with_detail(format!(
                    "entry `{entry_point}` ({stage}): push-constant block has zero byte size"
                )));
        }
        let aligned_size = raw_size.checked_add(3).map(|n| n & !3).ok_or_else(|| {
            CliError::new(CliErrorKind::ShaderReflectionFailed)
                .with_file(source_file)
                .with_field("push_constants")
                .with_detail("push-constant size overflow")
        })?;
        if aligned_size > MAX_PUSH_CONSTANT_BYTES {
            return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                .with_file(source_file)
                .with_field("push_constants")
                .with_detail(format!(
                    "entry `{entry_point}` ({stage}): push-constant size {aligned_size} bytes exceeds maximum {MAX_PUSH_CONSTANT_BYTES} bytes"
                )));
        }
        found_range = Some(PushConstantRange::new(aligned_size, stage.stage_flag()));
    }

    Ok(found_range.unwrap_or(PushConstantRange::NONE))
}

#[derive(Debug, Clone, Copy)]
struct MemberMatrixLayout {
    stride: u32,
    row_major: bool,
}

#[track_caller]
#[allow(clippy::too_many_arguments)]
fn compute_type_byte_size(
    source_file: &Path,
    entry_point: &str,
    type_id: u32,
    matrix_layout: Option<MemberMatrixLayout>,
    annotations: &AnnotationTables,
    defs: &BTreeMap<u32, &Instruction>,
    depth: usize,
) -> Result<u32, CliError> {
    if depth > MAX_TYPE_NESTING_DEPTH {
        return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
            .with_file(source_file)
            .with_field("push_constants")
            .with_detail(format!(
                "entry `{entry_point}`: SPIR-V type nesting depth exceeds {MAX_TYPE_NESTING_DEPTH}"
            )));
    }

    let inst = defs.get(&type_id).copied().ok_or_else(|| {
        CliError::new(CliErrorKind::ShaderReflectionFailed)
            .with_file(source_file)
            .with_field("push_constants")
            .with_detail(format!(
                "entry `{entry_point}`: missing SPIR-V type id {type_id}"
            ))
    })?;

    match inst.class.opcode {
        Op::TypeInt | Op::TypeFloat => {
            let bits = operand_u32(inst.operands.first()).unwrap_or(0);
            if matches!(bits, 8 | 16 | 32 | 64) {
                Ok(bits / 8)
            } else {
                Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_file)
                    .with_field("push_constants")
                    .with_detail(format!(
                        "entry `{entry_point}`: unsupported scalar bit width {bits}"
                    )))
            }
        }
        Op::TypeVector => {
            let Some(&Operand::IdRef(scalar_id)) = inst.operands.first() else {
                return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_file)
                    .with_field("push_constants")
                    .with_detail("malformed OpTypeVector"));
            };
            let count = operand_u32(inst.operands.get(1)).unwrap_or(0);
            if count == 0 {
                return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_file)
                    .with_field("push_constants")
                    .with_detail("vector component count cannot be 0"));
            }
            let scalar_bytes = compute_type_byte_size(
                source_file,
                entry_point,
                scalar_id,
                None,
                annotations,
                defs,
                depth + 1,
            )?;
            scalar_bytes.checked_mul(count).ok_or_else(|| {
                CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_file)
                    .with_field("push_constants")
                    .with_detail("vector byte size overflow")
            })
        }
        Op::TypeMatrix => {
            let Some(&Operand::IdRef(col_id)) = inst.operands.first() else {
                return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_file)
                    .with_field("push_constants")
                    .with_detail("malformed OpTypeMatrix"));
            };
            let cols = operand_u32(inst.operands.get(1)).unwrap_or(0);
            if cols == 0 {
                return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_file)
                    .with_field("push_constants")
                    .with_detail("matrix column count cannot be 0"));
            }
            let Some(vec_inst) = defs.get(&col_id).copied() else {
                return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_file)
                    .with_field("push_constants")
                    .with_detail(format!(
                        "entry `{entry_point}`: missing matrix column vector type `%id={col_id}`"
                    )));
            };
            if vec_inst.class.opcode != Op::TypeVector {
                return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_file)
                    .with_field("push_constants")
                    .with_detail("OpTypeMatrix column type is not OpTypeVector"));
            }
            let Some(&Operand::IdRef(scalar_id)) = vec_inst.operands.first() else {
                return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_file)
                    .with_field("push_constants")
                    .with_detail("malformed matrix column OpTypeVector"));
            };
            let rows = operand_u32(vec_inst.operands.get(1)).unwrap_or(0);
            if rows == 0 {
                return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_file)
                    .with_field("push_constants")
                    .with_detail("matrix row count cannot be 0"));
            }
            let scalar_size = compute_type_byte_size(
                source_file,
                entry_point,
                scalar_id,
                None,
                annotations,
                defs,
                depth + 1,
            )?;
            let Some(layout) = matrix_layout else {
                return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_file)
                    .with_field("push_constants")
                    .with_detail(format!(
                        "entry `{entry_point}`: matrix `%id={type_id}` in push-constant block is missing explicit `MatrixStride` decoration"
                    )));
            };
            let (major_count, vec_elems) = if layout.row_major {
                (rows, cols)
            } else {
                (cols, rows)
            };
            let vec_bytes = vec_elems.checked_mul(scalar_size).ok_or_else(|| {
                CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_file)
                    .with_field("push_constants")
                    .with_detail("matrix vector byte size overflow")
            })?;
            if layout.stride < vec_bytes {
                return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_file)
                    .with_field("push_constants")
                    .with_detail(format!(
                        "entry `{entry_point}`: MatrixStride {} is smaller than matrix vector size {vec_bytes} bytes",
                        layout.stride
                    )));
            }
            (major_count - 1)
                .checked_mul(layout.stride)
                .and_then(|prefix| prefix.checked_add(vec_bytes))
                .ok_or_else(|| {
                    CliError::new(CliErrorKind::ShaderReflectionFailed)
                        .with_file(source_file)
                        .with_field("push_constants")
                        .with_detail("strided matrix byte size overflow")
                })
        }
        Op::TypeArray => {
            let (Some(&Operand::IdRef(elem_id)), Some(&Operand::IdRef(len_id))) =
                (inst.operands.first(), inst.operands.get(1))
            else {
                return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_file)
                    .with_field("push_constants")
                    .with_detail("malformed OpTypeArray"));
            };
            let len = eval_constant_u32(source_file, entry_point, len_id, defs)?;
            if len == 0 {
                return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_file)
                    .with_field("push_constants")
                    .with_detail("array length cannot be 0"));
            }
            let elem_size = compute_type_byte_size(
                source_file,
                entry_point,
                elem_id,
                matrix_layout,
                annotations,
                defs,
                depth + 1,
            )?;
            if let Some(&stride) = annotations.array_strides.get(&type_id) {
                if stride < elem_size {
                    return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                        .with_file(source_file)
                        .with_field("push_constants")
                        .with_detail(format!(
                            "entry `{entry_point}`: ArrayStride {stride} is smaller than element size {elem_size} bytes"
                        )));
                }
                (len - 1)
                    .checked_mul(stride)
                    .and_then(|prefix| prefix.checked_add(elem_size))
                    .ok_or_else(|| {
                        CliError::new(CliErrorKind::ShaderReflectionFailed)
                            .with_file(source_file)
                            .with_field("push_constants")
                            .with_detail("strided array byte size overflow")
                    })
            } else {
                len.checked_mul(elem_size).ok_or_else(|| {
                    CliError::new(CliErrorKind::ShaderReflectionFailed)
                        .with_file(source_file)
                        .with_field("push_constants")
                        .with_detail("array byte size overflow")
                })
            }
        }
        Op::TypeStruct => {
            let mut max_end = 0u32;
            for (idx, op) in inst.operands.iter().enumerate() {
                let Operand::IdRef(member_type_id) = *op else {
                    continue;
                };
                let member_idx = u32::try_from(idx).unwrap_or(u32::MAX);
                let Some(&offset) = annotations.member_offsets.get(&(type_id, member_idx)) else {
                    return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                        .with_file(source_file)
                        .with_field("push_constants")
                        .with_detail(format!(
                            "entry `{entry_point}`: struct `%id={type_id}` member {member_idx} is missing explicit `Offset` decoration"
                        )));
                };
                let member_matrix_layout = annotations
                    .member_matrix_strides
                    .get(&(type_id, member_idx))
                    .map(|&stride| MemberMatrixLayout {
                        stride,
                        row_major: annotations
                            .member_row_major
                            .contains(&(type_id, member_idx)),
                    });
                let member_size = compute_type_byte_size(
                    source_file,
                    entry_point,
                    member_type_id,
                    member_matrix_layout,
                    annotations,
                    defs,
                    depth + 1,
                )?;
                let end = offset.checked_add(member_size).ok_or_else(|| {
                    CliError::new(CliErrorKind::ShaderReflectionFailed)
                        .with_file(source_file)
                        .with_field("push_constants")
                        .with_detail("struct member offset + size overflow")
                })?;
                max_end = max_end.max(end);
            }
            Ok(max_end)
        }
        other => Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
            .with_file(source_file)
            .with_field("push_constants")
            .with_detail(format!(
                "entry `{entry_point}`: unsupported SPIR-V type `{other:?}` inside push-constant block"
            ))),
    }
}

#[track_caller]
#[allow(clippy::too_many_arguments)]
fn extract_descriptor_bindings(
    source_file: &Path,
    stage: ShaderStage,
    entry_point: &str,
    global_vars: &[&Instruction],
    interface_ids: &BTreeSet<u32>,
    reachable_used_ids: &BTreeSet<u32>,
    annotations: &AnnotationTables,
    defs: &BTreeMap<u32, &Instruction>,
) -> Result<Vec<DescriptorBinding>, CliError> {
    let mut by_slot: BTreeMap<(u32, u32), DescriptorBinding> = BTreeMap::new();

    for var in global_vars {
        let Some(var_id) = var.result_id else {
            continue;
        };
        let Some(&Operand::StorageClass(storage_class)) = var.operands.first() else {
            continue;
        };
        if !matches!(
            storage_class,
            StorageClass::Uniform | StorageClass::StorageBuffer | StorageClass::UniformConstant
        ) {
            continue;
        }
        if !interface_ids.contains(&var_id) && !reachable_used_ids.contains(&var_id) {
            continue;
        }

        let Some(&set) = annotations.descriptor_sets.get(&var_id) else {
            return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                .with_file(source_file)
                .with_field("descriptor_binding")
                .with_detail(format!(
                    "entry `{entry_point}` ({stage}): resource variable `%id={var_id}` is missing `DescriptorSet` decoration"
                )));
        };
        let Some(&binding) = annotations.bindings.get(&var_id) else {
            return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                .with_file(source_file)
                .with_field("descriptor_binding")
                .with_detail(format!(
                    "entry `{entry_point}` ({stage}): resource variable `%id={var_id}` is missing `Binding` decoration"
                )));
        };

        let pointee_id = resolve_pointer_pointee(source_file, var, defs)?;
        let (inner_type_id, count) =
            unwrap_descriptor_array(source_file, entry_point, pointee_id, defs)?;
        let kind = classify_descriptor_kind(
            source_file,
            entry_point,
            set,
            binding,
            storage_class,
            inner_type_id,
            annotations,
            defs,
        )?;

        let record = DescriptorBinding {
            set,
            binding,
            kind,
            count,
            stages: stage.stage_flag(),
        };
        if by_slot.insert((set, binding), record).is_some() {
            return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                .with_file(source_file)
                .with_field("descriptor_binding")
                .with_detail(format!(
                    "entry `{entry_point}` ({stage}): duplicate descriptor binding `(set={set}, binding={binding})`"
                )));
        }
    }

    Ok(by_slot.into_values().collect())
}

#[track_caller]
fn unwrap_descriptor_array(
    source_file: &Path,
    entry_point: &str,
    type_id: u32,
    defs: &BTreeMap<u32, &Instruction>,
) -> Result<(u32, u32), CliError> {
    let inst = defs.get(&type_id).copied().ok_or_else(|| {
        CliError::new(CliErrorKind::ShaderReflectionFailed)
            .with_file(source_file)
            .with_field("descriptor_binding")
            .with_detail(format!(
                "entry `{entry_point}`: unknown resource type id {type_id}"
            ))
    })?;

    match inst.class.opcode {
        Op::TypeArray => {
            let (Some(&Operand::IdRef(elem_id)), Some(&Operand::IdRef(len_id))) =
                (inst.operands.first(), inst.operands.get(1))
            else {
                return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_file)
                    .with_field("descriptor_binding")
                    .with_detail("malformed descriptor OpTypeArray"));
            };
            let count = eval_constant_u32(source_file, entry_point, len_id, defs)?;
            if count == 0 || count == RUNTIME_DESCRIPTOR_COUNT {
                return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_file)
                    .with_field("descriptor_binding")
                    .with_detail(format!(
                        "entry `{entry_point}`: invalid fixed descriptor array count {count}"
                    )));
            }
            Ok((elem_id, count))
        }
        Op::TypeRuntimeArray => {
            let Some(&Operand::IdRef(elem_id)) = inst.operands.first() else {
                return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_file)
                    .with_field("descriptor_binding")
                    .with_detail("malformed descriptor OpTypeRuntimeArray"));
            };
            Ok((elem_id, RUNTIME_DESCRIPTOR_COUNT))
        }
        _ => Ok((type_id, 1)),
    }
}

#[track_caller]
#[allow(clippy::too_many_arguments)]
fn classify_descriptor_kind(
    source_file: &Path,
    entry_point: &str,
    set: u32,
    binding: u32,
    storage_class: StorageClass,
    inner_type_id: u32,
    annotations: &AnnotationTables,
    defs: &BTreeMap<u32, &Instruction>,
) -> Result<DescriptorKind, CliError> {
    match storage_class {
        StorageClass::StorageBuffer => Ok(DescriptorKind::StorageBuffer),
        StorageClass::Uniform => {
            if annotations.buffer_blocks.contains(&inner_type_id) {
                Ok(DescriptorKind::StorageBuffer)
            } else {
                Ok(DescriptorKind::UniformBuffer)
            }
        }
        StorageClass::UniformConstant => {
            let inst = defs.get(&inner_type_id).copied().ok_or_else(|| {
                CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_file)
                    .with_field("descriptor_binding")
                    .with_detail(format!(
                        "entry `{entry_point}`: unknown type id {inner_type_id} at `(set={set}, binding={binding})`"
                    ))
            })?;
            match inst.class.opcode {
                Op::TypeSampler => Ok(DescriptorKind::Sampler),
                Op::TypeSampledImage => Ok(DescriptorKind::CombinedImageSampler),
                Op::TypeImage => {
                    let sampled = operand_u32(inst.operands.get(5)).unwrap_or(0);
                    match sampled {
                        1 => Ok(DescriptorKind::SampledImage),
                        2 => Ok(DescriptorKind::StorageImage),
                        other => Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                            .with_file(source_file)
                            .with_field("descriptor_binding")
                            .with_detail(format!(
                                "entry `{entry_point}`: unsupported OpTypeImage `Sampled={other}` at `(set={set}, binding={binding})`"
                            ))),
                    }
                }
                other => Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_file)
                    .with_field("descriptor_binding")
                    .with_detail(format!(
                        "entry `{entry_point}`: unsupported UniformConstant resource `{other:?}` at `(set={set}, binding={binding})`"
                    ))),
            }
        }
        other => Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
            .with_file(source_file)
            .with_field("descriptor_binding")
            .with_detail(format!(
                "entry `{entry_point}`: unsupported descriptor storage class `{other:?}`"
            ))),
    }
}

#[track_caller]
fn resolve_pointer_pointee(
    source_file: &Path,
    var: &Instruction,
    defs: &BTreeMap<u32, &Instruction>,
) -> Result<u32, CliError> {
    let ptr_id = var.result_type.ok_or_else(|| {
        CliError::new(CliErrorKind::ShaderReflectionFailed)
            .with_file(source_file)
            .with_field("spirv_variable")
            .with_detail("OpVariable missing result pointer type")
    })?;
    let ptr_inst = defs.get(&ptr_id).copied().ok_or_else(|| {
        CliError::new(CliErrorKind::ShaderReflectionFailed)
            .with_file(source_file)
            .with_field("spirv_variable")
            .with_detail(format!("missing OpTypePointer definition `%id={ptr_id}`"))
    })?;
    if ptr_inst.class.opcode != Op::TypePointer {
        return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
            .with_file(source_file)
            .with_field("spirv_variable")
            .with_detail(format!(
                "expected OpTypePointer on `%id={ptr_id}`, found `{:?}`",
                ptr_inst.class.opcode
            )));
    }
    let Some(&Operand::IdRef(pointee_id)) = ptr_inst.operands.get(1) else {
        return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
            .with_file(source_file)
            .with_field("spirv_variable")
            .with_detail("OpTypePointer missing pointee type operand"));
    };
    Ok(pointee_id)
}

#[track_caller]
fn eval_constant_u32(
    source_file: &Path,
    entry_point: &str,
    const_id: u32,
    defs: &BTreeMap<u32, &Instruction>,
) -> Result<u32, CliError> {
    let inst = defs.get(&const_id).copied().ok_or_else(|| {
        CliError::new(CliErrorKind::ShaderReflectionFailed)
            .with_file(source_file)
            .with_field("spirv_constant")
            .with_detail(format!(
                "entry `{entry_point}`: missing constant `%id={const_id}`"
            ))
    })?;
    if !matches!(inst.class.opcode, Op::Constant | Op::SpecConstant) {
        return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
            .with_file(source_file)
            .with_field("spirv_constant")
            .with_detail(format!(
                "entry `{entry_point}`: array length `%id={const_id}` is `{:?}`, expected scalar constant",
                inst.class.opcode
            )));
    }
    let type_id = inst.result_type.ok_or_else(|| {
        CliError::new(CliErrorKind::ShaderReflectionFailed)
            .with_file(source_file)
            .with_field("spirv_constant")
            .with_detail(format!(
                "entry `{entry_point}`: constant `%id={const_id}` is missing result_type"
            ))
    })?;
    let type_inst = defs.get(&type_id).copied().ok_or_else(|| {
        CliError::new(CliErrorKind::ShaderReflectionFailed)
            .with_file(source_file)
            .with_field("spirv_constant")
            .with_detail(format!(
                "entry `{entry_point}`: missing constant type `%id={type_id}`"
            ))
    })?;
    if type_inst.class.opcode != Op::TypeInt {
        return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
            .with_file(source_file)
            .with_field("spirv_constant")
            .with_detail(format!(
                "entry `{entry_point}`: constant `%id={const_id}` has non-integer type `{:?}`",
                type_inst.class.opcode
            )));
    }
    let width = operand_u32(type_inst.operands.first()).unwrap_or(0);
    let signedness = operand_u32(type_inst.operands.get(1)).unwrap_or(u32::MAX);
    if !matches!(signedness, 0 | 1) {
        return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
            .with_file(source_file)
            .with_field("spirv_constant")
            .with_detail(format!(
                "entry `{entry_point}`: OpTypeInt `%id={type_id}` has invalid signedness {signedness}"
            )));
    }

    match (width, inst.operands.first()) {
        (8 | 16 | 32, Some(&Operand::LiteralBit32(val))) => {
            if signedness == 1 {
                let signed = match width {
                    8 => i64::from(val as i8),
                    16 => i64::from(val as i16),
                    _ => i64::from(val as i32),
                };
                if signed < 0 {
                    return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                        .with_file(source_file)
                        .with_field("spirv_constant")
                        .with_detail(format!(
                            "entry `{entry_point}`: negative signed constant {signed} on `%id={const_id}`"
                        )));
                }
                u32::try_from(signed).map_err(|_| {
                    CliError::new(CliErrorKind::ShaderReflectionFailed)
                        .with_file(source_file)
                        .with_field("spirv_constant")
                        .with_detail("signed integer constant overflows u32")
                })
            } else {
                let mask = match width {
                    8 => 0xFF,
                    16 => 0xFFFF,
                    _ => u32::MAX,
                };
                Ok(val & mask)
            }
        }
        (64, Some(&Operand::LiteralBit64(val))) => {
            if signedness == 1 && (val as i64) < 0 {
                return Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_file)
                    .with_field("spirv_constant")
                    .with_detail(format!(
                        "entry `{entry_point}`: negative 64-bit signed constant {} on `%id={const_id}`",
                        val as i64
                    )));
            }
            u32::try_from(val).map_err(|_| {
                CliError::new(CliErrorKind::ShaderReflectionFailed)
                    .with_file(source_file)
                    .with_field("spirv_constant")
                    .with_detail(format!(
                        "entry `{entry_point}`: 64-bit constant {val} overflows u32"
                    ))
            })
        }
        _ => Err(CliError::new(CliErrorKind::ShaderReflectionFailed)
            .with_file(source_file)
            .with_field("spirv_constant")
            .with_detail(format!(
                "entry `{entry_point}`: unsupported {width}-bit constant literal on `%id={const_id}`"
            ))),
    }
}

fn operand_u32(op: Option<&Operand>) -> Option<u32> {
    match op {
        Some(&Operand::LiteralBit32(v)) => Some(v),
        _ => None,
    }
}
