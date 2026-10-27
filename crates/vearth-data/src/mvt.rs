//! Minimal Mapbox Vector Tile (MVT) decoder.
//!
//! MVT is a small, frozen protobuf schema (vector_tile.proto, spec 2.1); a
//! hand-rolled wire reader keeps the dependency surface at zero and decodes
//! only what the voxelizer consumes: layer names/extents, feature geometry
//! (as flat ring/line lists in tile coordinates) and feature properties.

use crate::DataError;

#[path = "mvt_polygon.rs"]
pub(crate) mod polygon;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    F64(f64),
    I64(i64),
    Bool(bool),
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::F64(v) => Some(*v),
            Value::I64(v) => Some(*v as f64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeomType {
    Point,
    Line,
    Polygon,
}

pub struct Feature {
    /// Source feature identity from vector_tile.proto field 1. Protomaps
    /// carries the OSM-derived id through its building pyramid; retaining it
    /// lets render-side detail change without re-rolling a building's
    /// procedural state at every tile zoom.
    pub id: Option<u64>,
    pub geom_type: GeomType,
    /// Decoded geometry in tile coordinates (0..extent, y down): one entry
    /// per ring (polygons; closing vertex not repeated) or per linestring
    /// (lines) or per point-run (points).
    pub paths: Vec<Vec<[i32; 2]>>,
    /// Alternating key/value indices into the layer's `keys`/`values`.
    tags: Vec<u32>,
    // Initialized only after SemanticTile takes immutable ownership of the geometry.
    pub(crate) component_groups: std::sync::OnceLock<polygon::ComponentGroups>,
}

impl Feature {
    pub fn get<'l>(&self, layer: &'l Layer, key: &str) -> Option<&'l Value> {
        let ki = layer.keys.iter().position(|k| k == key)? as u32;
        self.tags
            .chunks_exact(2)
            .find(|kv| kv[0] == ki)
            .and_then(|kv| layer.values.get(kv[1] as usize))
    }

    pub(crate) fn tag_pairs(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.tags.chunks_exact(2).map(|pair| (pair[0], pair[1]))
    }

    pub(crate) fn estimated_heap_bytes(&self) -> usize {
        self.paths.capacity() * std::mem::size_of::<Vec<[i32; 2]>>()
            + self
                .paths
                .iter()
                .map(|path| path.capacity() * std::mem::size_of::<[i32; 2]>())
                .sum::<usize>()
            + self.tags.capacity() * std::mem::size_of::<u32>()
    }
}

pub struct Layer {
    pub name: String,
    pub extent: u32,
    pub keys: Vec<String>,
    pub values: Vec<Value>,
    pub features: Vec<Feature>,
}

pub struct VectorTile {
    pub layers: Vec<Layer>,
}

impl VectorTile {
    pub fn layer(&self, name: &str) -> Option<&Layer> {
        self.layers.iter().find(|l| l.name == name)
    }

    pub(crate) fn estimated_heap_bytes(&self) -> usize {
        self.layers.capacity() * std::mem::size_of::<Layer>()
            + self
                .layers
                .iter()
                .map(|layer| {
                    layer.name.capacity()
                        + layer.keys.capacity() * std::mem::size_of::<String>()
                        + layer.keys.iter().map(String::capacity).sum::<usize>()
                        + layer.values.capacity() * std::mem::size_of::<Value>()
                        + layer
                            .values
                            .iter()
                            .map(|value| match value {
                                Value::Str(value) => value.capacity(),
                                _ => 0,
                            })
                            .sum::<usize>()
                        + layer.features.capacity() * std::mem::size_of::<Feature>()
                        + layer
                            .features
                            .iter()
                            .map(Feature::estimated_heap_bytes)
                            .sum::<usize>()
                })
                .sum::<usize>()
    }
}

// Protobuf wire reader.

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

