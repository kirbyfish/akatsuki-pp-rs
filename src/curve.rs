#![cfg(any(
    feature = "fruits",
    all(feature = "osu", not(feature = "no_sliders_no_leniency"))
))]

use std::{borrow::Cow, cmp::Ordering, convert::identity, f32::consts::PI};

use crate::{
    math_util,
    parse::{PathControlPoint, PathType, Pos2},
};

const BEZIER_TOLERANCE: f32 = 0.25;
const CATMULL_DETAIL: usize = 50;
const CIRCULAR_ARC_TOLERANCE: f32 = 0.1;

struct BezierBuffers {
    buf1: Vec<Pos2>,
    buf2: Vec<Pos2>,
    buf3: Vec<Pos2>,
}

impl BezierBuffers {
    fn new(len: usize) -> Self {
        Self {
            buf1: vec![Pos2::zero(); len],
            buf2: vec![Pos2::zero(); len],
            buf3: vec![Pos2::zero(); len],
        }
    }
}

struct CircularArcProperties {
    theta_start: f32,
    theta_range: f32,
    direction: f32,
    radius: f32,
    centre: Pos2,
}

pub(crate) struct Curve {
    path: Vec<Pos2>,
    lengths: Vec<f32>,
}

impl Curve {
    pub(crate) fn new(points: &[PathControlPoint], expected_len: f32) -> Self {
        let mut path = Self::calculate_path(points);
        let lengths = Self::calculate_length(points, &mut path, expected_len);

        Self { path, lengths }
    }

    pub(crate) fn position_at(&self, progress: f32) -> Pos2 {
        let d = self.progress_to_dist(progress);
        let i = self.idx_of_dist(d);

        self.interpolate_vertices(i, d)
    }

    fn progress_to_dist(&self, progress: f32) -> f32 {
        progress.clamp(0.0, 1.0) * self.dist()
    }

    pub(crate) fn dist(&self) -> f32 {
        self.lengths.last().copied().unwrap_or(0.0)
    }

    fn idx_of_dist(&self, d: f32) -> usize {
        self.lengths
            .binary_search_by(|len| len.partial_cmp(&d).unwrap_or(Ordering::Equal))
            .map_or_else(identity, identity)
    }

    fn interpolate_vertices(&self, i: usize, d: f32) -> Pos2 {
        if self.path.is_empty() {
            return Pos2::zero();
        }

        if i == 0 {
            return self.path[0];
        } else if i >= self.path.len() {
            return self.path[self.path.len() - 1];
        }

        let p0 = self.path[i - 1];
        let p1 = self.path[i];

        let d0 = self.lengths[i - 1];
        let d1 = self.lengths[i];

        // * Avoid division by an almost-zero number in case
        // * two points are extremely close to each other
        if (d0 - d1).abs() <= f32::EPSILON {
            return p0;
        }

        let w = (d - d0) / (d1 - d0);

        p0 + (p1 - p0) * w
    }

    fn calculate_path(points: &[PathControlPoint]) -> Vec<Pos2> {
        if points.is_empty() {
            return Vec::new();
        }

        let mut path = Vec::new();
        let vertices: Vec<_> = points.iter().map(|p| p.pos).collect();
        let mut start = 0;

        for i in 0..points.len() {
            if points[i].kind.is_none() && i < points.len() - 1 {
                continue;
            }

            // * The current vertex ends the segment
            let segment_vertices = &vertices[start..i + 1];
            let segment_kind = points[start].kind.unwrap_or(PathType::Linear);

            Self::calculate_subpath(&mut path, segment_vertices, segment_kind);

            // * Start the new segment at the current vertex
            start = i;
        }

        path.dedup();

        path
    }

