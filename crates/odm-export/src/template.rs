//! The web-export template as ONE file, so installing it is copying a file
//! and there is never a pile of stale versions: a JSON header line (magic,
//! stamp, entry names + lengths) followed by the entries' bytes. xtask packs
//! it; `export_web` unpacks it next to the project bundle after checking the
//! stamp.

use serde_json::{Value, json};

const MAGIC: &str = "odm-web-template/1";

/// The files a template carries, in pack order. `xtask build-web-template`
/// reads exactly these; the exporter writes whatever the header names (so a
/// newer template can add files without changing old exporters — the stamp
/// gates real skew).
pub const TEMPLATE_FILES: &[&str] = &["index.html", "runtime.js", "odm_web.js", "odm_web_bg.wasm"];

pub fn pack(stamp: &str, files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let header = json!({
        "magic": MAGIC,
        "stamp": stamp,
        "files": files.iter().map(|(n, b)| json!({ "name": n, "len": b.len() })).collect::<Vec<_>>(),
    });
    let mut out = serde_json::to_string(&header).expect("header json").into_bytes();
    out.push(b'\n');
    for (_, bytes) in files {
        out.extend_from_slice(bytes);
    }
    out
}

#[derive(Debug)]
pub struct Template {
    pub stamp: String,
    pub files: Vec<(String, Vec<u8>)>,
}

pub fn unpack(data: &[u8]) -> Result<Template, String> {
    let nl = data
        .iter()
        .position(|&b| b == b'\n')
        .ok_or("not a web-export template (no header)")?;
    let header: Value = serde_json::from_slice(&data[..nl])
        .map_err(|_| "not a web-export template (bad header)".to_string())?;
    if header.get("magic").and_then(|m| m.as_str()) != Some(MAGIC) {
        return Err("not a web-export template (wrong magic)".into());
    }
    let stamp = header
        .get("stamp")
        .and_then(|s| s.as_str())
        .ok_or("template header has no stamp")?
        .to_string();
    let mut files = Vec::new();
    let mut at = nl + 1;
    for entry in header.get("files").and_then(|f| f.as_array()).ok_or("template header has no files")? {
        let name = entry.get("name").and_then(|n| n.as_str()).ok_or("entry has no name")?;
        let len = entry.get("len").and_then(|l| l.as_u64()).ok_or("entry has no len")? as usize;
        // Entries land in the export's output dir: flat names only.
        if name.contains('/') || name.contains('\\') || name.starts_with('.') {
            return Err(format!("template entry has a suspicious name {name:?}"));
        }
        let end = at.checked_add(len).filter(|&e| e <= data.len()).ok_or_else(|| {
            format!("template truncated in entry {name:?}")
        })?;
        files.push((name.to_string(), data[at..end].to_vec()));
        at = end;
    }
    if at != data.len() {
        return Err("template has trailing bytes after its entries".into());
    }
    Ok(Template { stamp, files })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let files: Vec<(&str, Vec<u8>)> =
            vec![("a.html", b"<html>".to_vec()), ("b.wasm", vec![0, 159, 146, 150])];
        let packed = pack("deadbeef", &files);
        let t = unpack(&packed).unwrap();
        assert_eq!(t.stamp, "deadbeef");
        assert_eq!(t.files.len(), 2);
        assert_eq!(t.files[0], ("a.html".to_string(), b"<html>".to_vec()));
        assert_eq!(t.files[1].1, vec![0, 159, 146, 150]);
    }

    #[test]
    fn rejects_junk() {
        assert!(unpack(b"").is_err());
        assert!(unpack(b"hello\nworld").is_err());
        let mut short = pack("s", &[("f", vec![1, 2, 3])]);
        short.truncate(short.len() - 1);
        assert!(unpack(&short).unwrap_err().contains("truncated"));
    }
}
