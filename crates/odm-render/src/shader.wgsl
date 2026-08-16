struct Globals {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    // xy = internal viewport size in pixels, z = supersample factor
    // (screen-space pixel measures divide by z to stay in output pixels).
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
// Supersampled premultiplied compose result + its factor k.
@group(3) @binding(3) var super_tex: texture_2d<f32>;
@group(3) @binding(4) var<uniform> super_k: vec4<u32>;

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

struct WireOut {
    // Same invariance need as VsOut: line depths must agree between the
    // peel and tail pipelines.
    @invariant @builtin(position) clip: vec4<f32>,
    // Signed pixel distance from the line centerline; +-(half_width + 0.5)
    // at the quad edges. Coverage alpha comes from this.
    @location(0) edge: f32,
    // Grid minor density fade: 1 where lines are sparse enough to draw, 0
    // where they would moire into each other.
    @location(1) fade: f32,
};

// One line segment: an instance holding both endpoints, drawn as a 4-vertex
// triangle strip expanded across the segment to the slot's pixel width plus
// half a pixel of analytic-AA feather each side. Line primitives are always
// one pixel wide in WebGPU, hence the quad.
@vertex
fn vs_wire(
    @builtin(vertex_index) vi: u32,
    @location(0) a: vec3<f32>,
    @location(1) b: vec3<f32>,
) -> WireOut {
    let wa = inst.world * vec4<f32>(a, 1.0);
    let wb = inst.world * vec4<f32>(b, 1.0);
    var ca = globals.view_proj * wa;
    var cb = globals.view_proj * wb;

    var out: WireOut;
    out.edge = 0.0;
    out.fade = 1.0;
    // Both ends at or behind the eye: emit an off-range triangle to be clipped.
    let eps = 1e-6;
    if (ca.w < eps && cb.w < eps) {
        out.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
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
    let reach = inst.params.x + 0.5;
    let offset = vec2<f32>(-dir.y, dir.x) * reach * side;

    let at_b = vi >= 2u;
    let clip = select(ca, cb, at_b);
    out.clip = vec4<f32>(clip.xy + offset / half_px * clip.w, clip.zw);
    out.edge = reach * side;

    // Density fade: params.y is the world-space spacing to the neighbouring
    // parallel line (grid minors). Project that offset at this end; when it
    // lands under a few pixels the lines would moire, so they melt away.
    let spacing = inst.params.y;
    if (spacing > 0.0) {
        let end = select(wa.xyz, wb.xyz, at_b);
        let along = wb.xyz - wa.xyz;
        let perp = normalize(cross(along, vec3<f32>(0.0, 0.0, 1.0))) * spacing;
        let cn = globals.view_proj * vec4<f32>(end + perp, 1.0);
        let here = clip.xy / max(clip.w, eps) * half_px;
        let there = cn.xy / max(cn.w, eps) * half_px;
        out.fade = smoothstep(2.0, 8.0, distance(here, there) / globals.viewport.z);
    }
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

// Line fragments are translucent by construction: coverage alpha is the
// analytic AA. Fully covered fragments still carry the slot's alpha, and
// faded-out grid minors discard so they don't occupy a peel layer.
@fragment
fn fs_line_translucent(in: WireOut) -> @location(0) vec4<f32> {
    if (peeled_or_hidden(in.clip)) {
        discard;
    }
    let coverage = clamp(inst.params.x + 0.5 - abs(in.edge), 0.0, 1.0);
    let a = inst.color.a * coverage * in.fade;
    if (a <= 0.0) {
        discard;
    }
    return vec4<f32>(inst.color.rgb * a, a);
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

fn compose_at(px: vec2<i32>) -> vec4<f32> {
    let acc = textureLoad(accum_tex, px, 0);
    return acc + (1.0 - acc.a) * textureLoad(opaque_tex, px, 0);
}

fn unpremultiply(c: vec4<f32>) -> vec4<f32> {
    if (c.a <= 0.0) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(c.rgb / c.a, c.a);
}

// Final compose: translucent accumulation over the opaque pass (both
// premultiplied; the opaque target was cleared to the premultiplied
// background), un-premultiplied for the sRGB target.
@fragment
fn fs_compose(in: FsQuad) -> @location(0) vec4<f32> {
    return unpremultiply(compose_at(vec2<i32>(in.pos.xy)));
}

// Supersampled variant: same compose, kept premultiplied linear so the
// downsample can average correctly.
@fragment
fn fs_compose_premul(in: FsQuad) -> @location(0) vec4<f32> {
    return compose_at(vec2<i32>(in.pos.xy));
}

// Box-downsample k x k premultiplied texels into one output pixel.
@fragment
fn fs_downsample(in: FsQuad) -> @location(0) vec4<f32> {
    let k = super_k.x;
    let base = vec2<u32>(in.pos.xy) * k;
    var sum = vec4<f32>(0.0);
    for (var y = 0u; y < k; y++) {
        for (var x = 0u; x < k; x++) {
            sum += textureLoad(super_tex, vec2<i32>(base + vec2<u32>(x, y)), 0);
        }
    }
    return unpremultiply(sum / f32(k * k));
}