    fn calculate_length(
        points: &[PathControlPoint],
        path: &mut Vec<Pos2>,
        expected_len: f32,
    ) -> Vec<f32> {
        let mut calculated_len = 0.0;
        let mut cumulative_len = vec![0.0];

        for i in 0..path.len() - 1 {
            let diff = path[i + 1] - path[i];
            calculated_len += diff.length();
            cumulative_len.push(calculated_len);
        }

        if (expected_len - calculated_len).abs() > f32::EPSILON {
            // * In osu-stable, if the last two control points of a slider are equal, extension is not performed
            let condition_opt = points
                .len()
                .checked_sub(2)
                .and_then(|i| points.get(i..))
                .filter(|suffix| suffix[0].pos == suffix[1].pos && expected_len > calculated_len);

            if condition_opt.is_some() {
                cumulative_len.push(calculated_len);

                return cumulative_len;
            }

            // * The last length is always incorrect
            cumulative_len.pop();

            let mut path_end_idx = path.len() - 1;

            if calculated_len > expected_len {
                // * The path will be shortened further, in which case we should trim
                // * any more unnecessary lengths and their associated path segments
                while cumulative_len
                    .last()
                    .filter(|&l| *l > expected_len)
                    .is_some()
                {
                    cumulative_len.pop();
                    path.remove(path_end_idx);
                    path_end_idx -= 1;
                }
            }

            if path_end_idx == 0 {
                // * The expected distance is negative or zero
                // * Perhaps negative path lengths should be disallowed altogether
                cumulative_len.push(0.0);

                return cumulative_len;
            }

            // * The direction of the segment to shorten or lengthen
            let dir = (path[path_end_idx] - path[path_end_idx - 1]).normalize();

            path[path_end_idx] =
                path[path_end_idx - 1] + dir * (expected_len - cumulative_len.last().unwrap());
            cumulative_len.push(expected_len);
        }

        cumulative_len
    }

    fn calculate_subpath(path: &mut Vec<Pos2>, sub_points: &[Pos2], kind: PathType) {
        match kind {
            PathType::Bezier => Self::approximate_bezier(path, sub_points),
            PathType::Catmull => Self::approximate_catmull(path, sub_points),
            PathType::Linear => Self::approximate_linear(path, sub_points),
            PathType::PerfectCurve => {
                if let [a, b, c] = sub_points {
                    Self::approximate_circular_arc(path, *a, *b, *c)
                } else {
                    Self::approximate_bezier(path, sub_points)
                }
            }
        }
    }

    fn approximate_bezier(path: &mut Vec<Pos2>, points: &[Pos2]) {
        let mut bufs = BezierBuffers::new(points.len()); // TODO: argument?

        Self::approximate_bspline(path, points, &mut bufs);
    }

    fn approximate_catmull(path: &mut Vec<Pos2>, points: &[Pos2]) {
        path.reserve_exact((points.len() - 1) * CATMULL_DETAIL * 2);

        let catmull_detail = CATMULL_DETAIL as f32;

        for i in 0..points.len() - 1 {
            let v2 = points[i];

            let v1 = i
                .checked_sub(1)
                .and_then(|i| points.get(i).copied())
                .unwrap_or(v2);

            let v3 = points.get(i + 1).copied().unwrap_or_else(|| v2 * 2.0 - v1);
            let v4 = points.get(i + 2).copied().unwrap_or_else(|| v3 * 2.0 - v2);

            for c in 0..CATMULL_DETAIL {
                let p1 = Self::catmull_find_point(v1, v2, v3, v4, c as f32 / catmull_detail);
                let p2 = Self::catmull_find_point(v1, v2, v3, v4, (c + 1) as f32 / catmull_detail);

                path.push(p1);
                path.push(p2);
            }
        }
    }

    fn approximate_linear(path: &mut Vec<Pos2>, points: &[Pos2]) {
        path.extend(points)
    }

