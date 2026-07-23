use odm_ir::{Color, Mesh, Node, Transform};
use odm_render::{DEFAULT_COLOR, RenderError, flatten_scene};
use odm_store::{Object, Store};

fn tri_mesh(x_off: f32) -> Mesh {
    Mesh {
        positions: vec![x_off, 0.0, 0.0, x_off + 1.0, 0.0, 0.0, x_off, 1.0, 0.0],
        indices: vec![0, 1, 2],
    }
}

fn translation(x: f64, y: f64, z: f64) -> Transform {
    let mut t = Transform::IDENTITY;
    t.0[12] = x;
    t.0[13] = y;
    t.0[14] = z;
    t
}

#[test]
fn flatten_accumulates_transforms_and_colors() {
    let store = Store::new();
    let mesh = store.put(Object::Mesh(tri_mesh(0.0).into()));
    let empty = store.put(Object::Mesh(Mesh { positions: vec![], indices: vec![] }.into()));

    let red = Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 };
    let blue = Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 };
    let root = Node {
        transform: translation(10.0, 0.0, 0.0),
        color: Some(red),
        children: vec![
            // Inherits red, nested translation.
            Node { transform: translation(0.0, 5.0, 0.0), mesh: Some(mesh), ..Default::default() },
            // Own color wins over inherited.
            Node { color: Some(blue), mesh: Some(mesh), ..Default::default() },
            // Empty mesh: skipped.
            Node { mesh: Some(empty), ..Default::default() },
        ],
        ..Default::default()
    };
    let root_hash = store.put(Object::Node(root));

    let scene = flatten_scene(&store, root_hash).unwrap();
    assert_eq!(scene.instances.len(), 2, "empty mesh must be skipped");
    assert_eq!(scene.meshes.len(), 1, "shared mesh stored once");

    let first = &scene.instances[0];
    assert_eq!(first.color, [1.0, 0.0, 0.0, 1.0], "inherited");
    assert_eq!(first.transform[3][0], 10.0, "x translation accumulated");
    assert_eq!(first.transform[3][1], 5.0, "y translation accumulated");

    let second = &scene.instances[1];
    assert_eq!(second.color, [0.0, 0.0, 1.0, 1.0], "own color wins");

    // Bounds: triangle spans x 0..1, y 0..1 locally; instances at
    // (10,5) and (10,0).
    let (min, max) = scene.bounds.unwrap();
    assert_eq!(min, [10.0, 0.0, 0.0]);
    assert_eq!(max, [11.0, 6.0, 0.0]);
}

#[test]
fn colorless_gets_default() {
    let store = Store::new();
    let mesh = store.put(Object::Mesh(tri_mesh(0.0).into()));
    let root = store.put(Object::Node(Node { mesh: Some(mesh), ..Default::default() }));
    let scene = flatten_scene(&store, root).unwrap();
    assert_eq!(scene.instances[0].color, DEFAULT_COLOR);
}

#[test]
fn missing_objects_error() {
    let store = Store::new();
    let bogus = odm_ir::Hash::of_bytes(b"nope");
    assert!(matches!(flatten_scene(&store, bogus), Err(RenderError::MissingObject(_))));

    let root = store.put(Object::Node(Node { mesh: Some(bogus), ..Default::default() }));
    assert!(matches!(flatten_scene(&store, root), Err(RenderError::MissingObject(_))));
}

#[test]
fn mesh_as_root_rejected() {
    let store = Store::new();
    let mesh = store.put(Object::Mesh(tri_mesh(0.0).into()));
    assert!(matches!(flatten_scene(&store, mesh), Err(RenderError::BadScene(_))));
}
