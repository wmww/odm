struct Globals {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    // xy = viewport size in pixels; zw unused.
    viewport: vec4<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct InstanceData {
    world: mat4x4<f32>,
    // Linear rgb + effective alpha (node alpha x render opacity).
    color: vec4<f32>,
    // Line-quad state: x = half line width in pixels; yzw unused.
    params: vec4<f32>,
};
@group(1) @binding(0) var<uniform> inst: InstanceData;

// Depth peeling inputs (peel geometry + tail passes).
@group(2) @binding(0) var prev_peel: texture_depth_2d;
@group(2) @binding(1) var opaque_depth: texture_depth_2d;

// Fullscreen composite inputs.
@group(3) @binding(0) var layer_tex: texture_2d<f32>;
@group(3) @binding(1) var accum_tex: texture_2d<f32>;
@group(3) @binding(2) var opaque_tex: texture_2d<f32>;

struct VsOut {
    // @invariant: peeling compares depths of the same triangle across
    // pipelines (peel layers vs tail); a ±1ulp drift would re-peel
    // already-composited surfaces once per layer.
    @invariant @builtin(position) clip: vec4<f32>,
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

// One line segment: an instance holding both endpoints, drawn as a 4-vertex
// triangle strip expanded to a fixed pixel width across the segment. Line
// primitives are always one pixel wide in WebGPU, hence the quad.
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
    let offset = vec2<f32>(-dir.y, dir.x) * inst.params.x * side;

    let at_b = vi >= 2u;
    let clip = select(ca, cb, at_b);
    out.clip = vec4<f32>(clip.xy + offset / half_px * clip.w, clip.zw);
    out.world_pos = select(wa.xyz, wb.xyz, at_b);
    return out;
}

// Flat shading from screen-space derivatives; no vertex normals needed.
fn mesh_shade(world_pos: vec3<f32>) -> f32 {
    var n = normalize(cross(dpdx(world_pos), dpdy(world_pos)));
    let v = normalize(globals.camera_pos.xyz - world_pos);
    if (dot(n, v) < 0.0) {
        n = -n;
    }
    return 0.35 + 0.65 * max(dot(n, v), 0.0);
}

// Opaque pass outputs are premultiplied (alpha 1 makes that a no-op).
@fragment
fn fs_mesh(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(inst.color.rgb * mesh_shade(in.world_pos), 1.0);
}

@fragment
fn fs_flat(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(inst.color.rgb, 1.0);
}

// True where a translucent fragment was already composited (at or nearer
// than the previous peel depth) or is hidden behind opaque geometry. Exact
// equality is what collapses coplanar translucent surfaces into one layer.
fn peeled_or_hidden(clip: vec4<f32>) -> bool {
    let px = vec2<i32>(clip.xy);
    return clip.z <= textureLoad(prev_peel, px, 0)
        || clip.z >= textureLoad(opaque_depth, px, 0);
}

// Translucent mesh fragments, premultiplied. Used by both the per-layer peel
// pipeline (blend replace + depth picks the single nearest fragment) and the
// tail pipeline (blend under, no depth).
@fragment
fn fs_mesh_translucent(in: VsOut) -> @location(0) vec4<f32> {
    if (peeled_or_hidden(in.clip)) {
        discard;
    }
    let a = inst.color.a;
    return vec4<f32>(inst.color.rgb * mesh_shade(in.world_pos) * a, a);
}

// ---------- fullscreen passes ----------

struct FsQuad {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) vi: u32) -> FsQuad {
    var out: FsQuad;
    out.pos = vec4<f32>(
        select(-1.0, 3.0, vi == 1u),
        select(-1.0, 3.0, vi == 2u),
        0.0,
        1.0,
    );
    return out;
}

// One peel layer composited under the accumulation; the under-blend state
// does the math (dst + (1 - dst.a) * src).
@fragment
fn fs_layer(in: FsQuad) -> @location(0) vec4<f32> {
    return textureLoad(layer_tex, vec2<i32>(in.pos.xy), 0);
}

// Final compose: translucent accumulation over the opaque pass (both
// premultiplied; the opaque target was cleared to the premultiplied
// background), un-premultiplied for the sRGB target.
@fragment
fn fs_compose(in: FsQuad) -> @location(0) vec4<f32> {
    let px = vec2<i32>(in.pos.xy);
    let acc = textureLoad(accum_tex, px, 0);
    let c = acc + (1.0 - acc.a) * textureLoad(opaque_tex, px, 0);
    if (c.a <= 0.0) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(c.rgb / c.a, c.a);
}
