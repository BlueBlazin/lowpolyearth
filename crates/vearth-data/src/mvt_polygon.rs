//! Exact source-ring component plans in the MVT tile coordinate system.

pub(crate) type ComponentGroups = Option<Vec<Vec<std::ops::Range<usize>>>>;

pub(crate) fn groups_heap_bytes(groups: &ComponentGroups) -> usize {
    groups.as_ref().map_or(0, |groups| {
        groups.capacity() * std::mem::size_of::<Vec<std::ops::Range<usize>>>()
            + groups
                .iter()
                .map(|group| group.capacity() * std::mem::size_of::<std::ops::Range<usize>>())
                .sum::<usize>()
    })
}

impl<'a> PolygonComponents<'a> {
    pub(crate) fn from_plan(paths: &'a [Vec<[i32; 2]>], groups: &'a ComponentGroups) -> Self {
        Self {
            paths,
            groups: std::borrow::Cow::Borrowed(groups.as_deref().unwrap_or(&[])),
            position: 0,
            compound: groups.is_none(),
        }
    }
}

// MVT 2.1 polygon ordering: positive tile-space area starts an exterior;
// following negative rings are its holes. Source paths and identity remain
// untouched; only the owned consumer geometry is separated into components.
// https://github.com/mapbox/vector-tile-spec/blob/master/2.1/README.md#4344-polygon-geometry-type
pub(crate) struct PolygonComponent<'a> {
    pub(crate) paths: std::borrow::Cow<'a, [Vec<[i32; 2]>]>,
    pub(crate) seed_anchor: [i32; 2],
}

pub(crate) struct PolygonComponents<'a> {
    paths: &'a [Vec<[i32; 2]>],
    groups: std::borrow::Cow<'a, [Vec<std::ops::Range<usize>>]>,
    position: usize,
    compound: bool,
}

impl<'a> Iterator for PolygonComponents<'a> {
    type Item = PolygonComponent<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.compound {
            self.compound = false;
            return (!self.paths.is_empty()).then(|| PolygonComponent {
                paths: std::borrow::Cow::Borrowed(self.paths),
                seed_anchor: self
                    .paths
                    .iter()
                    .flatten()
                    .next()
                    .copied()
                    .unwrap_or([0, 0]),
            });
        }
        let group = self.groups.get(self.position)?;
        self.position += 1;
        if group.len() == 1 {
            let paths = &self.paths[group[0].clone()];
            return Some(PolygonComponent {
                paths: std::borrow::Cow::Borrowed(paths),
                seed_anchor: paths[0][0],
            });
        }
        // Interacting rings retain their original relative source order. A
        // merged group's variation anchor is spatial, never its source ordinal.
        let seed_anchor = group.iter().map(|range| self.paths[range.start][0]).min()?;
        let paths = group
            .iter()
            .flat_map(|range| self.paths[range.clone()].iter().cloned())
            .collect();
        Some(PolygonComponent {
            paths: std::borrow::Cow::Owned(paths),
            seed_anchor,
        })
    }
}

#[cfg(test)]
pub(crate) fn polygon_components(paths: &[Vec<[i32; 2]>]) -> PolygonComponents<'_> {
    let groups = polygon_component_groups(paths);
    PolygonComponents {
        paths,
        compound: groups.is_none(),
        groups: std::borrow::Cow::Owned(groups.unwrap_or_default()),
        position: 0,
    }
}

