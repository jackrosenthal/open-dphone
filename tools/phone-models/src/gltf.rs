//! A model as binary glTF 2.0 (`.glb`).
//!
//! glTF's frame is y up, meters, with the model's front toward +z.  The
//! vertices are quantized (KHR_mesh_quantization): positions as 16-bit
//! integers that the node's scale and translation turn back into meters,
//! normals as 8-bit integers, and texture coordinates as 16-bit integers.  The
//! world's frame (millimeters, z up, y toward the back) turns into glTF's as
//! (x, z, -y) / 1000.  The desk is at y = 0, and the model's footprint is
//! centered on the y axis, the axis the homepage turns the model around.

use crate::geom::{Mesh, V3, v3};
use crate::phone::{Model, to_world};
use serde_json::{Value, json};

pub struct Glb {
    pub bytes: Vec<u8>,
    /// The farthest any part reaches from the y axis, and the model's
    /// height, in meters.
    pub radius: f64,
    pub height: f64,
    /// The model's bounding box (meters): min x, y, z, then max x, y, z.
    pub bounds: [f64; 6],
}

fn gl(v: V3) -> V3 {
    v3(v.x, v.z, -v.y) * 0.001
}

#[derive(Default)]
struct Bin {
    data: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Bin {
    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while !self.data.len().is_multiple_of(4) {
            self.data.push(0);
        }
        let mut v = json!({"buffer": 0, "byteOffset": self.data.len(), "byteLength": bytes.len()});
        if let Some(t) = target {
            v["target"] = json!(t);
        }
        self.data.extend_from_slice(bytes);
        self.views.push(v);
        self.views.len() - 1
    }

    /// A normalized integer attribute: `vals` holds `dims` components per
    /// vertex, each `size` bytes (1: signed byte, 2: signed or unsigned
    /// short), and each vertex is padded to a multiple of 4 bytes.
    fn ints(&mut self, vals: &[i32], dims: usize, size: usize, signed: bool, bounds: bool) -> usize {
        let count = vals.len() / dims;
        let stride = (dims * size).next_multiple_of(4);
        let mut bytes = Vec::with_capacity(count * stride);
        for c in vals.chunks_exact(dims) {
            for &v in c {
                bytes.extend_from_slice(&v.to_le_bytes()[..size]);
            }
            bytes.resize(bytes.len() + stride - dims * size, 0);
        }
        let view = self.view(&bytes, Some(34962));
        self.views[view]["byteStride"] = json!(stride);
        let ty = match (size, signed) {
            (1, true) => 5120,
            (1, false) => 5121,
            (2, true) => 5122,
            _ => 5123,
        };
        let kind = ["SCALAR", "VEC2", "VEC3", "VEC4"][dims - 1];
        let mut a = json!({"bufferView": view, "componentType": ty, "normalized": true, "count": count, "type": kind});
        if bounds {
            let (mut lo, mut hi) = (vec![i32::MAX; dims], vec![i32::MIN; dims]);
            for c in vals.chunks_exact(dims) {
                for d in 0..dims {
                    lo[d] = lo[d].min(c[d]);
                    hi[d] = hi[d].max(c[d]);
                }
            }
            a["min"] = json!(lo);
            a["max"] = json!(hi);
        }
        self.accessors.push(a);
        self.accessors.len() - 1
    }

    fn indices(&mut self, idx: &[u32], vertices: usize) -> usize {
        let (bytes, ty): (Vec<u8>, u32) = if vertices <= u16::MAX as usize {
            (idx.iter().flat_map(|&i| (i as u16).to_le_bytes()).collect(), 5123)
        } else {
            (idx.iter().flat_map(|i| i.to_le_bytes()).collect(), 5125)
        };
        let view = self.view(&bytes, Some(34963));
        self.accessors.push(json!({"bufferView": view, "componentType": ty, "count": idx.len(), "type": "SCALAR"}));
        self.accessors.len() - 1
    }
}

