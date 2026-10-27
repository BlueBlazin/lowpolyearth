//! Compact offline first-level administrative-region lookup.
//!
//! Owned location labels retain country-name suppression. Borrowed geographic
//! context also supports regional generation without allocating per building.

use flate2::read::ZlibDecoder;
use std::io::Read;
use std::sync::OnceLock;

pub const ADMIN_REGIONS_VERSION: u32 = 1;
pub const ADMIN_REGIONS_PROVENANCE: &str =
    "Natural Earth 5.1.1 Admin-1 states and provinces (public domain)";

fn compressed() -> &'static [u8] {
    vearth_platform::asset_bytes!("../assets/admin-regions-v1.bin.z")
}
const MAGIC: &[u8; 4] = b"VAR1";
const WORLD_DEGREES: i64 = 360;
const INDEX_DEGREES: i32 = 2;
const INDEX_COLUMNS: usize = 360 / INDEX_DEGREES as usize;
const INDEX_ROWS: usize = 180 / INDEX_DEGREES as usize;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminRegion {
    pub id: u64,
    pub name: String,
    pub(crate) country_name: String,
}

/// Source attribution for a containing administrative polygon, not a label or
/// a claim that its buildings share one architectural tradition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdminRegionContext {
    pub id: u64,
    pub name: &'static str,
    pub country_name: &'static str,
}

#[derive(Debug)]
struct RegionGeometry {
    region: AdminRegion,
    bbox: [i32; 4],
    polygons: Vec<Vec<CompiledRing>>,
}

#[derive(Debug)]
struct CompiledRing {
    points: Vec<[i64; 2]>,
    y_bounds: [i64; 2],
    rows: Vec<Vec<u32>>,
    #[cfg(test)]
    source: Vec<[i32; 2]>,
}

impl CompiledRing {
    fn new(source: Vec<[i32; 2]>, scale: i32) -> Self {
        let mut points = Vec::with_capacity(source.len());
        let mut previous_x = source.first().map_or(0, |p| i64::from(p[0]));
        let mut y_bounds = [i64::MAX, i64::MIN];
        for point in &source {
            previous_x = unwrap_longitude(point[0], previous_x, scale);
            let y = i64::from(point[1]);
            points.push([previous_x, y]);
            y_bounds[0] = y_bounds[0].min(y);
            y_bounds[1] = y_bounds[1].max(y);
        }
        let mut result = Self {
            points,
            y_bounds,
            rows: Vec::new(),
            #[cfg(test)]
            source,
        };
        if result.points.len() >= 32 && y_bounds[0] < y_bounds[1] {
            result.rows = vec![Vec::new(); 32];
            for end in 1..=result.points.len() {
                let [a, b] = result.edge(end);
                if a[1] == b[1] {
                    continue;
                }
                // Atlas queries are integer coordinates and ray crossings
                // include the lower endpoint but exclude the upper endpoint.
                let first = result.row(a[1].min(b[1]));
                let last = result.row(a[1].max(b[1]) - 1);
                for row in &mut result.rows[first..=last] {
                    row.push(end as u32);
                }
            }
        }
        result
    }

    fn row(&self, y: i64) -> usize {
        (((y - self.y_bounds[0]) * 32 / (self.y_bounds[1] - self.y_bounds[0] + 1)).clamp(0, 31))
            as usize
    }

    fn edge(&self, end: usize) -> [[i64; 2]; 2] {
        [self.points[end - 1], self.points[end % self.points.len()]]
    }