    fn approximate_circular_arc(path: &mut Vec<Pos2>, a: Pos2, b: Pos2, c: Pos2) {
        let pr = match Self::circular_arc_properties(a, b, c) {
            Some(pr) => pr,
            None => return Self::approximate_bezier(path, &[a, b, c]),
        };

        // * We select the amount of points for the approximation by requiring the discrete curvature
        // * to be smaller than the provided tolerance. The exact angle required to meet the tolerance
        // * is: 2 * Math.Acos(1 - TOLERANCE / r)
        // * The special case is required for extremely short sliders where the radius is smaller than
        // * the tolerance. This is a pathological rather than a realistic case.
        let amount_points = if 2.0 * pr.radius <= CIRCULAR_ARC_TOLERANCE {
            2
        } else {
            let divisor = 2.0 * (1.0 - CIRCULAR_ARC_TOLERANCE / pr.radius).acos();

            ((pr.theta_range / divisor).ceil() as usize).max(2)
        };

        path.reserve_exact(amount_points);
        let divisor = (amount_points - 1) as f32;
        let directed_range = pr.direction * pr.theta_range;

        let subpath = (0..amount_points).map(|i| {
            let fract = i as f32 / divisor;
            let theta = pr.theta_start + fract * directed_range;
            let (sin, cos) = theta.sin_cos();
            let origin = Pos2 { x: cos, y: sin };

            pr.centre + origin * pr.radius
        });

        path.extend(subpath);
    }

    fn approximate_bspline(path: &mut Vec<Pos2>, points: &[Pos2], bufs: &mut BezierBuffers) {
        let p = points.len();

        let mut to_flatten = Vec::new();
        let mut free_bufs = Vec::with_capacity(1);

        // In osu!lazer's code, `p` is always 0 so the first big `if` can be omitted

        to_flatten.push(Cow::Borrowed(points));

        // * "toFlatten" contains all the curves which are not yet approximated well enough.
        // * We use a stack to emulate recursion without the risk of running into a stack overflow.
        // * (More specifically, we iteratively and adaptively refine our curve with a
        // * <a href="https://en.wikipedia.org/wiki/Depth-first_search">Depth-first search</a>
        // * over the tree resulting from the subdivisions we make.)

        let mut left_child = bufs.buf2.to_owned();

        while let Some(mut parent) = to_flatten.pop() {
            if Self::bezier_is_flat_enough(&parent) {
                // * If the control points we currently operate on are sufficiently "flat", we use
                // * an extension to De Casteljau's algorithm to obtain a piecewise-linear approximation
                // * of the bezier curve represented by our control points, consisting of the same amount
                // * of points as there are control points.
                Self::bezier_approximate(&parent, path, bufs);
                free_bufs.push(parent);

                continue;
            }

            // * If we do not yet have a sufficiently "flat" (in other words, detailed) approximation we keep
            // * subdividing the curve we are currently operating on.
            let mut right_child = free_bufs
                .pop()
                .unwrap_or_else(|| Cow::Owned(vec![Pos2::zero(); p]));

            Self::bezier_subdivide(
                &parent,
                &mut left_child,
                right_child.to_mut(),
                &mut bufs.buf1,
            );

            // * We re-use the buffer of the parent for one of the children, so that we save one allocation per iteration.
            parent.to_mut().copy_from_slice(&left_child[..p]);

            to_flatten.push(right_child);
            to_flatten.push(parent);
        }

        path.push(points[p - 1]);
    }

    fn bezier_is_flat_enough(points: &[Pos2]) -> bool {
        let limit = BEZIER_TOLERANCE * BEZIER_TOLERANCE * 4.0;

        !points
            .iter()
            .zip(points.iter().skip(1))
            .zip(points.iter().skip(2))
            .any(|((&prev, &curr), &next)| (prev - curr * 2.0 + next).length_squared() > limit)
    }

    fn bezier_subdivide(points: &[Pos2], l: &mut [Pos2], r: &mut [Pos2], midpoints: &mut [Pos2]) {
        let count = points.len();
        midpoints[..count].copy_from_slice(&points[..count]);

        for i in (1..count).rev() {
            l[count - i - 1] = midpoints[0];
            r[i] = midpoints[i];

            for j in 0..i {
                midpoints[j] = (midpoints[j] + midpoints[j + 1]) / 2.0;
            }
        }

        l[count - 1] = midpoints[0];
        r[0] = midpoints[0];
    }

