use odm_ir::{Color, Node, Transform};
use odm_render::{Camera, Projection, RenderOptions, Renderer, flatten_scene};
use odm_store::{Object, Store};

/// Store with a colored cube + cylinder scene; returns (store, root hash).
fn demo_scene() -> (std::sync::Arc<Store>, odm_ir::Hash) {
    let store = Store::new();
    let kernel = odm_kernel::Kernel::new(store.clone());
    let cube = kernel.cube(2.0, 2.0, 2.0, true).unwrap();
    let cyl = kernel.cylinder(3.0, 0.8, 0.8, 48, false).unwrap();
    let mut off = Transform::IDENTITY;
    off.0[12] = 2.5;
    let child = |n: Node| store.put(Object::Node(n));
    let root = Node {
        children: vec![
            child(Node {
                mesh: Some(cube),
                color: Some(Color { r: 0.2, g: 0.4, b: 0.8, a: 1.0 }),
                ..Default::default()
            }),
            child(Node { mesh: Some(cyl), transform: off, ..Default::default() }),
        ],
        ..Default::default()
    };
    let root_hash = store.put(Object::Node(root));
    (store, root_hash)
}

fn renderer_or_skip() -> Option<Renderer> {
    match Renderer::new() {
        Ok(r) => Some(r),
        Err(e) => {
            eprintln!("skipping render test (no GPU adapter): {e}");
            None
        }
    }
}