pub fn write(model: &Model, name: &str) -> Glb {
    // Every part in glTF's frame, merged by material.
    let mut merged: Vec<Mesh> = vec![Mesh::default(); model.materials.len()];
    for (m, mesh) in &model.parts {
        merged[*m].append(&mesh.clone().map(|v| gl(to_world(v))));
    }
    let desk_y = model.desk_z * 0.001;
    let (mut lo, mut hi) = (v3(f64::MAX, f64::MAX, f64::MAX), v3(f64::MIN, f64::MIN, f64::MIN));
    for p in merged.iter().flat_map(|m| &m.pos) {
        lo = v3(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
        hi = v3(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
    }
    let shift = v3((lo.x + hi.x) / 2.0, desk_y, (lo.z + hi.z) / 2.0);
    let (mut radius, mut height) = (0f64, 0f64);
    for mesh in &mut merged {
        for p in &mut mesh.pos {
            *p = *p - shift;
            radius = radius.max(p.x.hypot(p.z));
            height = height.max(p.y);
        }
    }
    let (lo, hi) = (lo - shift, hi - shift);
    let bounds = [lo.x, lo.y, lo.z, hi.x, hi.y, hi.z];

    // Positions in [-1, 1] of the model's largest half-extent, around the
    // box's middle; the node scales them back.
    let (mut lo, mut hi) = (v3(f64::MAX, f64::MAX, f64::MAX), v3(f64::MIN, f64::MIN, f64::MIN));
    for p in merged.iter().flat_map(|m| &m.pos) {
        lo = v3(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
        hi = v3(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
    }
    let middle = (lo + hi) * 0.5;
    let extent = ((hi.x - lo.x).max(hi.y - lo.y).max(hi.z - lo.z) / 2.0).max(1e-9);
    let q16 = |v: f64| (v.clamp(-1.0, 1.0) * 32767.0).round() as i32;
    let q8 = |v: f64| (v.clamp(-1.0, 1.0) * 127.0).round() as i32;

    let mut bin = Bin::default();
    let mut primitives = Vec::new();
    for (i, mesh) in merged.iter().enumerate() {
        if mesh.idx.is_empty() {
            continue;
        }
        let mat = &model.materials[i];
        let n = mesh.pos.len();
        let pos: Vec<i32> = mesh.pos.iter().flat_map(|&p| {
            let q = (p - middle) * (1.0 / extent);
            [q16(q.x), q16(q.y), q16(q.z)]
        }).collect();
        let nrm: Vec<i32> = mesh.nrm.iter().flat_map(|n| [q8(n.x), q8(n.y), q8(n.z)]).collect();
        let mut attributes = json!({
            "POSITION": bin.ints(&pos, 3, 2, true, true),
            "NORMAL": bin.ints(&nrm, 3, 1, true, false),
        });
        if mat.base_texture.is_some() || mat.emissive_texture.is_some() {
            let uv: Vec<i32> = mesh.uv.iter().flat_map(|&(u, v)| [(u.clamp(0.0, 1.0) * 65535.0).round() as i32, (v.clamp(0.0, 1.0) * 65535.0).round() as i32]).collect();
            attributes["TEXCOORD_0"] = json!(bin.ints(&uv, 2, 2, false, false));
        }
        let indices = bin.indices(&mesh.idx, n);
        primitives.push(json!({"attributes": attributes, "indices": indices, "material": i}));
    }

    let mut images = Vec::new();
    for png in &model.images {
        let view = bin.view(png, None);
        images.push(json!({"bufferView": view, "mimeType": "image/png"}));
    }
    let textures: Vec<Value> = (0..images.len()).map(|i| json!({"source": i, "sampler": 0})).collect();

    let mut used = Vec::new();
    let materials: Vec<Value> = model
        .materials
        .iter()
        .map(|m| {
            let mut pbr = json!({
                "baseColorFactor": [m.color[0], m.color[1], m.color[2], m.alpha],
                "metallicFactor": m.metal,
                "roughnessFactor": m.rough,
            });
            if let Some(t) = m.base_texture {
                pbr["baseColorTexture"] = json!({"index": t});
            }
            let mut v = json!({"name": m.name, "pbrMetallicRoughness": pbr});
            let mut ext = serde_json::Map::new();
            if m.emissive != [0.0; 3] {
                v["emissiveFactor"] = json!(m.emissive);
                if m.emissive_strength != 1.0 {
                    ext.insert("KHR_materials_emissive_strength".into(), json!({"emissiveStrength": m.emissive_strength}));
                }
            }
            if let Some(t) = m.emissive_texture {
                v["emissiveTexture"] = json!({"index": t});
            }
            if m.coat > 0.0 {
                ext.insert("KHR_materials_clearcoat".into(), json!({"clearcoatFactor": m.coat, "clearcoatRoughnessFactor": 0.05}));
            }
            if m.alpha < 1.0 || m.name == "legends" {
                v["alphaMode"] = json!("BLEND");
            }
            if !m.casts_shadow {
                v["extras"] = json!({"castShadow": false});
            }
            for k in ext.keys() {
                if !used.contains(k) {
                    used.push(k.clone());
                }
            }
            if !ext.is_empty() {
                v["extensions"] = Value::Object(ext);
            }
            v
        })
        .collect();

    while !bin.data.len().is_multiple_of(4) {
        bin.data.push(0);
    }
    let mut doc = json!({
        "asset": {"version": "2.0", "generator": "phone-models (tools/phone-models in the Open Desk Phone repository)"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"name": name, "mesh": 0, "translation": [middle.x, middle.y, middle.z], "scale": [extent, extent, extent]}],
        "meshes": [{"name": name, "primitives": primitives}],
        "materials": materials,
        "accessors": bin.accessors,
        "bufferViews": bin.views,
        "buffers": [{"byteLength": bin.data.len()}],
    });
    if !images.is_empty() {
        doc["images"] = json!(images);
        doc["textures"] = json!(textures);
        doc["samplers"] = json!([{"magFilter": 9729, "minFilter": 9987, "wrapS": 33071, "wrapT": 33071}]);
    }
    used.push("KHR_mesh_quantization".into());
    doc["extensionsUsed"] = json!(used);
    doc["extensionsRequired"] = json!(["KHR_mesh_quantization"]);
    let mut text = serde_json::to_vec(&doc).unwrap();
    while !text.len().is_multiple_of(4) {
        text.push(b' ');
    }
    let total = 12 + 8 + text.len() + 8 + bin.data.len();
    let mut out = Vec::with_capacity(total);
    out.extend(b"glTF");
    out.extend(2u32.to_le_bytes());
    out.extend((total as u32).to_le_bytes());
    out.extend((text.len() as u32).to_le_bytes());
    out.extend(b"JSON");
    out.extend(&text);
    out.extend((bin.data.len() as u32).to_le_bytes());
    out.extend(b"BIN\0");
    out.extend(&bin.data);
    Glb { bytes: out, radius, height, bounds }
}