    fn contains(&self, point: [i32; 2], scale: i32) -> bool {
        let point_x = i64::from(point[0]);
        let point_y = i64::from(point[1]);
        if self.points.len() < 3 || point_y < self.y_bounds[0] || point_y >= self.y_bounds[1] {
            return false;
        }
        // Unwrapping the source chain once differs from per-query unwrapping
        // only by this whole-world translation. Apply it before the original
        // floating-point intersection expression, preserving boundary ties.
        let first_x = self.points[0][0];
        let shift = unwrap_longitude(first_x as i32, point_x, scale) - first_x;
        let crosses = |end: usize| {
            let [a, b] = self.edge(end);
            let (ax, ay, bx, by) = (a[0] + shift, a[1], b[0] + shift, b[1]);
            if (ay > point_y) == (by > point_y) {
                return false;
            }
            let intersection_x =
                ax as f64 + (point_y - ay) as f64 * (bx - ax) as f64 / (by - ay) as f64;
            (point_x as f64) < intersection_x
        };
        if self.rows.is_empty() {
            (1..=self.points.len()).fold(false, |inside, end| inside ^ crosses(end))
        } else {
            self.rows[self.row(point_y)]
                .iter()
                .fold(false, |inside, &end| inside ^ crosses(end as usize))
        }
    }
}

#[derive(Debug)]
struct Atlas {
    scale: i32,
    regions: Vec<RegionGeometry>,
    spatial_index: Vec<Vec<usize>>,
}

#[derive(Clone, Copy)]
pub struct AdminRegionAtlas;

impl AdminRegionAtlas {
    pub const fn bundled() -> Self {
        Self
    }

    pub fn region_at(self, lat_rad: f64, lon_rad: f64) -> Option<AdminRegion> {
        atlas()
            .region_at(lat_rad, lon_rad)
            .map(|geometry| geometry.region.clone())
    }

    /// Allocation-free attribution using the bundled polygon boundaries.
    /// Ocean, uncovered coastline, and invalid coordinates return `None`.
    pub fn context_at(self, lat_rad: f64, lon_rad: f64) -> Option<AdminRegionContext> {
        atlas().region_at(lat_rad, lon_rad).map(region_context)
    }

    /// Every source record, including territories and disputed subdivisions.
    /// Stable source identifiers are the authority; names are source spellings.
    pub fn contexts(self) -> impl ExactSizeIterator<Item = AdminRegionContext> {
        atlas().regions.iter().map(region_context)
    }
}

fn region_context(geometry: &'static RegionGeometry) -> AdminRegionContext {
    AdminRegionContext {
        id: geometry.region.id,
        name: &geometry.region.name,
        country_name: &geometry.region.country_name,
    }
}

impl Atlas {
    fn region_at(&self, lat_rad: f64, lon_rad: f64) -> Option<&RegionGeometry> {
        if !lat_rad.is_finite()
            || !lon_rad.is_finite()
            || lat_rad.abs() > std::f64::consts::FRAC_PI_2
        {
            return None;
        }
        let latitude = lat_rad.to_degrees();
        let longitude = (lon_rad.rem_euclid(std::f64::consts::TAU).to_degrees() + 180.0)
            .rem_euclid(360.0)
            - 180.0;
        let point = [
            (longitude * f64::from(self.scale)).round() as i32,
            (latitude * f64::from(self.scale)).round() as i32,
        ];
        let (column, row) = index_coordinate(point, self.scale);
        self.spatial_index[row * INDEX_COLUMNS + column]
            .iter()
            .map(|&index| &self.regions[index])
            .filter(|candidate| bbox_contains(candidate.bbox, point, self.scale))
            .filter(|candidate| {
                candidate.polygons.iter().any(|polygon| {
                    polygon.iter().fold(false, |inside, ring| {
                        inside ^ ring.contains(point, self.scale)
                    })
                })
            })
            // Natural Earth contains a few overlapping/disputed shapes.
            // Prefer the tighter containing extent, then make the result
            // deterministic by stable source id.
            .min_by_key(|candidate| {
                let width = bbox_width(candidate.bbox, self.scale);
                let height = i64::from(candidate.bbox[3]) - i64::from(candidate.bbox[1]);
                (width.saturating_mul(height), candidate.region.id)
            })
    }
}