    // * https://en.wikipedia.org/wiki/De_Casteljau%27s_algorithm
    fn bezier_approximate(points: &[Pos2], path: &mut Vec<Pos2>, bufs: &mut BezierBuffers) {
        let count = points.len();

        let BezierBuffers {
            buf1: l,
            buf2: r,
            buf3: midpoints,
        } = bufs;

        Self::bezier_subdivide(points, l, r, midpoints);
        path.push(points[0]);

        let l = &l[..count];
        let r = &r[1..count];

        let left_iter = l
            .iter()
            .skip(1)
            .zip(l.iter().skip(2))
            .zip(l.iter().skip(3))
            .step_by(2);

        let right_iter = r
            .iter()
            .skip(1)
            .zip(r.iter().skip(2))
            .zip(r.iter().skip(3))
            .step_by(2);

        let subpath = left_iter
            .chain(right_iter)
            .map(|((&prev, &curr), &next)| (prev + curr * 2.0 + next) * 0.25);

        path.extend(subpath);
    }

    fn catmull_find_point(v1: Pos2, v2: Pos2, v3: Pos2, v4: Pos2, t: f32) -> Pos2 {
        let t2 = t * t;
        let t3 = t * t * t;

        let x = 0.5
            * (2.0 * v2.x
                + (-v1.x + v3.x) * t
                + (2.0 * v1.x - 5.0 * v2.x + 4.0 * v3.x - v4.x) * t2
                + (-v1.x + 3.0 * v2.x - 3.0 * v3.x + v4.x) * t3);

        let y = 0.5
            * (2.0 * v2.y
                + (-v1.y + v3.y) * t
                + (2.0 * v1.y - 5.0 * v2.y + 4.0 * v3.y - v4.y) * t2
                + (-v1.y + 3.0 * v2.y - 3.0 * v3.y + v4.y) * t3);

        Pos2 { x, y }
    }

    fn circular_arc_properties(a: Pos2, b: Pos2, c: Pos2) -> Option<CircularArcProperties> {
        // * If we have a degenerate triangle where a side-length is almost zero,
        // * then give up and fallback to a more numerically stable method.
        if ((b.y - a.y) * (c.x - a.x) - (b.x - a.x) * (c.y - a.y)).abs() <= f32::EPSILON {
            return None;
        }

        let d = 2.0 * (a.x * (b - c).y + b.x * (c - a).y + c.x * (a - b).y);
        let a_sq = a.length_squared();
        let b_sq = b.length_squared();
        let c_sq = c.length_squared();

        let centre = Pos2 {
            x: (a_sq * (b - c).y + b_sq * (c - a).y + c_sq * (a - b).y) / d,
            y: ((c - b).x + b_sq * (a - c).x + c_sq * (b - a).x) / d,
        };

        let d_a = a - centre;
        let d_c = c - centre;

        let radius = d_a.length();

        let theta_start = d_a.y.atan2(d_a.x);
        let mut theta_end = d_c.y.atan2(d_c.x);

        while theta_end < theta_start {
            theta_end += 2.0 * PI;
        }

        let mut direction = 1.0;
        let mut theta_range = theta_end - theta_start;

        // * Decide in which direction to draw the circle,
        // * depending on which side of AC B lies.
        let mut ortho_a_to_c = c - a;

        ortho_a_to_c = Pos2 {
            x: ortho_a_to_c.y,
            y: -ortho_a_to_c.x,
        };

        if ortho_a_to_c.dot(b - a) < 0.0 {
            direction = -direction;
            theta_range = 2.0 * PI - theta_range;
        }

        Some(CircularArcProperties {
            theta_start,
            theta_range,
            direction,
            radius,
            centre,
        })
    }
}

pub(crate) enum Points {
    Single(Pos2),
    Multi(Vec<Pos2>),
}