fn decode(png_bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

#[test]
fn render_smoke_and_determinism() {
    let Some(mut renderer) = renderer_or_skip() else { return };
    let (store, root) = demo_scene();
    let scene = flatten_scene(&store, root).unwrap();
    assert_eq!(scene.instances.len(), 2);

    let opts = RenderOptions::default_with(320, 240);
    let png1 = renderer.render_png(&scene, &opts).unwrap();
    let (w, h, pixels) = decode(&png1);
    assert_eq!((w, h), (320, 240));

    let distinct: std::collections::HashSet<&[u8]> = pixels.chunks_exact(4).collect();
    assert!(distinct.len() > 1, "image is a single flat color — nothing rendered");

    let png2 = renderer.render_png(&scene, &opts).unwrap();
    assert_eq!(png1, png2, "same adapter renders must be byte-identical");
}

#[test]
fn option_variants_render() {
    let Some(mut renderer) = renderer_or_skip() else { return };
    let (store, root) = demo_scene();
    let scene = flatten_scene(&store, root).unwrap();

    // Ortho auto camera.
    let mut opts = RenderOptions::default_with(160, 120);
    opts.camera = Camera::Auto { direction: [0.0, 0.0, -1.0], ortho: true };
    renderer.render_png(&scene, &opts).unwrap();

    // Wireframe (edges only).
    let mut opts = RenderOptions::default_with(160, 120);
    opts.wireframe = true;
    renderer.render_png(&scene, &opts).unwrap();

    // Explicit camera, no grid.
    let opts = RenderOptions {
        width: 160,
        height: 120,
        camera: Camera::Explicit {
            eye: [5.0, 5.0, 4.0],
            target: [0.0, 0.0, 0.0],
            up: [0.0, 0.0, 1.0],
            projection: Projection::Perspective { fov_y_deg: 35.0 },
        },
        wireframe: false,
        grid: false,
        background: [0.0, 0.0, 0.0, 1.0],
        opacity: 1.0,
        peel_layers: 4,
    };
    renderer.render_png(&scene, &opts).unwrap();

    // Empty scene renders too (grid + background only).
    let empty_store = Store::new();
    let empty_root = empty_store.put(Object::Node(Node::default()));
    let empty = flatten_scene(&empty_store, empty_root).unwrap();
    let opts = RenderOptions::default_with(64, 64);
    renderer.render_png(&empty, &opts).unwrap();
}

/// Top-down ortho over stacked axis-aligned boxes: top faces shade to
/// exactly 1.0 at the image center (n == v), so blend math is checkable to
/// within sRGB quantization.
fn stacked_scene(layers: &[(f64, [f32; 4])]) -> (std::sync::Arc<Store>, odm_ir::Hash) {
    let store = Store::new();
    let kernel = odm_kernel::Kernel::new(store.clone());
    let child = |n: Node| store.put(Object::Node(n));
    let children = layers
        .iter()
        .map(|(z, c)| {
            let slab = kernel.cube(10.0, 10.0, 1.0, true).unwrap();
            let mut off = Transform::IDENTITY;
            off.0[14] = *z;
            child(Node {
                mesh: Some(slab),
                transform: off,
                color: Some(Color { r: c[0], g: c[1], b: c[2], a: c[3] }),
                ..Default::default()
            })
        })
        .collect();
    let root_hash = store.put(Object::Node(Node { children, ..Default::default() }));
    (store, root_hash)
}

fn top_down_opts(width: u32, height: u32) -> RenderOptions {
    let mut opts = RenderOptions::default_with(width, height);
    opts.camera = Camera::Auto { direction: [0.0, 0.0, -1.0], ortho: true };
    opts.grid = false;
    opts.background = [0.0, 0.0, 0.0, 1.0];
    opts
}

fn center_pixel(png: &[u8]) -> [u8; 4] {
    let (w, h, pixels) = decode(png);
    let i = ((h / 2 * w + w / 2) * 4) as usize;
    pixels[i..i + 4].try_into().unwrap()
}

/// linear -> 8-bit sRGB, the encoding the readback applies.
fn srgb8(linear: f64) -> f64 {
    let s = if linear <= 0.0031308 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    s * 255.0
}

fn assert_near(got: [u8; 4], want: [f64; 4], what: &str) {
    for (g, w) in got.iter().zip(want) {
        assert!(
            (*g as f64 - w).abs() <= 2.0,
            "{what}: got {got:?}, want ~{want:?}"
        );
    }
}

#[test]
fn translucent_blend_math() {
    let Some(mut renderer) = renderer_or_skip() else { return };
    // Opaque red below, 50% green above: center = 0.5 green + 0.5 red.
    let (store, root) = stacked_scene(&[
        (0.0, [1.0, 0.0, 0.0, 1.0]),
        (3.0, [0.0, 1.0, 0.0, 0.5]),
    ]);
    let scene = flatten_scene(&store, root).unwrap();
    let opts = top_down_opts(160, 160);
    let png = renderer.render_png(&scene, &opts).unwrap();
    let want = srgb8(0.5);
    assert_near(center_pixel(&png), [want, want, 0.0, 255.0], "green(0.5) over red");

    // Three stacked 50% layers over opaque red — needs correct front-to-back
    // ordering: 0.5*g1 + 0.25*g2 + 0.125*g3 + 0.125*red.
    let (store, root) = stacked_scene(&[
        (0.0, [1.0, 0.0, 0.0, 1.0]),
        (2.0, [0.0, 1.0, 0.0, 0.5]),
        (4.0, [0.0, 0.0, 1.0, 0.5]),
        (6.0, [1.0, 1.0, 1.0, 0.5]),
    ]);
    let scene = flatten_scene(&store, root).unwrap();
    let png = renderer.render_png(&scene, &opts).unwrap();
    // Front-to-back: white 0.5, blue 0.25, green 0.125, red 0.125.
    let want = [
        srgb8(0.5 * 1.0 + 0.125 * 1.0),
        srgb8(0.5 * 1.0 + 0.125 * 1.0),
        srgb8(0.5 * 1.0 + 0.25 * 1.0),
        255.0,
    ];
    assert_near(center_pixel(&png), want, "three peeled layers");
}

#[test]
fn xray_opacity_option() {
    let Some(mut renderer) = renderer_or_skip() else { return };
    // One opaque green slab; opts.opacity 0.5 turns it translucent over black.
    let (store, root) = stacked_scene(&[(0.0, [0.0, 1.0, 0.0, 1.0])]);
    let scene = flatten_scene(&store, root).unwrap();
    let mut opts = top_down_opts(64, 64);
    opts.opacity = 0.5;
    let png = renderer.render_png(&scene, &opts).unwrap();
    assert_near(center_pixel(&png), [0.0, srgb8(0.5), 0.0, 255.0], "x-ray over black");
}

#[test]
fn translucent_determinism_and_depth_invariance() {
    let Some(mut renderer) = renderer_or_skip() else { return };
    // Interpenetrating translucent boxes — the case draw-order sorting gets
    // wrong and peeling must not.
    let store = Store::new();
    let kernel = odm_kernel::Kernel::new(store.clone());
    let child = |n: Node| store.put(Object::Node(n));
    let mut tilt = Transform::IDENTITY;
    // Rotate ~30 deg around X so the two boxes interpenetrate.
    let (s, c) = (0.5f64, 0.75f64.sqrt());
    tilt.0[5] = c;
    tilt.0[6] = s;
    tilt.0[9] = -s;
    tilt.0[10] = c;
    let cube = kernel.cube(4.0, 4.0, 4.0, true).unwrap();
    let root = store.put(Object::Node(Node {
        children: vec![
            child(Node {
                mesh: Some(cube),
                color: Some(Color { r: 1.0, g: 0.2, b: 0.2, a: 0.4 }),
                ..Default::default()
            }),
            child(Node {
                mesh: Some(cube),
                transform: tilt,
                color: Some(Color { r: 0.2, g: 0.2, b: 1.0, a: 0.4 }),
                ..Default::default()
            }),
        ],
        ..Default::default()
    }));
    let scene = flatten_scene(&store, root).unwrap();
    let opts = RenderOptions::default_with(200, 150);
    let a = renderer.render_png(&scene, &opts).unwrap();
    let b = renderer.render_png(&scene, &opts).unwrap();
    assert_eq!(a, b, "translucent renders must be byte-identical");

    // Invariance canary: a single translucent surface must render identically
    // at N=1 and N=4 — if depths are not bit-identical across passes, the
    // surface re-composites once per layer and the image darkens with N.
    let (store, root) = stacked_scene(&[(0.0, [0.0, 1.0, 0.0, 0.5])]);
    let scene = flatten_scene(&store, root).unwrap();
    let mut opts = top_down_opts(160, 160);
    opts.peel_layers = 1;
    let n1 = renderer.render_png(&scene, &opts).unwrap();
    opts.peel_layers = 4;
    let n4 = renderer.render_png(&scene, &opts).unwrap();
    assert_eq!(n1, n4, "peel layer count must not change a single-surface render");
}

#[test]
fn transparent_background_png_alpha() {
    let Some(mut renderer) = renderer_or_skip() else { return };
    let (store, root) = stacked_scene(&[(0.0, [0.0, 1.0, 0.0, 0.5])]);
    let scene = flatten_scene(&store, root).unwrap();
    let mut opts = top_down_opts(160, 160);
    opts.background = [0.0, 0.0, 0.0, 0.0];
    let png = renderer.render_png(&scene, &opts).unwrap();
    // Over nothing: straight green at alpha 127/128.
    assert_near(center_pixel(&png), [0.0, srgb8(1.0), 0.0, 127.5], "unpremultiplied green");
    // A corner shows pure transparent background.
    let (_, _, pixels) = decode(&png);
    assert_eq!(&pixels[..4], &[0, 0, 0, 0], "empty pixels stay fully transparent");
}

#[test]
fn bad_size_rejected() {
    let Some(mut renderer) = renderer_or_skip() else { return };
    let (store, root) = demo_scene();
    let scene = flatten_scene(&store, root).unwrap();
    let opts = RenderOptions::default_with(0, 100);
    assert!(renderer.render_png(&scene, &opts).is_err());
}
