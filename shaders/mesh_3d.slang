// Milestone 2 Phase 2/4/5 camera-relative 3D mesh shader (ADR-0004, ADR-0010, ADR-0014).
//
// Conventions (SPN-INV-050..053):
// - Right-handed, Y-up, -Z forward; column-major matrices (`mul(M, v)` with
//   `slangc -matrix-layout-column-major`).
// - `mvp_camera_relative` transforms local `f32` vertex positions through a
//   camera-relative model transform (`mesh_world_f64 - cam_eye_f64` computed on
//   the CPU in `f64` before `f32` conversion); absolute world positions never
//   enter the GPU (SPN-INV-052).
// - Reversed-Z `[0, 1]` clip depth and the single-site Vulkan NDC Y-flip live
//   exclusively in `spawn-math`'s projection matrix; this shader performs no
//   axis sign hacks (SPN-INV-053).
// - `MeshPushConstants` occupies exactly 128 bytes (Vulkan 1.3 baseline minimum
//   `maxPushConstantsSize`), aligned on 16-byte boundaries:
//     * `0..64`   : `mvp_camera_relative` (`float4x4`)
//     * `64..112` : columns 0..2 of the upper-left 3x3 world normal basis (`xyz`);
//                   `normal_col0.w` blends between flat unlit color (`0.0`, for
//                   deterministic pixel-readback tests) and directional shading (`1.0`).
//     * `112..128`: linear RGBA `base_color` multiplier.

struct MeshPushConstants {
    float4x4 mvp_camera_relative;
    float4 normal_col0;
    float4 normal_col1;
    float4 normal_col2;
    float4 base_color;
};

[[vk::push_constant]]
MeshPushConstants pc;

struct VertexInput {
    [[vk::location(0)]] float3 position : POSITION;
    [[vk::location(1)]] float3 normal : NORMAL;
    [[vk::location(2)]] float4 color : COLOR0;
};

struct VertexOutput {
    float4 clip_position : SV_Position;
    [[vk::location(0)]] float3 world_normal : NORMAL;
    [[vk::location(1)]] float4 color : COLOR0;
    [[vk::location(2)]] nointerpolation float shade_weight : TEXCOORD0;
};

[shader("vertex")]
VertexOutput vs_main(VertexInput input) {
    VertexOutput out_vertex;
    out_vertex.clip_position = mul(pc.mvp_camera_relative, float4(input.position, 1.0));
    out_vertex.world_normal =
        pc.normal_col0.xyz * input.normal.x +
        pc.normal_col1.xyz * input.normal.y +
        pc.normal_col2.xyz * input.normal.z;
    out_vertex.color = input.color * pc.base_color;
    out_vertex.shade_weight = saturate(pc.normal_col0.w);
    return out_vertex;
}

[shader("fragment")]
float4 fs_main(VertexOutput input) : SV_Target0 {
    float len_sq = dot(input.world_normal, input.world_normal);
    float3 unit_normal =
        (len_sq > 1e-12) ? (input.world_normal * rsqrt(len_sq)) : float3(0.0, 1.0, 0.0);
    float3 light_dir = normalize(float3(0.45, 0.80, 0.39));
    float lit = lerp(0.35, 1.0, saturate(dot(unit_normal, light_dir)));
    float shade = lerp(1.0, lit, input.shade_weight);
    return float4(input.color.rgb * shade, input.color.a);
}