fn index_coordinate(point: [i32; 2], scale: i32) -> (usize, usize) {
    let column = ((i64::from(point[0]) + 180 * i64::from(scale))
        / (i64::from(INDEX_DEGREES) * i64::from(scale)))
    .clamp(0, INDEX_COLUMNS as i64 - 1) as usize;
    let row = ((i64::from(point[1]) + 90 * i64::from(scale))
        / (i64::from(INDEX_DEGREES) * i64::from(scale)))
    .clamp(0, INDEX_ROWS as i64 - 1) as usize;
    (column, row)
}

fn spatial_index(regions: &[RegionGeometry], scale: i32) -> Vec<Vec<usize>> {
    let mut index = vec![Vec::new(); INDEX_COLUMNS * INDEX_ROWS];
    for (region_index, geometry) in regions.iter().enumerate() {
        let (mut left, bottom) = index_coordinate([geometry.bbox[0], geometry.bbox[1]], scale);
        let (mut right, top) = index_coordinate([geometry.bbox[2], geometry.bbox[3]], scale);
        if i64::from(geometry.bbox[2]) - i64::from(geometry.bbox[0]) > 180 * i64::from(scale) {
            // The existing periodic polygon test unwraps dateline crossings.
            // Keep every longitude candidate for those enclosing extents.
            left = 0;
            right = INDEX_COLUMNS - 1;
        }
        for row in bottom..=top {
            for column in left..=right {
                index[row * INDEX_COLUMNS + column].push(region_index);
            }
        }
    }
    index
}

fn bbox_width(bbox: [i32; 4], scale: i32) -> i64 {
    let width = i64::from(bbox[2]) - i64::from(bbox[0]);
    let world = WORLD_DEGREES * i64::from(scale);
    width.min(world - width)
}

fn bbox_contains(bbox: [i32; 4], point: [i32; 2], scale: i32) -> bool {
    if point[1] < bbox[1] || point[1] > bbox[3] {
        return false;
    }
    let world = WORLD_DEGREES * i64::from(scale);
    let width = i64::from(bbox[2]) - i64::from(bbox[0]);
    width > world / 2 || (point[0] >= bbox[0] && point[0] <= bbox[2])
}

fn unwrap_longitude(longitude: i32, around: i64, scale: i32) -> i64 {
    let world = WORLD_DEGREES * i64::from(scale);
    let half = world / 2;
    let delta = (i64::from(longitude) - around + half).rem_euclid(world) - half;
    around + delta
}

#[cfg(test)]
fn point_in_polygon(point: [i32; 2], rings: &[CompiledRing], scale: i32) -> bool {
    rings
        .iter()
        .filter(|ring| ring.source.len() >= 3)
        .fold(false, |inside, ring| {
            inside ^ point_in_ring(point, &ring.source, scale)
        })
}

#[cfg(test)]
fn point_in_ring(point: [i32; 2], ring: &[[i32; 2]], scale: i32) -> bool {
    let point_x = i64::from(point[0]);
    let point_y = i64::from(point[1]);
    let mut inside = false;
    let first_x = unwrap_longitude(ring[0][0], point_x, scale);
    let mut ax = first_x;
    let mut ay = i64::from(ring[0][1]);
    for index in 1..=ring.len() {
        let (bx, by) = if index == ring.len() {
            (first_x, i64::from(ring[0][1]))
        } else {
            (
                unwrap_longitude(ring[index][0], ax, scale),
                i64::from(ring[index][1]),
            )
        };
        if (ay > point_y) != (by > point_y) {
            let intersection_x =
                ax as f64 + (point_y - ay) as f64 * (bx - ax) as f64 / (by - ay) as f64;
            if (point_x as f64) < intersection_x {
                inside = !inside;
            }
        }
        ax = bx;
        ay = by;
    }
    inside
}