enum Field<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    Fixed32([u8; 4]),
    Fixed64([u8; 8]),
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    fn done(&self) -> bool {
        self.pos >= self.buf.len()
    }

    fn varint(&mut self) -> Result<u64, DataError> {
        let mut v = 0u64;
        let mut shift = 0u32;
        loop {
            let b = *self
                .buf
                .get(self.pos)
                .ok_or_else(|| DataError::Decode("mvt: truncated varint".into()))?;
            self.pos += 1;
            if shift >= 64 {
                return Err(DataError::Decode("mvt: varint overflow".into()));
            }
            v |= ((b & 0x7f) as u64) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
            shift += 7;
        }
    }

    fn field(&mut self) -> Result<(u32, Field<'a>), DataError> {
        let key = self.varint()?;
        let (num, wire) = ((key >> 3) as u32, key & 7);
        let bad = |m: &str| DataError::Decode(format!("mvt: {m}"));
        match wire {
            0 => Ok((num, Field::Varint(self.varint()?))),
            1 => {
                let end = self.pos + 8;
                let s = self
                    .buf
                    .get(self.pos..end)
                    .ok_or_else(|| bad("truncated f64"))?;
                self.pos = end;
                Ok((num, Field::Fixed64(s.try_into().unwrap())))
            }
            2 => {
                let len = self.varint()? as usize;
                let end = self.pos + len;
                let s = self
                    .buf
                    .get(self.pos..end)
                    .ok_or_else(|| bad("truncated bytes"))?;
                self.pos = end;
                Ok((num, Field::Bytes(s)))
            }
            5 => {
                let end = self.pos + 4;
                let s = self
                    .buf
                    .get(self.pos..end)
                    .ok_or_else(|| bad("truncated f32"))?;
                self.pos = end;
                Ok((num, Field::Fixed32(s.try_into().unwrap())))
            }
            w => Err(bad(&format!("wire type {w}"))),
        }
    }
}

