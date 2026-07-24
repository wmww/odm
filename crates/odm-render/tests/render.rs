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
    let root = Node {
        children: vec![
            Node {
                mesh: Some(cube),
                color: Some(Color { r: 0.2, g: 0.4, b: 0.8, a: 1.0 }),
                ..Default::default()
            },
            Node { mesh: Some(cyl), transform: off, ..Default::default() },
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
    };
    renderer.render_png(&scene, &opts).unwrap();

    // Empty scene renders too (grid + background only).
    let empty_store = Store::new();
    let empty_root = empty_store.put(Object::Node(Node::default()));
    let empty = flatten_scene(&empty_store, empty_root).unwrap();
    let opts = RenderOptions::default_with(64, 64);
    renderer.render_png(&empty, &opts).unwrap();
}

#[test]
fn bad_size_rejected() {
    let Some(mut renderer) = renderer_or_skip() else { return };
    let (store, root) = demo_scene();
    let scene = flatten_scene(&store, root).unwrap();
    let opts = RenderOptions::default_with(0, 100);
    assert!(renderer.render_png(&scene, &opts).is_err());
}
