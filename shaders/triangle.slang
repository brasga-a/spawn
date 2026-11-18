// Milestone 2 Phase 2/3 unindexed colored triangle shader (ADR-0004, ADR-0014).
//
// Conventions:
// - Uses `SV_VulkanVertexID` (`BuiltIn VertexIndex`) so zero vertex buffer
//   bindings and only baseline `OpCapability Shader` are required.
// - Clip-space positions use Vulkan NDC directly (+X right, +Y down, reversed-Z
//   depth z = 0.5 in [0, 1]); no Y-flip is performed in shader code (SPN-INV-053).
// - Vertex order (top, bottom-left, bottom-right) produces positive Vulkan
//   signed area (`a > 0`), matching `VK_FRONT_FACE_COUNTER_CLOCKWISE`.

struct VertexOutput {
    float4 clip_position : SV_Position;
    [[vk::location(0)]] float3 color : COLOR0;
};

[shader("vertex")]
VertexOutput vs_main(uint vertex_id : SV_VulkanVertexID) {
    static const float2 positions[3] = {
        float2(0.0, -0.5),
        float2(-0.5, 0.5),
        float2(0.5, 0.5),
    };
    static const float3 colors[3] = {
        float3(1.0, 0.2, 0.2),
        float3(0.2, 1.0, 0.2),
        float3(0.2, 0.4, 1.0),
    };

    uint index = min(vertex_id, 2u);
    VertexOutput out_vertex;
    out_vertex.clip_position = float4(positions[index], 0.5, 1.0);
    out_vertex.color = colors[index];
    return out_vertex;
}

[shader("fragment")]
float4 fs_main(VertexOutput input) : SV_Target0 {
    return float4(input.color, 1.0);
}
