//! STL export: every solid of a built scene, in world space, converted to
//! millimetres (STL carries no unit, and mm is what readers assume) and
//! written as binary STL.
//!
//! No reorientation (scenes are Z-up right-handed, as STL consumers expect)
//! and no recentering (1:1 coordinates keep multi-file exports registered).
//! Color and opacity are ignored. The bytes are a pure function of the scene
//! and the options — no timestamp.

use crate::scene;
use odm_build::Units;
use odm_ir::{Mesh, Node, Transform};
use odm_kernel::{CancelToken, Kernel, KernelError};
use odm_store::Store;
use std::path::{Path, PathBuf};

/// The export's options — one struct shared by the `export` request and the
/// viewer dialog, so a new option is added in one place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StlOptions {
    /// What one model unit is; the file is in mm.
    pub units: Units,
    /// Fuse all solids into one manifold (overlaps merge, disjoint parts stay
    /// separate bodies). Off = every solid as-is: exact, but overlapping
    /// siblings self-intersect.
    pub union: bool,
}

#[derive(Clone, Debug)]
pub struct StlReport {
    pub path: PathBuf,
    /// The options as resolved.
    pub units: Units,
    pub union: bool,
    pub size_mm: [f64; 3],
    /// With union off, `volume_mm3` and `bodies` are sums over the solids.
    pub volume_mm3: f64,
    pub tris: usize,
    pub bodies: usize,
    pub warnings: Vec<String>,
}

impl StlReport {
    /// `80 × 60 × 4.2 mm · 1 body · 2,312 triangles`
    pub fn line(&self) -> String {
        let body = if self.bodies == 1 { "body" } else { "bodies" };
        let tri = if self.tris == 1 { "triangle" } else { "triangles" };
        format!(
            "{} mm · {} {body} · {} {tri}",
            fmt_size(self.size_mm),
            self.bodies,
            thousands(self.tris)
        )
    }
}

/// A model smaller than this, or larger than `MAX_SIDE_MM`, on its longest
/// side is almost always the wrong unit.
const MIN_SIDE_MM: f64 = 1.0;
const MAX_SIDE_MM: f64 = 2000.0;

/// The wrong-unit catch, shared by the report and the dialog's live size line.
pub fn size_warning(size_mm: [f64; 3]) -> Option<String> {
    let longest = size_mm.iter().copied().fold(0.0, f64::max);
    let which = if longest < MIN_SIDE_MM {
        "under 1 mm"
    } else if longest > MAX_SIDE_MM {
        "over 2000 mm"
    } else {
        return None;
    };
    Some(format!("the longest side is {which} ({} mm) — check `units` in odm.toml", fmt_num(longest)))
}

/// `80 × 60 × 4.2`
pub fn fmt_size(size: [f64; 3]) -> String {
    format!("{} × {} × {}", fmt_num(size[0]), fmt_num(size[1]), fmt_num(size[2]))
}

/// Four significant digits, no trailing zeros.
fn fmt_num(v: f64) -> String {
    if v == 0.0 || !v.is_finite() {
        return format!("{v}");
    }
    let digits = (3 - v.abs().log10().floor() as i32).clamp(0, 9) as usize;
    let s = format!("{v:.digits$}");
    match s.contains('.') {
        true => s.trim_end_matches('0').trim_end_matches('.').to_owned(),
        false => s,
    }
}

fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Export the scene under `root` to `out`. `label` (`<project> <view path>`)
/// goes in the file's header. The caller keeps `root` alive in the store (a
/// `RootPin`) for the duration; nothing is added to the store.
pub fn export_stl(
    store: &Store,
    kernel: &Kernel,
    root: &Node,
    label: &str,
    out: &Path,
    opts: &StlOptions,
    cancel: Option<&CancelToken>,
) -> Result<StlReport, String> {
    let mut operands = scene::scene_solids(store, root)?;
    if operands.is_empty() {
        return Err("nothing to export (the scene has no solids)".to_owned());
    }
    let scale = opts.units.to_mm();
    for (_, t) in &mut operands {
        *t = scaled(t, scale);
    }
    let solids = kernel.export_solids(&operands, opts.union, cancel).map_err(|e| match e {
        KernelError::Cancelled => "export cancelled".to_owned(),
        e => e.to_string(),
    })?;

    let mut tris = Tris::default();
    for solid in &solids {
        tris.add(&solid.mesh);
    }
    if tris.bytes.is_empty() {
        return Err("nothing to export (the scene's solids are empty)".to_owned());
    }
    let count = u32::try_from(tris.count)
        .map_err(|_| format!("{} triangles is more than an STL file can hold", tris.count))?;

    let mut file = Vec::with_capacity(84 + tris.bytes.len());
    file.extend_from_slice(&header(label));
    file.extend_from_slice(&count.to_le_bytes());
    file.extend_from_slice(&tris.bytes);
    if cancel.is_some_and(|c| c.is_cancelled()) {
        return Err("export cancelled".to_owned());
    }
    write_atomic(out, &file)?;

    let bodies = solids.iter().map(|s| s.bodies).sum();
    let size_mm = [0, 1, 2].map(|k| tris.max[k] - tris.min[k]);
    let mut warnings = Vec::new();
    if !opts.union && solids.len() > 1 {
        warnings.push(format!("{} solids written as-is; overlaps are not fused", solids.len()));
    }
    warnings.extend(size_warning(size_mm));
    if tris.dropped > 0 {
        warnings.push(format!(
            "{} triangles too small for STL's 32-bit coordinates were dropped",
            tris.dropped
        ));
    }
    Ok(StlReport {
        path: out.to_path_buf(),
        units: opts.units,
        union: opts.union,
        size_mm,
        volume_mm3: solids.iter().map(|s| s.volume).sum(),
        tris: tris.count,
        bodies,
        warnings,
    })
}

/// `scale` applied after `t` (column-major; the bottom row stays put).
fn scaled(t: &Transform, scale: f64) -> Transform {
    let mut m = t.0;
    for (i, e) in m.iter_mut().enumerate() {
        if i % 4 < 3 {
            *e *= scale;
        }
    }
    Transform(m)
}

/// The 80-byte header: `ODM <label>`, ASCII only, zero-padded. Starting with
/// `ODM` is what keeps it from starting with `solid`, which readers take for
/// the ASCII format.
fn header(label: &str) -> [u8; 80] {
    let mut out = [0u8; 80];
    let text = format!("ODM {label}");
    let bytes = text.chars().map(|c| if c.is_ascii_graphic() || c == ' ' { c as u8 } else { b'?' });
    for (slot, b) in out.iter_mut().zip(bytes) {
        *slot = b;
    }
    out
}

/// The file's triangle records, accumulated across solids.
#[derive(Default)]
struct Tris {
    bytes: Vec<u8>,
    count: usize,
    dropped: usize,
    min: [f64; 3],
    max: [f64; 3],
}

impl Tris {
    fn add(&mut self, mesh: &Mesh) {
        if self.count == 0 && self.dropped == 0 {
            (self.min, self.max) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        }
        let vertex = |i: u32| -> [f64; 3] {
            let at = i as usize * 3;
            [mesh.positions[at], mesh.positions[at + 1], mesh.positions[at + 2]]
        };
        for p in mesh.positions.chunks_exact(3) {
            for k in 0..3 {
                self.min[k] = self.min[k].min(p[k]);
                self.max[k] = self.max[k].max(p[k]);
            }
        }
        for tri in mesh.indices.chunks_exact(3) {
            let v = [vertex(tri[0]), vertex(tri[1]), vertex(tri[2])];
            // The one narrowing. Only an edge collapsed to a single f32
            // point drops its triangle — that takes both triangles on the
            // edge, so the surface stays closed. A collinear sliver stays:
            // dropping it would open a hole.
            let n = v.map(|p| p.map(|c| c as f32));
            if n[0] == n[1] || n[1] == n[2] || n[2] == n[0] {
                self.dropped += 1;
                continue;
            }
            for c in normal(&v) {
                self.bytes.extend_from_slice(&c.to_le_bytes());
            }
            for c in n.iter().flatten() {
                self.bytes.extend_from_slice(&c.to_le_bytes());
            }
            self.bytes.extend_from_slice(&[0, 0]); // attribute byte count
            self.count += 1;
        }
    }
}

