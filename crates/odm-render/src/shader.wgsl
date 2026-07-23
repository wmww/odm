struct Globals {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct InstanceData {
    world: mat4x4<f32>,
    color: vec4<f32>,
};
@group(1) @binding(0) var<uniform> inst: InstanceData;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec3<f32>) -> VsOut {
    var out: VsOut;
    let wp = inst.world * vec4<f32>(pos, 1.0);
    out.world_pos = wp.xyz;
    out.clip = globals.view_proj * wp;
    return out;
}

// Flat shading from screen-space derivatives; no vertex normals needed.
@fragment
fn fs_mesh(in: VsOut) -> @location(0) vec4<f32> {
    var n = normalize(cross(dpdx(in.world_pos), dpdy(in.world_pos)));
    let v = normalize(globals.camera_pos.xyz - in.world_pos);
    if (dot(n, v) < 0.0) {
        n = -n;
    }
    let shade = 0.35 + 0.65 * max(dot(n, v), 0.0);
    return vec4<f32>(inst.color.rgb * shade, 1.0);
}

@fragment
fn fs_flat(in: VsOut) -> @location(0) vec4<f32> {
    return inst.color;
}