impl Points {
    #[inline]
    fn point_at_distance(&self, dist: f32) -> Pos2 {
        match self {
            Points::Multi(points) => math_util::point_at_distance(points, dist),
            Points::Single(point) => *point,
        }
    }
}

pub(crate) enum Curve_<'p> {
    Bezier {
        path: Vec<Pos2>,
        lengths: Vec<f32>,
    },
    Catmull(Points),
    Linear(&'p [Pos2]),
    Perfect {
        origin: Pos2,
        center: Pos2,
        radius: f32,
    },
}

impl<'p> Curve_<'p> {
    #[inline]
    pub(crate) fn new(points: &'p [Pos2], kind: PathType, expected_len: f32) -> Self {
        match kind {
            PathType::Bezier => Self::bezier(points, expected_len),
            PathType::Catmull => Self::catmull(points),
            PathType::Linear => Self::Linear(points),
            PathType::PerfectCurve => Self::perfect(points),
        }
    }

    fn bezier(points: &[Pos2], expected_len: f32) -> Self {
        let points: Vec<_> = points
            .iter()
            .copied()
            .map(|point| point - points[0])
            .collect();

        let len = points.len();

        if len == 1 {
            return Self::Bezier {
                path: points,
                lengths: vec![0.0],
            };
        }

        // First calculate a path of coordinates
        let mut start = 0;
        let mut path = Vec::new();
        let mut bufs = BezierBuffers::new(len);

        for (end, (curr, next)) in (1..).zip(points.iter().zip(points.iter().skip(1))) {
            if end - start > 1 && curr == next {
                Self::bezier_subpath(&mut path, &points[start..end], &mut bufs);
                start = end;
            }
        }

        Self::bezier_subpath(&mut path, &points[start..], &mut bufs);
        let last_point = &points[len - 1];
        path.push(*last_point);

        // Then calculated cumulative lenghts
        let mut calculated_len = 0.0;
        let mut cumulative_len = vec![0.0];

        for i in 0..path.len() - 1 {
            let diff = path[i + 1] - path[i];
            calculated_len += diff.length();
            cumulative_len.push(calculated_len);
        }

        if (expected_len - calculated_len).abs() > f32::EPSILON {
            // * In osu-stable, if the last two control points of a slider are equal, extension is not performed
            if points
                .get(len - 2)
                .filter(|&p| p == last_point && expected_len > calculated_len)
                .is_some()
            {
                cumulative_len.push(calculated_len);

                return Self::Bezier {
                    path,
                    lengths: cumulative_len,
                };
            }

            // * The last length is always incorrect
            cumulative_len.pop();

            let mut path_end_idx = path.len() - 1;

            if calculated_len > expected_len {
                // * The path will be shortened further, in which case we should trim
                // * any more unnecessary lengths and their associated path segments
                while cumulative_len
                    .last()
                    .filter(|&l| *l > expected_len)
                    .is_some()
                {
                    cumulative_len.pop();
                    path.remove(path_end_idx);
                    path_end_idx -= 1;
                }
            }

            if path_end_idx == 0 {
                // * The expected distance is negative or zero
                // * Perhaps negative path lengths should be disallowed altogether
                cumulative_len.push(0.0);

                return Self::Bezier {
                    path,
                    lengths: cumulative_len,
                };
            }

            // * The direction of the segment to shorten or lengthen
            let dir = (path[path_end_idx] - path[path_end_idx - 1]).normalize();

            path[path_end_idx] =
                path[path_end_idx - 1] + dir * (expected_len - cumulative_len.last().unwrap());
            cumulative_len.push(expected_len);
        }

        Self::Bezier {
            path,
            lengths: cumulative_len,
        }
    }