fn zigzag(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

fn decode_value(buf: &[u8]) -> Result<Value, DataError> {
    let mut r = Reader::new(buf);
    let mut out = Value::Bool(false);
    while !r.done() {
        let (num, f) = r.field()?;
        out = match (num, f) {
            (1, Field::Bytes(b)) => Value::Str(
                std::str::from_utf8(b)
                    .map_err(|_| DataError::Decode("mvt: bad utf8".into()))?
                    .to_string(),
            ),
            (2, Field::Fixed32(b)) => Value::F64(f32::from_le_bytes(b) as f64),
            (3, Field::Fixed64(b)) => Value::F64(f64::from_le_bytes(b)),
            (4, Field::Varint(v)) => Value::I64(v as i64),
            (5, Field::Varint(v)) => Value::I64(v as i64),
            (6, Field::Varint(v)) => Value::I64(zigzag(v)),
            (7, Field::Varint(v)) => Value::Bool(v != 0),
            _ => continue,
        };
    }
    Ok(out)
}

/// Decode the geometry command stream (MoveTo=1, LineTo=2, ClosePath=7)
/// into flat paths. Cursor deltas are zigzag-encoded and cumulative across
/// the whole stream.
fn decode_geometry(geom: &[u64], geom_type: GeomType) -> Result<Vec<Vec<[i32; 2]>>, DataError> {
    let mut paths = Vec::new();
    let mut cur: Vec<[i32; 2]> = Vec::new();
    let (mut x, mut y) = (0i64, 0i64);
    let mut i = 0usize;
    while i < geom.len() {
        let cmd = geom[i];
        i += 1;
        let (id, count) = (cmd & 0x7, (cmd >> 3) as usize);
        match id {
            1 | 2 => {
                if geom.len() < i + count * 2 {
                    return Err(DataError::Decode("mvt: truncated geometry".into()));
                }
                if id == 1 && !cur.is_empty() {
                    paths.push(std::mem::take(&mut cur));
                }
                for _ in 0..count {
                    x += zigzag(geom[i]);
                    y += zigzag(geom[i + 1]);
                    i += 2;
                    cur.push([x as i32, y as i32]);
                }
            }
            7 => {
                // ClosePath: ring complete (closing vertex implicit).
                if !cur.is_empty() {
                    paths.push(std::mem::take(&mut cur));
                }
            }
            other => return Err(DataError::Decode(format!("mvt: geometry cmd {other}"))),
        }
    }
    if !cur.is_empty() {
        paths.push(cur);
    }
    // Points arrive as one MoveTo run; split multipoints into single points
    // so downstream code can treat every path uniformly.
    if geom_type == GeomType::Point {
        paths = paths
            .into_iter()
            .flat_map(|p| p.into_iter().map(|pt| vec![pt]).collect::<Vec<_>>())
            .collect();
    }
    Ok(paths)
}

fn decode_feature(buf: &[u8]) -> Result<Option<Feature>, DataError> {
    let mut r = Reader::new(buf);
    let mut id = None;
    let mut tags = Vec::new();
    let mut geom_ints = Vec::new();
    let mut geom_type = None;
    while !r.done() {
        let (num, f) = r.field()?;
        match (num, f) {
            (1, Field::Varint(v)) => id = Some(v),
            (2, Field::Bytes(b)) => {
                let mut pr = Reader::new(b);
                while !pr.done() {
                    tags.push(pr.varint()? as u32);
                }
            }
            (3, Field::Varint(v)) => {
                geom_type = match v {
                    1 => Some(GeomType::Point),
                    2 => Some(GeomType::Line),
                    3 => Some(GeomType::Polygon),
                    _ => None, // UNKNOWN: skip feature
                };
            }
            (4, Field::Bytes(b)) => {
                let mut gr = Reader::new(b);
                while !gr.done() {
                    geom_ints.push(gr.varint()?);
                }
            }
            _ => {}
        }
    }
    let Some(geom_type) = geom_type else {
        return Ok(None);
    };
    Ok(Some(Feature {
        id,
        geom_type,
        paths: decode_geometry(&geom_ints, geom_type)?,
        tags,
        component_groups: std::sync::OnceLock::new(),
    }))
}

fn decode_layer(buf: &[u8]) -> Result<Layer, DataError> {
    let mut layer = Layer {
        name: String::new(),
        extent: 4096,
        keys: Vec::new(),
        values: Vec::new(),
        features: Vec::new(),
    };
    let mut r = Reader::new(buf);
    while !r.done() {
        let (num, f) = r.field()?;
        match (num, f) {
            (1, Field::Bytes(b)) => {
                layer.name = std::str::from_utf8(b)
                    .map_err(|_| DataError::Decode("mvt: bad layer name".into()))?
                    .to_string();
            }
            (2, Field::Bytes(b)) => {
                if let Some(feat) = decode_feature(b)? {
                    layer.features.push(feat);
                }
            }
            (3, Field::Bytes(b)) => layer.keys.push(
                std::str::from_utf8(b)
                    .map_err(|_| DataError::Decode("mvt: bad key".into()))?
                    .to_string(),
            ),
            (4, Field::Bytes(b)) => layer.values.push(decode_value(b)?),
            (5, Field::Varint(v)) => layer.extent = v as u32,
            _ => {}
        }
    }
    Ok(layer)
}

pub fn decode_tile(buf: &[u8]) -> Result<VectorTile, DataError> {
    let mut layers = Vec::new();
    let mut r = Reader::new(buf);
    while !r.done() {
        let (num, f) = r.field()?;
        if let (3, Field::Bytes(b)) = (num, f) {
            layers.push(decode_layer(b)?);
        }
    }
    Ok(VectorTile { layers })
}

#[cfg(test)]
pub(crate) mod encode {
    //! Tiny MVT encoder for tests (and only tests): builds valid tiles from
    //! path lists so decoder tests don't need binary fixtures.

    use super::GeomType;

    pub fn varint(out: &mut Vec<u8>, mut v: u64) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                break;
            }
            out.push(b | 0x80);
        }
    }

    pub fn zigzag(v: i64) -> u64 {
        ((v << 1) ^ (v >> 63)) as u64
    }

    fn bytes_field(out: &mut Vec<u8>, num: u32, payload: &[u8]) {
        varint(out, ((num as u64) << 3) | 2);
        varint(out, payload.len() as u64);
        out.extend_from_slice(payload);
    }

    pub struct FeatureSpec {
        pub id: Option<u64>,
        pub geom_type: GeomType,
        pub paths: Vec<Vec<[i32; 2]>>,
        /// (key index, value index) pairs.
        pub tags: Vec<(u32, u32)>,
    }

    pub struct LayerSpec<'a> {
        pub name: &'a str,
        pub extent: u32,
        pub keys: &'a [&'a str],
        pub values: &'a [super::Value],
        pub features: &'a [FeatureSpec],
    }

    fn layer(spec: &LayerSpec<'_>) -> Vec<u8> {
        let mut layer = Vec::new();
        bytes_field(&mut layer, 1, spec.name.as_bytes());
        for f in spec.features {
            let mut feat = Vec::new();
            if let Some(id) = f.id {
                varint(&mut feat, 1 << 3);
                varint(&mut feat, id);
            }
            // tags
            let mut tags = Vec::new();
            for (k, v) in &f.tags {
                varint(&mut tags, *k as u64);
                varint(&mut tags, *v as u64);
            }
            bytes_field(&mut feat, 2, &tags);
            // type
            varint(&mut feat, 3 << 3);
            varint(
                &mut feat,
                match f.geom_type {
                    GeomType::Point => 1,
                    GeomType::Line => 2,
                    GeomType::Polygon => 3,
                },
            );
            // geometry
            let mut geom = Vec::new();
            let (mut cx, mut cy) = (0i64, 0i64);
            for path in &f.paths {
                varint(&mut geom, (1 << 3) | 1); // MoveTo x1
                varint(&mut geom, zigzag(path[0][0] as i64 - cx));
                varint(&mut geom, zigzag(path[0][1] as i64 - cy));
                cx = path[0][0] as i64;
                cy = path[0][1] as i64;
                if path.len() > 1 {
                    varint(&mut geom, ((path.len() as u64 - 1) << 3) | 2); // LineTo xN
                    for p in &path[1..] {
                        varint(&mut geom, zigzag(p[0] as i64 - cx));
                        varint(&mut geom, zigzag(p[1] as i64 - cy));
                        cx = p[0] as i64;
                        cy = p[1] as i64;
                    }
                }
                if f.geom_type == GeomType::Polygon {
                    varint(&mut geom, (1 << 3) | 7); // ClosePath x1
                }
            }
            bytes_field(&mut feat, 4, &geom);
            bytes_field(&mut layer, 2, &feat);
        }
        for k in spec.keys {
            bytes_field(&mut layer, 3, k.as_bytes());
        }
        for v in spec.values {
            let mut val = Vec::new();
            match v {
                super::Value::Str(s) => bytes_field(&mut val, 1, s.as_bytes()),
                super::Value::F64(d) => {
                    varint(&mut val, (3 << 3) | 1);
                    val.extend_from_slice(&d.to_le_bytes());
                }
                super::Value::I64(i) => {
                    varint(&mut val, 6 << 3);
                    varint(&mut val, zigzag(*i));
                }
                super::Value::Bool(b) => {
                    varint(&mut val, 7 << 3);
                    varint(&mut val, *b as u64);
                }
            }
            bytes_field(&mut layer, 4, &val);
        }
        // extent
        let mut ext = Vec::new();
        varint(&mut ext, 5 << 3);
        varint(&mut ext, spec.extent as u64);
        layer.extend_from_slice(&ext);

        layer
    }

    pub fn tile_layers(layers: &[LayerSpec<'_>]) -> Vec<u8> {
        let mut tile = Vec::new();
        for spec in layers {
            bytes_field(&mut tile, 3, &layer(spec));
        }
        tile
    }

    pub fn tile(
        layer_name: &str,
        extent: u32,
        keys: &[&str],
        values: &[super::Value],
        feats: &[FeatureSpec],
    ) -> Vec<u8> {
        tile_layers(&[LayerSpec {
            name: layer_name,
            extent,
            keys,
            values,
            features: feats,
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::encode::FeatureSpec;
    use super::*;

    #[test]
    fn round_trips_polygon_with_props() {
        let buf = encode::tile(
            "buildings",
            4096,
            &["height", "kind"],
            &[Value::F64(42.5), Value::Str("building".into())],
            &[FeatureSpec {
                id: Some(0x1234_5678_9abc_def0),
                geom_type: GeomType::Polygon,
                paths: vec![vec![[10, 10], [200, 10], [200, 150], [10, 150]]],
                tags: vec![(0, 0), (1, 1)],
            }],
        );
        let tile = decode_tile(&buf).unwrap();
        let layer = tile.layer("buildings").unwrap();
        assert_eq!(layer.extent, 4096);
        assert_eq!(layer.features.len(), 1);
        let f = &layer.features[0];
        assert_eq!(f.id, Some(0x1234_5678_9abc_def0));
        assert_eq!(f.geom_type, GeomType::Polygon);
        assert_eq!(
            f.paths,
            vec![vec![[10, 10], [200, 10], [200, 150], [10, 150]]]
        );
        assert_eq!(f.get(layer, "height").unwrap().as_f64(), Some(42.5));
        assert_eq!(f.get(layer, "kind").unwrap().as_str(), Some("building"));
        assert!(f.get(layer, "nope").is_none());
    }

    #[test]
    fn round_trips_multiline_and_negative_deltas() {
        let paths = vec![
            vec![[500, 500], [100, 800], [900, 900]],
            vec![[0, 0], [50, 1]],
        ];
        let buf = encode::tile(
            "roads",
            8192,
            &[],
            &[],
            &[FeatureSpec {
                id: None,
                geom_type: GeomType::Line,
                paths: paths.clone(),
                tags: vec![],
            }],
        );
        let tile = decode_tile(&buf).unwrap();
        let layer = tile.layer("roads").unwrap();
        assert_eq!(layer.extent, 8192);
        assert_eq!(layer.features[0].paths, paths);
    }

    #[test]
    fn splits_multipoints() {
        let buf = encode::tile(
            "pois",
            4096,
            &[],
            &[],
            &[FeatureSpec {
                id: None,
                geom_type: GeomType::Point,
                paths: vec![vec![[5, 6], [7, 8]]],
                tags: vec![],
            }],
        );
        let tile = decode_tile(&buf).unwrap();
        assert_eq!(
            tile.layer("pois").unwrap().features[0].paths,
            vec![vec![[5, 6]], vec![[7, 8]]]
        );
    }

    #[test]
    fn rejects_garbage() {
        assert!(
            decode_tile(&[
                0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01
            ])
            .is_err()
        );
    }
}
