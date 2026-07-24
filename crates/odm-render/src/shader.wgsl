struct Globals {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    // xy = viewport size in pixels, z = half wire width in pixels.
    viewport: vec4<f32>,
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

// One wire: an instance holding both endpoints, drawn as a 4-vertex triangle
// strip expanded to a fixed pixel width across the segment. Line primitives
// are always one pixel wide in WebGPU, hence the quad.
@vertex
fn vs_wire(
    @builtin(vertex_index) vi: u32,
    @location(0) a: vec3<f32>,
    @location(1) b: vec3<f32>,
) -> VsOut {
    let wa = inst.world * vec4<f32>(a, 1.0);
    let wb = inst.world * vec4<f32>(b, 1.0);
    var ca = globals.view_proj * wa;
    var cb = globals.view_proj * wb;

    var out: VsOut;
    // Both ends at or behind the eye: emit an off-range triangle to be clipped.
    let eps = 1e-6;
    if (ca.w < eps && cb.w < eps) {
        out.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
        out.world_pos = wa.xyz;
        return out;
    }
    // One end behind: pull it up to the near plane so the segment stays sane.
    if (ca.w < eps) {
        ca = mix(ca, cb, (eps - ca.w) / (cb.w - ca.w));
    } else if (cb.w < eps) {
        cb = mix(cb, ca, (eps - cb.w) / (ca.w - cb.w));
    }

    // Perpendicular offset, computed in pixels then folded back into clip space.
    let half_px = globals.viewport.xy * 0.5;
    let delta = cb.xy / cb.w * half_px - ca.xy / ca.w * half_px;
    let len = length(delta);
    let dir = select(vec2<f32>(1.0, 0.0), delta / len, len > 1e-6);
    let side = select(-1.0, 1.0, (vi & 1u) == 1u);
    let offset = vec2<f32>(-dir.y, dir.x) * globals.viewport.z * side;

    let at_b = vi >= 2u;
    let clip = select(ca, cb, at_b);
    out.clip = vec4<f32>(clip.xy + offset / half_px * clip.w, clip.zw);
    out.world_pos = select(wa.xyz, wb.xyz, at_b);
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