    fn bezier_subpath(result: &mut Vec<Pos2>, points: &[Pos2], bufs: &mut BezierBuffers) {
        let p = points.len();

        let mut to_flatten = Vec::new();
        let mut free_bufs = Vec::with_capacity(1);

        // In osu!lazer's code, `p` is always 0 when approximating bezier
        // so the first big `if` can be omitted

        to_flatten.push(Cow::Borrowed(points));

        // * "toFlatten" contains all the curves which are not yet approximated well enough.
        // * We use a stack to emulate recursion without the risk of running into a stack overflow.
        // * (More specifically, we iteratively and adaptively refine our curve with a
        // * <a href="https://en.wikipedia.org/wiki/Depth-first_search">Depth-first search</a>
        // * over the tree resulting from the subdivisions we make.)

        let mut left_child = bufs.buf2.to_owned();

        while let Some(mut parent) = to_flatten.pop() {
            if Self::bezier_is_flat_enough(&parent) {
                // * If the control points we currently operate on are sufficiently "flat", we use
                // * an extension to De Casteljau's algorithm to obtain a piecewise-linear approximation
                // * of the bezier curve represented by our control points, consisting of the same amount
                // * of points as there are control points.
                Self::bezier_approximate(&parent, result, bufs);
                free_bufs.push(parent);

                continue;
            }

            // * If we do not yet have a sufficiently "flat" (in other words, detailed) approximation we keep
            // * subdividing the curve we are currently operating on.
            let mut right_child = free_bufs
                .pop()
                .unwrap_or_else(|| Cow::Owned(vec![Pos2::zero(); p]));

            Self::bezier_subdivide(
                &parent,
                &mut left_child,
                right_child.to_mut(),
                &mut bufs.buf1,
            );

            // * We re-use the buffer of the parent for one of the children, so that we save one allocation per iteration.
            parent.to_mut().copy_from_slice(&left_child[..p]);

            to_flatten.push(right_child);
            to_flatten.push(parent);
        }
    }

    fn bezier_is_flat_enough(points: &[Pos2]) -> bool {
        let limit = BEZIER_TOLERANCE * BEZIER_TOLERANCE * 4.0;

        !points
            .iter()
            .zip(points.iter().skip(1))
            .zip(points.iter().skip(2))
            .any(|((&prev, &curr), &next)| (prev - curr * 2.0 + next).length_squared() > limit)
    }

    fn bezier_subdivide(points: &[Pos2], l: &mut [Pos2], r: &mut [Pos2], buf: &mut [Pos2]) {
        let count = points.len();
        let midpoints = buf;
        midpoints[..count].copy_from_slice(&points[..count]);

        for i in (1..count).rev() {
            l[count - i - 1] = midpoints[0];
            r[i] = midpoints[i];

            for j in 0..i {
                midpoints[j] = (midpoints[j] + midpoints[j + 1]) / 2.0;
            }
        }

        l[count - 1] = midpoints[0];
        r[0] = midpoints[0];
    }

    // * https://en.wikipedia.org/wiki/De_Casteljau%27s_algorithm
    fn bezier_approximate(points: &[Pos2], output: &mut Vec<Pos2>, bufs: &mut BezierBuffers) {
        let count = points.len();
        let r = &mut bufs.buf1;
        let l = &mut bufs.buf2;

        Self::bezier_subdivide(points, l, r, &mut bufs.buf3);
        l[count..2 * count - 1].copy_from_slice(&r[1..count]);
        output.push(points[0]);

        let new_points = l
            .iter()
            .skip(1)
            .zip(l.iter().skip(2))
            .zip(l.iter().skip(3))
            .step_by(2)
            .take(count.saturating_sub(2))
            .map(|((&prev, &curr), &next)| (prev + curr * 2.0 + next) * 0.25);

        output.extend(new_points);
    }