fn atlas() -> &'static Atlas {
    static ATLAS: OnceLock<Atlas> = OnceLock::new();
    ATLAS.get_or_init(|| decode_atlas(compressed()).expect("bundled admin-region atlas is valid"))
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let end = self.position.saturating_add(N);
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| "truncated admin-region atlas".to_string())?;
        self.position = end;
        Ok(bytes.try_into().expect("slice length checked"))
    }

    fn u16(&mut self) -> Result<u16, String> {
        self.take().map(u16::from_le_bytes)
    }

    fn u32(&mut self) -> Result<u32, String> {
        self.take().map(u32::from_le_bytes)
    }

    fn u64(&mut self) -> Result<u64, String> {
        self.take().map(u64::from_le_bytes)
    }

    fn i32(&mut self) -> Result<i32, String> {
        self.take().map(i32::from_le_bytes)
    }

    fn string(&mut self) -> Result<String, String> {
        let length = usize::from(self.u16()?);
        let end = self.position.saturating_add(length);
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| "truncated admin-region name".to_string())?;
        self.position = end;
        std::str::from_utf8(bytes)
            .map(str::to_string)
            .map_err(|_| "invalid UTF-8 in admin-region atlas".to_string())
    }
}

fn decode_atlas(compressed: &[u8]) -> Result<Atlas, String> {
    let mut bytes = Vec::new();
    ZlibDecoder::new(compressed)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("admin-region zlib: {error}"))?;
    let mut reader = Reader::new(&bytes);
    if &reader.take::<4>()? != MAGIC {
        return Err("invalid admin-region atlas magic".into());
    }
    let version = reader.u32()?;
    if version != ADMIN_REGIONS_VERSION {
        return Err(format!("unsupported admin-region atlas version {version}"));
    }
    let scale = i32::try_from(reader.u32()?).map_err(|_| "invalid atlas scale")?;
    if scale <= 0 {
        return Err("invalid admin-region atlas scale".into());
    }
    let region_count = usize::try_from(reader.u32()?).map_err(|_| "invalid admin-region count")?;
    let mut regions = Vec::with_capacity(region_count);
    for _ in 0..region_count {
        let id = reader.u64()?;
        let bbox = [reader.i32()?, reader.i32()?, reader.i32()?, reader.i32()?];
        let name = reader.string()?;
        let country_name = reader.string()?;
        let polygon_count = usize::from(reader.u16()?);
        let mut polygons = Vec::with_capacity(polygon_count);
        for _ in 0..polygon_count {
            let ring_count = usize::from(reader.u16()?);
            let mut rings = Vec::with_capacity(ring_count);
            for _ in 0..ring_count {
                let point_count =
                    usize::try_from(reader.u32()?).map_err(|_| "invalid point count")?;
                let mut ring = Vec::with_capacity(point_count);
                let (mut x, mut y) = (0_i32, 0_i32);
                for _ in 0..point_count {
                    x = x
                        .checked_add(reader.i32()?)
                        .ok_or_else(|| "admin-region longitude overflow".to_string())?;
                    y = y
                        .checked_add(reader.i32()?)
                        .ok_or_else(|| "admin-region latitude overflow".to_string())?;
                    ring.push([x, y]);
                }
                rings.push(CompiledRing::new(ring, scale));
            }
            polygons.push(rings);
        }
        regions.push(RegionGeometry {
            region: AdminRegion {
                id,
                name,
                country_name,
            },
            bbox,
            polygons,
        });
    }
    if reader.position != bytes.len() {
        return Err("trailing bytes in admin-region atlas".into());
    }
    let spatial_index = spatial_index(&regions, scale);
    Ok(Atlas {
        scale,
        regions,
        spatial_index,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn region(lat: f64, lon: f64) -> String {
        AdminRegionAtlas::bundled()
            .region_at(lat.to_radians(), lon.to_radians())
            .unwrap_or_else(|| panic!("no region at {lat}, {lon}"))
            .name
    }

    #[test]
    fn bundled_regions_cover_requested_examples() {
        assert_eq!(region(31.7619, -106.4850), "Texas");
        assert_eq!(region(39.1463, 73.4603), "Gorno-Badakhshan");
        assert_eq!(region(-33.8688, 151.2093), "New South Wales");
    }

    #[test]
    fn periodic_ring_test_handles_the_antimeridian() {
        let scale = 100;
        let ring = vec![
            [179 * scale, -scale],
            [-179 * scale, -scale],
            [-179 * scale, scale],
            [179 * scale, scale],
        ];
        assert!(point_in_ring([17950, 0], &ring, scale));
        assert!(point_in_ring([-17950, 0], &ring, scale));
        assert!(!point_in_ring([0, 0], &ring, scale));
    }

    #[test]
    fn compiled_rings_preserve_periodic_boundary_and_scanline_membership() {
        let scale = 100;
        let mut rings = vec![
            vec![[17900, -100], [-17900, -100], [-17900, 100], [17900, 100]],
            vec![
                [-18000, 8900],
                [-9000, 8700],
                [0, 8900],
                [9000, 8700],
                [18000, 8900],
            ],
            vec![
                [-18000, -8900],
                [9000, -8700],
                [0, -8900],
                [-9000, -8700],
                [18000, -8900],
            ],
            vec![[-18000, -100], [0, -100], [18000, 100], [0, 100]],
            vec![[0, 0], [100, 100], [0, 100], [100, 0], [0, 0]],
        ];
        for center in [-17900, 0, 17900] {
            rings.push(
                (0..128)
                    .map(|i| {
                        let angle = i as f64 * std::f64::consts::TAU / 128.0;
                        let radius = if i % 3 == 0 { 330.0 } else { 220.0 };
                        let x = center + (angle.cos() * radius).round() as i32;
                        [
                            (x + 18000).rem_euclid(36000) - 18000,
                            (angle.sin() * radius).round() as i32,
                        ]
                    })
                    .collect(),
            );
        }
        for source in rings {
            let compiled = CompiledRing::new(source.clone(), scale);
            let check = |point| {
                assert_eq!(
                    compiled.contains(point, scale),
                    point_in_ring(point, &source, scale),
                    "point={point:?}, source={source:?}"
                )
            };
            for y in (-9000..=9000).step_by(173) {
                for x in (-54000..=54000).step_by(277) {
                    check([x, y]);
                }
            }
            for &point in &source {
                for dx in [-1, 0, 1] {
                    for dy in [-1, 0, 1] {
                        check([point[0] + dx, point[1] + dy]);
                    }
                }
            }
            for row in 0..=32 {
                let y = compiled.y_bounds[0]
                    + (compiled.y_bounds[1] - compiled.y_bounds[0] + 1) * row / 32;
                for dy in [-1, 0, 1] {
                    for x in (-18000..=18000).step_by(131) {
                        check([x, (y + dy) as i32]);
                    }
                }
            }
        }
        for source in [vec![], vec![[0, 0]], vec![[0, 0], [1, 1]], vec![[0, 0]; 40]] {
            let compiled = CompiledRing::new(source, scale);
            assert!(!compiled.contains([0, 0], scale));
        }
    }

    #[test]
    fn compiled_real_atlas_rings_match_source_edges_and_interiors() {
        let atlas = atlas();
        let mut total = 0;
        let mut indexed = 0;
        for geometry in &atlas.regions {
            for polygon in &geometry.polygons {
                for ring in polygon {
                    if ring.source.len() < 3 {
                        continue;
                    }
                    total += 1;
                    indexed += usize::from(!ring.rows.is_empty());
                    let check = |point| {
                        assert_eq!(
                            ring.contains(point, atlas.scale),
                            point_in_ring(point, &ring.source, atlas.scale),
                            "{} {:?}",
                            geometry.region.name,
                            point
                        );
                    };
                    for index in (0..ring.source.len()).step_by((ring.source.len() / 24).max(1)) {
                        let a = ring.source[index];
                        let b = ring.source[(index + 1) % ring.source.len()];
                        for point in [
                            a,
                            [
                                ((i64::from(a[0]) + i64::from(b[0])) / 2) as i32,
                                ((i64::from(a[1]) + i64::from(b[1])) / 2) as i32,
                            ],
                        ] {
                            for offset in [[0, 0], [1, 0], [-1, 0], [0, 1], [0, -1]] {
                                check([point[0] + offset[0], point[1] + offset[1]]);
                            }
                        }
                    }
                    let lo = [
                        ring.source.iter().map(|p| p[0]).min().unwrap(),
                        ring.source.iter().map(|p| p[1]).min().unwrap(),
                    ];
                    let hi = [
                        ring.source.iter().map(|p| p[0]).max().unwrap(),
                        ring.source.iter().map(|p| p[1]).max().unwrap(),
                    ];
                    for y in 1..5 {
                        for x in 1..5 {
                            check([
                                lo[0] + ((i64::from(hi[0]) - i64::from(lo[0])) * x / 5) as i32,
                                lo[1] + ((i64::from(hi[1]) - i64::from(lo[1])) * y / 5) as i32,
                            ]);
                        }
                    }
                }
            }
        }
        assert!(total > 4_589 && indexed > 1_000);
    }

    #[test]
    #[ignore = "local geographic-query benchmark"]
    fn compiled_geographic_query_benchmark() {
        let atlas = atlas();
        let geography = atlas
            .region_at(52.36_f64.to_radians(), 4.90_f64.to_radians())
            .unwrap();
        let points: Vec<_> = (0..50_000)
            .map(|i| {
                [
                    ((4.85 + (i % 251) as f64 / 251.0 * 0.1) * f64::from(atlas.scale)).round()
                        as i32,
                    ((52.30 + (i % 197) as f64 / 197.0 * 0.1) * f64::from(atlas.scale)).round()
                        as i32,
                ]
            })
            .collect();
        let original = |point| {
            geography
                .polygons
                .iter()
                .any(|polygon| point_in_polygon(point, polygon, atlas.scale))
        };
        let compiled = |point| {
            geography.polygons.iter().any(|polygon| {
                polygon.iter().fold(false, |inside, ring| {
                    inside ^ ring.contains(point, atlas.scale)
                })
            })
        };
        let start = vearth_platform::time::Instant::now();
        let expected: Vec<_> = points.iter().map(|&p| original(p)).collect();
        let original_time = start.elapsed();
        let start = vearth_platform::time::Instant::now();
        let actual: Vec<_> = points.iter().map(|&p| compiled(p)).collect();
        let compiled_time = start.elapsed();
        assert_eq!(actual, expected);
        eprintln!(
            "{}: {original_time:?} -> {compiled_time:?} ({:.2}x)",
            geography.region.name,
            original_time.as_secs_f64() / compiled_time.as_secs_f64()
        );
    }

    #[test]
    fn borrowed_context_catalog_retains_every_source_identity() {
        let contexts: Vec<_> = AdminRegionAtlas::bundled().contexts().collect();
        assert_eq!(contexts.len(), 4_589);
        assert_eq!(
            contexts
                .iter()
                .map(|context| context.country_name)
                .collect::<HashSet<_>>()
                .len(),
            253
        );
        assert_eq!(
            contexts
                .iter()
                .map(|context| context.id)
                .collect::<HashSet<_>>()
                .len(),
            contexts.len()
        );
        for context in contexts {
            assert!(!context.name.is_empty());
            assert!(!context.country_name.is_empty());
        }
        let atlas = AdminRegionAtlas::bundled();
        let borrowed = atlas
            .context_at(18.5207_f64.to_radians(), 73.8524_f64.to_radians())
            .unwrap();
        let owned = atlas
            .region_at(18.5207_f64.to_radians(), 73.8524_f64.to_radians())
            .unwrap();
        assert_eq!(borrowed.name, "Maharashtra");
        assert_eq!(borrowed.country_name, "India");
        assert_eq!(borrowed.id, owned.id);
        assert_eq!(borrowed.name, owned.name);
        assert_eq!(borrowed.country_name, owned.country_name);
        let repeated = atlas
            .context_at(18.5207_f64.to_radians(), 73.8524_f64.to_radians())
            .unwrap();
        assert_eq!(borrowed.name.as_ptr(), repeated.name.as_ptr());
    }

    #[test]
    fn borders_and_island_contexts_use_containing_polygons() {
        for (latitude, longitude, country) in [
            (31.7619_f64, -106.4850_f64, "United States of America"),
            (31.6904, -106.4245, "Mexico"),
            (48.5734, 7.7521, "France"),
            (48.5729, 7.8155, "Germany"),
            (1.3521, 103.8198, "Singapore"),
            (-18.1248, 178.4501, "Fiji"),
            (-13.8333, -171.7667, "Samoa"),
            (13.1132, -59.5988, "Barbados"),
            (43.7384, 7.4246, "Monaco"),
            (41.9029, 12.4534, "Vatican"),
            (-17.7333, 168.3167, "Vanuatu"),
        ] {
            let found = AdminRegionAtlas::bundled()
                .context_at(latitude.to_radians(), longitude.to_radians())
                .unwrap_or_else(|| panic!("no region at {latitude}, {longitude}"));
            assert_eq!(found.country_name, country, "{latitude}, {longitude}");
        }
    }

    #[test]
    fn index_preserves_exhaustive_polygon_results() {
        let atlas = atlas();
        for latitude in (-85..=85).step_by(5) {
            for longitude in (-180..180).step_by(5) {
                let point = [longitude * atlas.scale, latitude * atlas.scale];
                let expected = atlas
                    .regions
                    .iter()
                    .filter(|candidate| bbox_contains(candidate.bbox, point, atlas.scale))
                    .filter(|candidate| {
                        candidate
                            .polygons
                            .iter()
                            .any(|polygon| point_in_polygon(point, polygon, atlas.scale))
                    })
                    .min_by_key(|candidate| {
                        (
                            bbox_width(candidate.bbox, atlas.scale)
                                * (i64::from(candidate.bbox[3]) - i64::from(candidate.bbox[1])),
                            candidate.region.id,
                        )
                    })
                    .map(|candidate| candidate.region.id);
                let actual = atlas
                    .region_at(
                        f64::from(latitude).to_radians(),
                        f64::from(longitude).to_radians(),
                    )
                    .map(|candidate| candidate.region.id);
                assert_eq!(actual, expected, "{latitude}, {longitude}");
            }
        }
    }

    #[test]
    fn index_and_polygon_holes_handle_dateline_and_empty_space() {
        let scale = 100;
        let regions = vec![RegionGeometry {
            region: AdminRegion {
                id: 1,
                name: "Dateline".into(),
                country_name: "Synthetic".into(),
            },
            bbox: [-17900, -100, 17900, 100],
            polygons: vec![vec![
                CompiledRing::new(
                    vec![[17900, -100], [-17900, -100], [-17900, 100], [17900, 100]],
                    scale,
                ),
                CompiledRing::new(
                    vec![[17980, -20], [-17980, -20], [-17980, 20], [17980, 20]],
                    scale,
                ),
            ]],
        }];
        let atlas = Atlas {
            scale,
            spatial_index: spatial_index(&regions, scale),
            regions,
        };
        for longitude in [179.5_f64, -179.5, 179.5 + 360.0, -179.5 - 360.0] {
            assert!(atlas.region_at(0.0, longitude.to_radians()).is_some());
        }
        assert!(atlas.region_at(0.0, std::f64::consts::PI).is_none());
        assert!(atlas.region_at(0.0, 0.0).is_none());
        assert!(
            atlas
                .region_at(2.0_f64.to_radians(), 179.5_f64.to_radians())
                .is_none()
        );
    }

    #[test]
    fn invalid_coordinates_have_no_geographic_attribution() {
        let atlas = AdminRegionAtlas::bundled();
        for (latitude, longitude) in [
            (f64::NAN, 0.0),
            (0.0, f64::NAN),
            (f64::INFINITY, 0.0),
            (0.0, f64::NEG_INFINITY),
            (91.0_f64.to_radians(), 0.0),
            (-91.0_f64.to_radians(), 0.0),
        ] {
            assert!(atlas.context_at(latitude, longitude).is_none());
            assert!(atlas.region_at(latitude, longitude).is_none());
        }
    }
}