/// Unit normal from the f64 positions (before narrowing); zero for a
/// zero-area triangle — readers recompute normals anyway.
fn normal(v: &[[f64; 3]; 3]) -> [f32; 3] {
    let a = [0, 1, 2].map(|k| v[1][k] - v[0][k]);
    let b = [0, 1, 2].map(|k| v[2][k] - v[0][k]);
    let n = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len > 0.0 && len.is_finite() {
        // `+ 0.0` turns a -0.0 component into 0.0.
        n.map(|c| (c / len + 0.0) as f32)
    } else {
        [0.0; 3]
    }
}

/// Temp file beside the target, then rename: the target may be open in a
/// program that watches it, and must never be seen half-written.
fn write_atomic(out: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = out.file_name().ok_or_else(|| format!("{} is not a file path", out.display()))?;
    let tmp = out.with_file_name(format!(".{}.tmp{}", name.to_string_lossy(), std::process::id()));
    let result = std::fs::write(&tmp, bytes)
        .and_then(|()| std::fs::rename(&tmp, out))
        .map_err(|e| format!("write {}: {e}", out.display()));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use odm_ir::Hash;
    use odm_store::Object;
    use std::sync::Arc;

    struct Fixture {
        store: Arc<Store>,
        kernel: Arc<Kernel>,
        dir: tempfile::TempDir,
    }

    fn fixture() -> Fixture {
        let store = Store::new();
        Fixture { kernel: Kernel::new(store.clone()), store, dir: tempfile::tempdir().unwrap() }
    }

    fn moved(x: f64) -> Transform {
        let mut t = Transform::IDENTITY;
        t.0[12] = x;
        t
    }

    impl Fixture {
        /// A root holding one node per (mesh, transform).
        fn scene(&self, parts: &[(Hash, Transform)]) -> Node {
            let children = parts
                .iter()
                .map(|&(mesh, transform)| {
                    let node = Node { transform, mesh: Some(mesh), ..Node::default() };
                    self.store.put(Object::Node(node))
                })
                .collect();
            Node { children, ..Node::default() }
        }

        fn cube(&self, side: f64) -> Hash {
            self.kernel.cube(side, side, side, false).unwrap()
        }

        fn export(&self, root: &Node, units: Units, union: bool) -> (StlReport, Vec<u8>) {
            let out = self.dir.path().join("part.stl");
            let opts = StlOptions { units, union };
            let report =
                export_stl(&self.store, &self.kernel, root, "proj root.js", &out, &opts, None)
                    .unwrap();
            (report, std::fs::read(&out).unwrap())
        }
    }

    /// (normal, vertices) per record.
    fn parse(file: &[u8]) -> Vec<([f32; 3], [[f32; 3]; 3])> {
        let count = u32::from_le_bytes(file[80..84].try_into().unwrap()) as usize;
        assert_eq!(file.len(), 84 + 50 * count, "50 bytes per triangle");
        let f = |at: usize| f32::from_le_bytes(file[at..at + 4].try_into().unwrap());
        (0..count)
            .map(|i| {
                let at = 84 + 50 * i;
                assert_eq!(&file[at + 48..at + 50], &[0, 0]);
                let v = |j: usize| [f(at + 12 * j), f(at + 12 * j + 4), f(at + 12 * j + 8)];
                (v(0), [v(1), v(2), v(3)])
            })
            .collect()
    }

    /// Signed volume of the file's triangles.
    fn volume(tris: &[([f32; 3], [[f32; 3]; 3])]) -> f64 {
        tris.iter()
            .map(|(_, v)| {
                let [a, b, c] = v.map(|p| p.map(f64::from));
                (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]))
                    / 6.0
            })
            .sum()
    }

    /// The file welds back into a solid: watertight after f32 narrowing.
    fn reweld(f: &Fixture, tris: &[([f32; 3], [[f32; 3]; 3])]) -> Hash {
        let positions: Vec<f64> =
            tris.iter().flat_map(|(_, v)| v.iter().flatten().map(|&c| c as f64)).collect();
        let indices: Vec<u32> = (0..tris.len() as u32 * 3).collect();
        f.kernel.solid_from_mesh(&positions, &indices).expect("the STL is watertight")
    }

    #[test]
    fn a_cube_is_twelve_outward_triangles_under_a_plain_header() {
        let f = fixture();
        let root = f.scene(&[(f.cube(20.0), Transform::IDENTITY)]);
        let (report, file) = f.export(&root, Units::Mm, true);

        let mut head = [0u8; 80];
        head[..16].copy_from_slice(b"ODM proj root.js");
        assert_eq!(&file[..80], &head);
        let tris = parse(&file);
        assert_eq!(tris.len(), 12);
        let center = [10.0f32; 3];
        for (n, v) in &tris {
            // Axis-aligned unit normal, pointing away from the center.
            assert_eq!(n.iter().filter(|c| c.abs() == 1.0).count(), 1, "{n:?}");
            assert_eq!(n.iter().filter(|c| **c == 0.0).count(), 2, "{n:?}");
            let out: f32 = (0..3).map(|k| n[k] * (v[0][k] - center[k])).sum();
            assert!(out > 0.0, "{n:?} at {v:?}");
        }
        assert!((volume(&tris) - 8000.0).abs() < 1e-6);
        assert_eq!(report.size_mm, [20.0; 3]);
        assert_eq!((report.tris, report.bodies), (12, 1));
        assert!((report.volume_mm3 - 8000.0).abs() < 1e-9);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(report.line(), "20 × 20 × 20 mm · 1 body · 12 triangles");
        reweld(&f, &tris);

        // No timestamp, no temp file left, nothing added to the store.
        let objects = f.store.object_count();
        assert_eq!(f.export(&root, Units::Mm, true).1, file);
        assert_eq!(f.store.object_count(), objects);
        let left: Vec<_> = std::fs::read_dir(f.dir.path()).unwrap().collect();
        assert_eq!(left.len(), 1, "{left:?}");
    }

    #[test]
    fn units_scale_to_millimetres() {
        let f = fixture();
        let root = f.scene(&[(f.cube(0.02), Transform::IDENTITY)]);
        let (m, _) = f.export(&root, Units::M, true);
        assert!(m.size_mm.iter().all(|s| (s - 20.0).abs() < 1e-12), "{:?}", m.size_mm);
        assert!((m.volume_mm3 - 8000.0).abs() < 1e-6);
        assert!(m.warnings.is_empty(), "{:?}", m.warnings);

        let (ft, _) = f.export(&root, Units::Ft, true);
        assert!((ft.size_mm[0] - 6.096).abs() < 1e-12);

        let (mm, _) = f.export(&root, Units::Mm, true);
        assert_eq!(mm.size_mm, [0.02; 3]);
        assert!(mm.warnings.iter().any(|w| w.contains("check `units`")), "{:?}", mm.warnings);

        let big = f.scene(&[(f.cube(3.0), Transform::IDENTITY)]);
        let (big, _) = f.export(&big, Units::M, true);
        assert!(big.warnings.iter().any(|w| w.contains("over 2000 mm")), "{:?}", big.warnings);
    }

    #[test]
    fn union_fuses_overlaps_and_counts_loose_bodies() {
        let f = fixture();
        let cube = f.cube(10.0);
        let root = f.scene(&[(cube, Transform::IDENTITY), (cube, moved(5.0))]);
        let (fused, file) = f.export(&root, Units::Mm, true);
        assert_eq!(fused.bodies, 1);
        assert!((fused.volume_mm3 - 1500.0).abs() < 1e-9);
        assert!(fused.warnings.is_empty(), "{:?}", fused.warnings);
        assert!((volume(&parse(&file)) - 1500.0).abs() < 1e-6);
        reweld(&f, &parse(&file));

        // Union off: both shells, as they are; the options echo back.
        let (raw, file) = f.export(&root, Units::Mm, false);
        assert_eq!((raw.tris, raw.bodies, raw.union), (24, 2, false));
        assert_eq!(parse(&file).len(), 24);
        assert!((raw.volume_mm3 - 2000.0).abs() < 1e-9);
        assert!(raw.warnings.iter().any(|w| w.contains("2 solids written as-is")));

        let apart = f.scene(&[(cube, Transform::IDENTITY), (cube, moved(50.0))]);
        let (apart, _) = f.export(&apart, Units::Mm, true);
        assert_eq!(apart.bodies, 2);
        assert!(apart.warnings.is_empty(), "{:?}", apart.warnings);
        assert_eq!(apart.size_mm, [60.0, 10.0, 10.0]);
    }

    /// A negative determinant flips winding; the file must still be outward.
    #[test]
    fn a_mirrored_instance_keeps_positive_volume() {
        let f = fixture();
        let mut mirror = Transform::IDENTITY;
        mirror.0[0] = -1.0;
        for union in [true, false] {
            let root = f.scene(&[(f.cube(10.0), mirror)]);
            let (_, file) = f.export(&root, Units::Mm, union);
            assert!((volume(&parse(&file)) - 1000.0).abs() < 1e-6);
        }
    }

    /// An edge shorter than f32 resolution where it sits collapses; both its
    /// triangles go, and what is left still closes.
    #[test]
    fn an_edge_below_f32_resolution_drops_both_its_triangles() {
        let f = fixture();
        // A square pyramid whose apex is split into two points 1e-6 apart:
        // far below f32 spacing at x = 1000.
        let (a, b) = (1000.0, 1000.0 + 1e-6);
        #[rustfmt::skip]
        let positions = [
            999.0, -1.0, 0.0,   1001.0, -1.0, 0.0,   1001.0, 1.0, 0.0,   999.0, 1.0, 0.0,
            a, 0.0, 1.0,   b, 0.0, 1.0,
        ];
        #[rustfmt::skip]
        let indices = [
            0, 2, 1,  0, 3, 2,          // base
            0, 1, 4,  1, 5, 4,          // front, split along the apex edge
            1, 2, 5,
            2, 3, 5,  3, 4, 5,          // back
            3, 0, 4,
        ];
        let mesh = f.kernel.solid_from_mesh(&positions, &indices).unwrap();
        let root = f.scene(&[(mesh, Transform::IDENTITY)]);
        let (report, file) = f.export(&root, Units::Mm, false);
        assert_eq!(report.tris, 6);
        assert!(report.warnings.iter().any(|w| w.starts_with("2 triangles")), "{:?}", report.warnings);
        reweld(&f, &parse(&file));
    }

    #[test]
    fn a_scene_without_solids_is_an_error() {
        let f = fixture();
        let out = f.dir.path().join("part.stl");
        let opts = StlOptions { units: Units::Mm, union: true };
        let err = export_stl(&f.store, &f.kernel, &Node::default(), "p", &out, &opts, None)
            .unwrap_err();
        assert!(err.contains("nothing to export"), "{err}");
        assert!(!out.exists());
    }

    #[test]
    fn a_cancelled_export_writes_nothing() {
        let f = fixture();
        let root = f.scene(&[(f.cube(10.0), Transform::IDENTITY)]);
        let out = f.dir.path().join("part.stl");
        let cancel = CancelToken::new();
        cancel.cancel();
        let opts = StlOptions { units: Units::Mm, union: true };
        let err = export_stl(&f.store, &f.kernel, &root, "p", &out, &opts, Some(&cancel))
            .unwrap_err();
        assert!(err.contains("cancelled"), "{err}");
        assert_eq!(std::fs::read_dir(f.dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn the_header_is_ascii_padded_and_never_reads_as_ascii_stl() {
        let h = header("solid café\n");
        assert_eq!(&h[..16], b"ODM solid caf??\0");
        assert_eq!(header(&"x".repeat(200)).len(), 80);
        assert_eq!(fmt_size([0.08, 60.0, 2312.7]), "0.08 × 60 × 2313");
        assert_eq!(thousands(1234567), "1,234,567");
    }
}
