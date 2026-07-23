use crate::Hash;
use serde_json::Value;
use std::collections::BTreeMap;

/// Bump when the canonical encoding of any type changes.
const FORMAT_VERSION: u8 = 2;

// Type tags for domain separation.
pub(crate) mod tag {
    pub const MESH: u8 = 1;
    pub const NODE: u8 = 2;
    pub const COLOR: u8 = 3;
    pub const TRANSFORM: u8 = 4;
    pub const JSON_NULL: u8 = 16;
    pub const JSON_BOOL: u8 = 17;
    pub const JSON_NUMBER: u8 = 18;
    pub const JSON_STRING: u8 = 19;
    pub const JSON_ARRAY: u8 = 20;
    pub const JSON_OBJECT: u8 = 21;
}

/// Canonical-encoding writer feeding a blake3 hasher.
pub struct Hasher(blake3::Hasher);

impl Hasher {
    pub fn new() -> Self {
        let mut h = blake3::Hasher::new();
        h.update(&[FORMAT_VERSION]);
        Hasher(h)
    }

    pub fn u8(&mut self, v: u8) {
        self.0.update(&[v]);
    }
    pub fn u32(&mut self, v: u32) {
        self.0.update(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.0.update(&v.to_le_bytes());
    }
    pub fn f32(&mut self, v: f32) {
        self.0.update(&v.to_bits().to_le_bytes());
    }
    pub fn f64(&mut self, v: f64) {
        self.0.update(&v.to_bits().to_le_bytes());
    }
    pub fn len(&mut self, v: usize) {
        self.u64(v as u64);
    }
    pub fn str(&mut self, v: &str) {
        self.len(v.len());
        self.0.update(v.as_bytes());
    }
    pub fn hash(&mut self, v: &Hash) {
        self.0.update(&v.0);
    }
    pub fn f32s(&mut self, v: &[f32]) {
        self.len(v.len());
        for &x in v {
            self.f32(x);
        }
    }
    pub fn u32s(&mut self, v: &[u32]) {
        self.len(v.len());
        for &x in v {
            self.u32(x);
        }
    }
    pub fn opt<T: Canonical>(&mut self, v: &Option<T>) {
        match v {
            None => self.u8(0),
            Some(x) => {
                self.u8(1);
                x.write(self);
            }
        }
    }

    pub fn finish(self) -> Hash {
        Hash(*self.0.finalize().as_bytes())
    }
}

impl Default for Hasher {
    fn default() -> Self {
        Self::new()
    }
}

pub trait Canonical {
    fn write(&self, w: &mut Hasher);

    fn hash(&self) -> Hash {
        let mut w = Hasher::new();
        self.write(&mut w);
        w.finish()
    }
}

impl Canonical for Hash {
    fn write(&self, w: &mut Hasher) {
        w.hash(self);
    }
}

impl Canonical for String {
    fn write(&self, w: &mut Hasher) {
        w.str(self);
    }
}

impl Canonical for Value {
    fn write(&self, w: &mut Hasher) {
        match self {
            Value::Null => w.u8(tag::JSON_NULL),
            Value::Bool(b) => {
                w.u8(tag::JSON_BOOL);
                w.u8(*b as u8);
            }
            Value::Number(n) => {
                w.u8(tag::JSON_NUMBER);
                // All JS numbers are f64; hash the f64 bits so 1 and 1.0 agree.
                w.f64(n.as_f64().unwrap_or(f64::NAN));
            }
            Value::String(s) => {
                w.u8(tag::JSON_STRING);
                w.str(s);
            }
            Value::Array(a) => {
                w.u8(tag::JSON_ARRAY);
                w.len(a.len());
                for v in a {
                    v.write(w);
                }
            }
            Value::Object(m) => {
                w.u8(tag::JSON_OBJECT);
                // Sort keys for canonical order regardless of insertion order.
                let sorted: BTreeMap<&String, &Value> = m.iter().collect();
                w.len(sorted.len());
                for (k, v) in sorted {
                    w.str(k);
                    v.write(w);
                }
            }
        }
    }
}