pub(crate) fn polygon_component_groups(paths: &[Vec<[i32; 2]>]) -> ComponentGroups {
    if paths.len() < 2 {
        return None;
    }
    let mut ends = Vec::new();
    for (index, ring) in paths.iter().enumerate() {
        if ring.len() < 3 {
            return None;
        }
        let area =
            ring.iter()
                .zip(ring.iter().cycle().skip(1))
                .try_fold(0_i128, |sum, (a, b)| {
                    sum.checked_add(
                        i128::from(a[0]) * i128::from(b[1]) - i128::from(b[0]) * i128::from(a[1]),
                    )
                })?;
        if area == 0 || (index == 0 && area < 0) {
            return None;
        }
        if area > 0 && index != 0 {
            ends.push(index);
        }
    }
    if ends.is_empty() {
        return None;
    }
    ends.push(paths.len());
    let mut start = 0;
    let components: Vec<_> = ends
        .iter()
        .map(|&end| {
            let range = start..end;
            start = end;
            let bounds = paths[range.clone()]
                .iter()
                .map(|ring| raw_ring_bounds(ring))
                .fold([[i32::MAX; 2], [i32::MIN; 2]], |[lo, hi], [a, b]| {
                    [
                        [lo[0].min(a[0]), lo[1].min(a[1])],
                        [hi[0].max(b[0]), hi[1].max(b[1])],
                    ]
                });
            (range, bounds)
        })
        .collect();
    // The bounds include holes, even malformed ones outside their exterior.
    // Keep any touching, crossing or overlapping components in one even-odd
    // domain. Disjoint groups can then be emitted independently without
    // turning a source XOR into a union across overlapping rings.
    let centres =
        components
            .iter()
            .fold([[i64::MAX; 2], [i64::MIN; 2]], |[lo, hi], (_, bounds)| {
                let center = [
                    i64::from(bounds[0][0]) + i64::from(bounds[1][0]),
                    i64::from(bounds[0][1]) + i64::from(bounds[1][1]),
                ];
                [
                    [lo[0].min(center[0]), lo[1].min(center[1])],
                    [hi[0].max(center[0]), hi[1].max(center[1])],
                ]
            });
    let axis = usize::from(centres[1][1] - centres[0][1] > centres[1][0] - centres[0][0]);
    let mut order: Vec<_> = (0..components.len()).collect();
    order.sort_unstable_by_key(|&index| components[index].1[0][axis]);
    let mut parents: Vec<_> = (0..components.len()).collect();
    fn root(parents: &mut [usize], mut index: usize) -> usize {
        while parents[index] != index {
            parents[index] = parents[parents[index]];
            index = parents[index];
        }
        index
    }
    for (position, &a_index) in order.iter().enumerate() {
        let (a_range, a_bounds) = &components[a_index];
        let a = &paths[a_range.clone()];
        for &b_index in &order[position + 1..] {
            let (b_range, b_bounds) = &components[b_index];
            if b_bounds[0][axis] > a_bounds[1][axis] {
                break;
            }
            if raw_bounds_disjoint(*a_bounds, *b_bounds) {
                continue;
            }
            let a_root = root(&mut parents, a_index);
            let b_root = root(&mut parents, b_index);
            if a_root == b_root {
                continue;
            }
            let b = &paths[b_range.clone()];
            if a.iter()
                .any(|ar| b.iter().any(|br| raw_rings_intersect(ar, br)))
                || b.iter()
                    .any(|ring| raw_component_contains(a, ring[0]) != Some(false))
                || a.iter()
                    .any(|ring| raw_component_contains(b, ring[0]) != Some(false))
            {
                parents[a_root.max(b_root)] = a_root.min(b_root);
            }
        }
    }
    let mut groups = vec![Vec::new(); components.len()];
    for (index, (range, _)) in components.into_iter().enumerate() {
        let owner = root(&mut parents, index);
        groups[owner].push(range);
    }
    groups.retain(|group| !group.is_empty());
    Some(groups)
}

fn raw_ring_bounds(ring: &[[i32; 2]]) -> [[i32; 2]; 2] {
    ring.iter()
        .fold([[i32::MAX; 2], [i32::MIN; 2]], |[lo, hi], p| {
            [
                [lo[0].min(p[0]), lo[1].min(p[1])],
                [hi[0].max(p[0]), hi[1].max(p[1])],
            ]
        })
}

fn raw_bounds_disjoint(a: [[i32; 2]; 2], b: [[i32; 2]; 2]) -> bool {
    (0..2).any(|axis| a[0][axis] > b[1][axis] || b[0][axis] > a[1][axis])
}

fn raw_orientation(a: [i32; 2], b: [i32; 2], p: [i32; 2]) -> i128 {
    let delta = |end: i32, start: i32| i128::from(end) - i128::from(start);
    delta(b[0], a[0]) * delta(p[1], a[1]) - delta(b[1], a[1]) * delta(p[0], a[0])
}

fn raw_ring_contains(ring: &[[i32; 2]], point: [i32; 2]) -> Option<bool> {
    let mut inside = false;
    for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
        let side = raw_orientation(a, b, point);
        if side == 0 && !raw_bounds_disjoint(raw_ring_bounds(&[a, b]), [point, point]) {
            return None;
        }
        if (a[1] > point[1]) != (b[1] > point[1]) && (side > 0) == (b[1] > a[1]) {
            inside = !inside;
        }
    }
    Some(inside)
}

pub(crate) fn raw_component_contains(paths: &[Vec<[i32; 2]>], point: [i32; 2]) -> Option<bool> {
    paths.iter().try_fold(false, |inside, ring| {
        Some(inside ^ raw_ring_contains(ring, point)?)
    })
}

fn raw_rings_intersect(a: &[[i32; 2]], b: &[[i32; 2]]) -> bool {
    if raw_bounds_disjoint(raw_ring_bounds(a), raw_ring_bounds(b)) {
        return false;
    }
    a.iter().zip(a.iter().cycle().skip(1)).any(|(&a0, &a1)| {
        b.iter().zip(b.iter().cycle().skip(1)).any(|(&b0, &b1)| {
            if raw_bounds_disjoint(raw_ring_bounds(&[a0, a1]), raw_ring_bounds(&[b0, b1])) {
                return false;
            }
            let sides_a = [raw_orientation(a0, a1, b0), raw_orientation(a0, a1, b1)];
            let sides_b = [raw_orientation(b0, b1, a0), raw_orientation(b0, b1, a1)];
            (sides_a[0] == 0 || sides_a[1] == 0 || sides_a[0].signum() != sides_a[1].signum())
                && (sides_b[0] == 0
                    || sides_b[1] == 0
                    || sides_b[0].signum() != sides_b[1].signum())
        })
    })
}