    fn catmull(points: &[Pos2]) -> Self {
        let len = points.len();

        if len == 1 {
            return Self::Catmull(Points::Single(points[0]));
        }

        let mut result = Vec::with_capacity((len * CATMULL_DETAIL * 2) as usize);

        // Handle first iteration distinctly because of v1
        let v1 = points[0];
        let v2 = points[0];
        let v3 = points.get(1).copied().unwrap_or(v2);
        let v4 = points.get(2).copied().unwrap_or_else(|| v3 * 2.0 - v2);

        Self::catmull_points(&mut result, v1, v2, v3, v4);

        // Remaining iterations
        for (i, (&v1, &v2)) in (2..).zip(points.iter().zip(points.iter().skip(1))) {
            let v3 = points.get(i).copied().unwrap_or_else(|| v2 * 2.0 - v1);
            let v4 = points.get(i + 1).copied().unwrap_or_else(|| v3 * 2.0 - v2);

            Self::catmull_points(&mut result, v1, v2, v3, v4);
        }

        Self::Catmull(Points::Multi(result))
    }

    #[inline]
    fn catmull_points(result: &mut Vec<Pos2>, v1: Pos2, v2: Pos2, v3: Pos2, v4: Pos2) {
        let mut c = 0.0;

        let x1 = 2.0 * v1.x;
        let x2 = -v1.x + v3.x;
        let x3 = 2.0 * v1.x - 5.0 * v2.x + 4.0 * v3.x - v4.x;
        let x4 = -v1.x + 3.0 * (v2.x - v3.x) + v4.x;

        let y1 = 2.0 * v1.y;
        let y2 = -v1.y + v3.y;
        let y3 = 2.0 * v1.y - 5.0 * v2.y + 4.0 * v3.y - v4.y;
        let y4 = -v1.y + 3.0 * (v2.y - v3.y) + v4.y;

        let catmull_detail = CATMULL_DETAIL as f32;

        loop {
            let t1 = c / catmull_detail;
            let t2 = t1 * t1;
            let t3 = t2 * t1;

            result.push(Pos2 {
                x: 0.5 * (x1 + x2 * t1 + x3 * t2 + x4 * t3),
                y: 0.5 * (y1 + y2 * t1 + y3 * t2 + y4 * t3),
            });

            let t1 = (c + 1.0) / catmull_detail;
            let t2 = t1 * t1;
            let t3 = t2 * t1;

            result.push(Pos2 {
                x: 0.5 * (x1 + x2 * t1 + x3 * t2 + x4 * t3),
                y: 0.5 * (y1 + y2 * t1 + y3 * t2 + y4 * t3),
            });

            c += 1.0;

            if c >= catmull_detail {
                return;
            }
        }
    }

    fn perfect(points: &[Pos2]) -> Self {
        let (a, b, c) = (points[0], points[1], points[2]);
        let (center, mut radius) = math_util::get_circum_circle(a, b, c);
        radius *= ((!math_util::is_left(a, b, c)) as i8 * 2 - 1) as f32;

        Self::Perfect {
            origin: a,
            center,
            radius,
        }
    }

    fn interpolate_vertices(path: &[Pos2], lengths: &[f32], i: usize, d: f32) -> Pos2 {
        if path.is_empty() {
            return Pos2::zero();
        }

        if i == 0 {
            return path[0];
        } else if i >= path.len() {
            return path[path.len() - 1];
        }

        let p0 = path[i - 1];
        let p1 = path[i];

        let d0 = lengths[i - 1];
        let d1 = lengths[i];

        // * Avoid division by an almost-zero number in case
        // * two points are extremely close to each other
        if (d0 - d1).abs() <= f32::EPSILON {
            return p0;
        }

        let w = (d - d0) / (d1 - d0);

        p0 + (p1 - p0) * w
    }

    pub(crate) fn point_at_distance(&self, dist: f32) -> Pos2 {
        match self {
            Self::Bezier { path, lengths } => {
                let idx = lengths
                    .binary_search_by(|len| len.partial_cmp(&dist).unwrap_or(Ordering::Equal))
                    .map_or_else(identity, identity);

                Self::interpolate_vertices(path, lengths, idx, dist)
            }
            Self::Catmull(points) => points.point_at_distance(dist),
            Self::Linear(points) => math_util::point_at_distance(points, dist),
            Self::Perfect {
                origin,
                center,
                radius,
            } => math_util::rotate(*center, *origin, dist / *radius),
        }
    }
}
