//! Wireframe mode: edges only, and picking that hits the wires themselves.

use odm_ir::{Node, Transform};
use odm_render::{
    Camera, DEFAULT_COLOR, Projection, RenderOptions, Renderer, WIRE_WIDTH_PX, flatten_scene,
    pick_wire,
};
use odm_store::{Object, Store};

const PX: u32 = 200;
/// Ortho half-height 5 over 200 px.
const PX_PER_UNIT: f64 = 20.0;

fn at(x: f64, y: f64) -> [f64; 2] {
    [PX as f64 / 2.0 + x * PX_PER_UNIT, PX as f64 / 2.0 - y * PX_PER_UNIT]
}

fn translated(x: f64, y: f64, z: f64) -> Transform {
    let mut t = Transform::IDENTITY;
    (t.0[12], t.0[13], t.0[14]) = (x, y, z);
    t
}

/// Three cubes stacked along the view axis, seen straight down -z in ortho:
/// 0 = big at z=0, 1 = small behind it, 2 = big far behind (projects exactly
/// onto cube 0).
fn stacked_scene() -> (std::sync::Arc<Store>, odm_ir::Hash) {
    let store = Store::new();
    let kernel = odm_kernel::Kernel::new(store.clone());
    let big = kernel.cube(4.0, 4.0, 4.0, true).unwrap();
    let small = kernel.cube(0.6, 0.6, 0.6, true).unwrap();
    let child = |n: Node| store.put(Object::Node(n));
    let root = Node {
        children: vec![
            child(Node { mesh: Some(big), ..Default::default() }),
            child(Node {
                mesh: Some(small),
                transform: translated(1.0, -0.3, -5.0),
                ..Default::default()
            }),
            child(Node {
                mesh: Some(big),
                transform: translated(0.0, 0.0, -10.0),
                ..Default::default()
            }),
        ],
        ..Default::default()
    };
    let root_hash = store.put(Object::Node(root));
    (store, root_hash)
}

fn ortho_opts() -> RenderOptions {
    let mut opts = RenderOptions::default_with(PX, PX);
    opts.camera = Camera::Explicit {
        eye: [0.0, 0.0, 20.0],
        target: [0.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        projection: Projection::Orthographic { height: 10.0 },
    };
    opts
}

#[test]
fn picks_the_wire_nearest_the_click() {
    let (store, root) = stacked_scene();
    let scene = flatten_scene(&store, root).unwrap();
    let opts = ortho_opts();
    let pick = |p: [f64; 2]| pick_wire(&scene, &opts, p, 6.0).map(|h| h.instance);

    // Top edge of the small cube: it is fully occluded by cube 0, but no wire
    // of cube 0 runs anywhere near, so it is still selectable.
    assert_eq!(pick(at(1.0, 0.0)), Some(1));

    // Silhouette shared by cubes 0 and 2 — the near one wins.
    assert_eq!(pick(at(-2.0, 0.0)), Some(0));

    // Empty space.
    assert_eq!(pick(at(4.5, 4.5)), None);

    // Just outside the pick radius of the same silhouette edge.
    assert_eq!(pick(at(-2.5, 0.0)), None);
}

fn render_rgba(renderer: &mut Renderer, scene: &odm_render::RenderScene, opts: &RenderOptions) -> Vec<u8> {
    let png = renderer.render_png(scene, opts).unwrap();
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    buf
}

#[test]
fn wireframe_draws_no_fill() {
    let Ok(mut renderer) = Renderer::new() else {
        eprintln!("skipping (no GPU adapter)");
        return;
    };
    let (store, root) = stacked_scene();
    let scene = flatten_scene(&store, root).unwrap();

    let mut opts = ortho_opts();
    opts.grid = false;
    let count_drawn = |px: &[u8]| {
        let bg = &px[..4];
        px.chunks_exact(4).filter(|p| *p != bg).count()
    };
    let solid = count_drawn(&render_rgba(&mut renderer, &scene, &opts));
    opts.wireframe = true;
    let wire = count_drawn(&render_rgba(&mut renderer, &scene, &opts));

    assert!(wire > 0, "wireframe drew nothing");
    assert!(
        wire * 2 < solid,
        "wireframe covers {wire} px vs solid {solid} — surfaces still filled?"
    );
}

/// Wires are screen-space quads, so their width is exact: measure the ink laid
/// down across a scanline cutting the big cube's left silhouette edge.
#[test]
fn wires_are_wire_width_px_wide() {
    let Ok(mut renderer) = Renderer::new() else {
        eprintln!("skipping (no GPU adapter)");
        return;
    };
    let (store, root) = stacked_scene();
    let scene = flatten_scene(&store, root).unwrap();
    let mut opts = ortho_opts();
    opts.grid = false;
    opts.wireframe = true;
    let px = render_rgba(&mut renderer, &scene, &opts);

    // Coverage sums linearly, so decode out of sRGB before adding it up.
    let srgb_to_linear = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let bg = opts.background[0];
    let ink = DEFAULT_COLOR[0] - bg;
    // Row at world y = 1.0, across the silhouette at world x = -2 (nothing
    // else is drawn within ±0.5 world units of it).
    let row = at(0.0, 1.0)[1] as usize;
    let coverage: f32 = (50..70)
        .map(|x| {
            let p = (row * PX as usize + x) * 4;
            (srgb_to_linear(px[p]) - bg) / ink
        })
        .sum();

    assert!(
        (coverage - WIRE_WIDTH_PX).abs() < 0.75,
        "wire measured {coverage:.2} px wide, expected {WIRE_WIDTH_PX}"
    );
}
