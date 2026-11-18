//! Local `f32` geometric primitives (`Aabb`, `Sphere`, `Plane`, `Ray`; ADR-0010).
//!
//! All types are `#[repr(C)]`, `Copy`, lifetime-free, and allocation-free.
//! Physics broadphase, collision manifolds, and spatial acceleration structures belong to
//! `spawn-physics`, not `spawn-math`.

use crate::{Mat4, MathError, MathErrorKind, Vec3};

/// Local-space axis-aligned bounding box (`min <= max` per axis).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Aabb {
    /// Minimum corner (`x_min, y_min, z_min`).
    pub min: Vec3,
    /// Maximum corner (`x_max, y_max, z_max`).
    pub max: Vec3,
}

impl Aabb {
    /// Zero-volume box at the origin.
    pub const ZERO: Self = Self {
        min: Vec3::ZERO,
        max: Vec3::ZERO,
    };

    /// Constructs a validated `Aabb` requiring finite corners and `min <= max` on every axis.
    ///
    /// # Errors
    ///
    /// Returns [`MathError`] (`MATH-0005` [`MathErrorKind::InvalidBounds`]) if any coordinate is
    /// non-finite or `min.x > max.x || min.y > max.y || min.z > max.z`.
    #[track_caller]
    pub fn try_new(min: Vec3, max: Vec3) -> Result<Self, MathError> {
        if !min.is_finite() || !max.is_finite() || min.x > max.x || min.y > max.y || min.z > max.z {
            return Err(MathError::new(MathErrorKind::InvalidBounds)
                .with_detail(format!("min = {min:?}, max = {max:?}")));
        }
        Ok(Self { min, max })
    }

    /// Constructs a validated `Aabb` from `center` and non-negative `half_extents`.
    ///
    /// # Errors
    ///
    /// Returns [`MathError`] (`MATH-0005` [`MathErrorKind::InvalidBounds`]) if `center` or
    /// `half_extents` is non-finite, any half-extent is negative, or `center ± half_extents` overflows.
    #[track_caller]
    pub fn try_from_center_half_extents(
        center: Vec3,
        half_extents: Vec3,
    ) -> Result<Self, MathError> {
        if !center.is_finite()
            || !half_extents.is_finite()
            || half_extents.x < 0.0
            || half_extents.y < 0.0
            || half_extents.z < 0.0
        {
            return Err(
                MathError::new(MathErrorKind::InvalidBounds).with_detail(format!(
                    "center = {center:?}, half_extents = {half_extents:?}"
                )),
            );
        }
        Self::try_new(center - half_extents, center + half_extents)
    }

    /// Zero-extent box containing `point` (`None` if `point` is non-finite).
    #[inline]
    pub fn try_from_point(point: Vec3) -> Option<Self> {
        point.is_finite().then_some(Self {
            min: point,
            max: point,
        })
    }

    /// Tightest `Aabb` enclosing a non-empty slice of finite points (`None` if `points` is empty
    /// or contains any non-finite point).
    #[inline]
    pub fn from_points(points: &[Vec3]) -> Option<Self> {
        let (&first, rest) = points.split_first()?;
        if !first.is_finite() {
            return None;
        }
        let mut min = first;
        let mut max = first;
        for &p in rest {
            if !p.is_finite() {
                return None;
            }
            min = min.min(p);
            max = max.max(p);
        }
        Some(Self { min, max })
    }

    /// Returns `true` if both corners are finite and `min <= max` on all axes.
    #[inline]
    pub fn is_valid(self) -> bool {
        self.min.is_finite()
            && self.max.is_finite()
            && self.min.x <= self.max.x
            && self.min.y <= self.max.y
            && self.min.z <= self.max.z
    }

    /// Center point `(min + max) * 0.5` (computed as `min * 0.5 + max * 0.5` to avoid overflow).
    #[inline]
    pub fn center(self) -> Vec3 {
        self.min * 0.5 + self.max * 0.5
    }

    /// Full side lengths `max - min`.
    #[inline]
    pub fn extents(self) -> Vec3 {
        self.max - self.min
    }

    /// Half side lengths `(max - min) * 0.5` (overflow-safe).
    #[inline]
    pub fn half_extents(self) -> Vec3 {
        self.max * 0.5 - self.min * 0.5
    }

    /// Returns `true` if `self` is valid and `point` lies inside or on the boundary of `self`.
    #[inline]
    pub fn contains_point(self, point: Vec3) -> bool {
        self.is_valid()
            && point.is_finite()
            && point.x >= self.min.x
            && point.x <= self.max.x
            && point.y >= self.min.y
            && point.y <= self.max.y
            && point.z >= self.min.z
            && point.z <= self.max.z
    }

    /// Clamps `point` to the nearest point inside or on `self` (`NAN` if `self` is invalid).
    #[inline]
    pub fn closest_point(self, point: Vec3) -> Vec3 {
        point.clamp(self.min, self.max)
    }

    /// Smallest `Aabb` enclosing both `self` and `other`.
    #[inline]
    pub fn union(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    /// Expands `self` to include `point` (no-op if `point` is non-finite).
    #[inline]
    pub fn expand_to_include(self, point: Vec3) -> Self {
        if !point.is_finite() {
            return self;
        }
        Self {
            min: self.min.min(point),
            max: self.max.max(point),
        }
    }

    /// Computes the tightest axis-aligned bounding box of `self` after applying an affine `mat`
    /// (Arvo's column-wise min/max method).
    #[inline]
    pub fn transformed_by_affine_mat4(self, mat: Mat4) -> Self {
        let mut new_min = mat.w_axis.xyz();
        let mut new_max = new_min;
        for (col, (lo, hi)) in [
            (mat.x_axis.xyz(), (self.min.x, self.max.x)),
            (mat.y_axis.xyz(), (self.min.y, self.max.y)),
            (mat.z_axis.xyz(), (self.min.z, self.max.z)),
        ] {
            let a = col * lo;
            let b = col * hi;
            new_min += a.min(b);
            new_max += a.max(b);
        }
        Self {
            min: new_min,
            max: new_max,
        }
    }
}

/// Local-space bounding sphere (`radius >= 0.0`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Sphere {
    /// Sphere center.
    pub center: Vec3,
    /// Non-negative sphere radius (`meters`).
    pub radius: f32,
}

impl Sphere {
    /// Unit sphere at the origin (`center = ZERO`, `radius = 1.0`).
    pub const UNIT: Self = Self {
        center: Vec3::ZERO,
        radius: 1.0,
    };

    /// Constructs a validated `Sphere` requiring finite `center` and finite `radius >= 0.0`.
    ///
    /// # Errors
    ///
    /// Returns [`MathError`] (`MATH-0005` [`MathErrorKind::InvalidBounds`]) if `center` or `radius`
    /// is non-finite or `radius < 0.0`.
    #[track_caller]
    pub fn try_new(center: Vec3, radius: f32) -> Result<Self, MathError> {
        if !center.is_finite() || !radius.is_finite() || radius < 0.0 {
            return Err(MathError::new(MathErrorKind::InvalidBounds)
                .with_detail(format!("center = {center:?}, radius = {radius}")));
        }
        Ok(Self { center, radius })
    }

    /// Returns `true` if `center` and `radius` are finite and `radius >= 0.0`.
    #[inline]
    pub fn is_valid(self) -> bool {
        self.center.is_finite() && self.radius.is_finite() && self.radius >= 0.0
    }

    /// Returns `true` if `self` is valid and `point` lies inside or on the sphere surface.
    #[inline]
    pub fn contains_point(self, point: Vec3) -> bool {
        if !self.is_valid() || !point.is_finite() {
            return false;
        }
        point.distance(self.center) <= self.radius
    }

    /// Tightest [`Aabb`] enclosing this sphere.
    #[inline]
    pub fn bounding_aabb(self) -> Aabb {
        let r = Vec3::splat(self.radius);
        Aabb {
            min: self.center - r,
            max: self.center + r,
        }
    }
}

/// Oriented 3D plane defined by `normal.dot(p) + d == 0`, where `normal` has unit length.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plane {
    /// Unit plane normal pointing into the positive half-space.
    pub normal: Vec3,
    /// Signed offset along `-normal` (`d = -normal.dot(point_on_plane)`).
    pub d: f32,
}

impl Default for Plane {
    #[inline]
    fn default() -> Self {
        Self::XZ
    }
}

impl Plane {
    /// Horizontal ground plane `y = 0` with upward normal `+Y` (`normal = (0, 1, 0), d = 0`).
    pub const XZ: Self = Self {
        normal: Vec3::Y,
        d: 0.0,
    };

    /// Constructs a validated `Plane` passing through `point` with outward `normal` (normalized internally).
    ///
    /// # Errors
    ///
    /// Returns [`MathError`] (`MATH-0004` [`MathErrorKind::DegenerateDirection`]) if `point` or
    /// `normal` is non-finite or `normal` has zero length.
    #[track_caller]
    pub fn try_from_point_normal(point: Vec3, normal: Vec3) -> Result<Self, MathError> {
        if !point.is_finite() || !normal.is_finite() {
            return Err(MathError::new(MathErrorKind::DegenerateDirection)
                .with_detail(format!("point = {point:?}, normal = {normal:?}")));
        }
        let Some(unit_normal) = normal.try_normalize() else {
            return Err(MathError::new(MathErrorKind::DegenerateDirection)
                .with_detail("plane normal is zero"));
        };
        let d = -unit_normal.dot(point);
        if !d.is_finite() {
            return Err(MathError::new(MathErrorKind::DegenerateDirection)
                .with_detail("plane offset overflowed"));
        }
        Ok(Self {
            normal: unit_normal,
            d,
        })
    }

    /// Constructs a validated `Plane` from three non-collinear points `(a, b, c)` using right-handed
    /// counter-clockwise front-face orientation (`normal = normalize((b - a) × (c - a))`).
    ///
    /// # Errors
    ///
    /// Returns [`MathError`] (`MATH-0004` [`MathErrorKind::DegenerateDirection`]) if the points
    /// are collinear, coincident, or non-finite.
    #[track_caller]
    pub fn try_from_points_ccw(a: Vec3, b: Vec3, c: Vec3) -> Result<Self, MathError> {
        if !a.is_finite() || !b.is_finite() || !c.is_finite() {
            return Err(MathError::new(MathErrorKind::DegenerateDirection)
                .with_detail("non-finite triangle vertices for plane"));
        }
        Self::try_from_point_normal(a, (b - a).cross(c - a))
    }

    /// Returns `true` if `normal` is unit-length and `d` is finite.
    #[inline]
    pub fn is_valid(self) -> bool {
        self.normal.is_normalized() && self.d.is_finite()
    }

    /// Signed distance `normal.dot(point) + d` (`> 0` in front of the plane, `< 0` behind).
    #[inline]
    pub fn signed_distance(self, point: Vec3) -> f32 {
        self.normal.dot(point) + self.d
    }

    /// Orthogonal projection of `point` onto the plane.
    #[inline]
    pub fn project_point(self, point: Vec3) -> Vec3 {
        point - self.normal * self.signed_distance(point)
    }
}

/// Half-line `origin + direction * t` (`t >= 0.0`) with unit-length `direction`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    /// Ray origin.
    pub origin: Vec3,
    /// Unit ray direction.
    pub direction: Vec3,
}

impl Default for Ray {
    #[inline]
    fn default() -> Self {
        Self {
            origin: Vec3::ZERO,
            direction: Vec3::FORWARD,
        }
    }
}

impl Ray {
    /// Constructs a validated `Ray` from `origin` and `direction` (normalized internally).
    ///
    /// # Errors
    ///
    /// Returns [`MathError`] (`MATH-0004` [`MathErrorKind::DegenerateDirection`]) if `origin` or
    /// `direction` is non-finite or `direction` has zero length.
    #[track_caller]
    pub fn try_new(origin: Vec3, direction: Vec3) -> Result<Self, MathError> {
        if !origin.is_finite() || !direction.is_finite() {
            return Err(MathError::new(MathErrorKind::DegenerateDirection)
                .with_detail(format!("origin = {origin:?}, direction = {direction:?}")));
        }
        let Some(unit_dir) = direction.try_normalize() else {
            return Err(MathError::new(MathErrorKind::DegenerateDirection)
                .with_detail("ray direction is zero"));
        };
        Ok(Self {
            origin,
            direction: unit_dir,
        })
    }

    /// Returns `true` if `origin` is finite and `direction` is unit-length.
    #[inline]
    pub fn is_valid(self) -> bool {
        self.origin.is_finite() && self.direction.is_normalized()
    }

    /// Evaluates `origin + direction * t`.
    #[inline]
    pub fn at(self, t: f32) -> Vec3 {
        self.origin + self.direction * t
    }

    /// Smallest non-negative `t >= 0.0` where `self` hits `plane`, or `None` if parallel,
    /// pointing away, or invalid.
    #[inline]
    pub fn intersect_plane(self, plane: Plane) -> Option<f32> {
        if !self.is_valid() || !plane.is_valid() {
            return None;
        }
        let denom = plane.normal.dot(self.direction);
        if denom.abs() <= 1.0e-7 {
            return if plane.signed_distance(self.origin).abs() <= 1.0e-6 {
                Some(0.0)
            } else {
                None
            };
        }
        let t = -plane.signed_distance(self.origin) / denom;
        (t.is_finite() && t >= 0.0).then_some(t)
    }

    /// Smallest non-negative `t >= 0.0` where `self` enters or starts inside `sphere`, or `None`.
    #[inline]
    pub fn intersect_sphere(self, sphere: Sphere) -> Option<f32> {
        if !self.is_valid() || !sphere.is_valid() {
            return None;
        }
        let oc = self.origin - sphere.center;
        let b = oc.dot(self.direction);
        let c = oc.length_squared() - sphere.radius * sphere.radius;
        if c <= 0.0 {
            return Some(0.0);
        }
        if b > 0.0 {
            return None;
        }
        let disc = b * b - c;
        if disc < 0.0 || !disc.is_finite() {
            return None;
        }
        let t = (-b - disc.sqrt()).max(0.0);
        t.is_finite().then_some(t)
    }

    /// Smallest non-negative `t >= 0.0` where `self` enters or starts inside `aabb`, or `None`.
    ///
    /// Uses explicit parallel-slab checks so rays lying on an AABB face with zero axis direction
    /// never produce `0.0 * Inf = NaN`.
    #[inline]
    pub fn intersect_aabb(self, aabb: Aabb) -> Option<f32> {
        if !self.is_valid() || !aabb.is_valid() {
            return None;
        }
        let mut t_min = 0.0_f32;
        let mut t_max = f32::INFINITY;

        for (orig, dir, lo, hi) in [
            (self.origin.x, self.direction.x, aabb.min.x, aabb.max.x),
            (self.origin.y, self.direction.y, aabb.min.y, aabb.max.y),
            (self.origin.z, self.direction.z, aabb.min.z, aabb.max.z),
        ] {
            if dir.abs() < f32::MIN_POSITIVE {
                if orig < lo || orig > hi {
                    return None;
                }
            } else {
                let inv_d = 1.0 / dir;
                let mut t1 = (lo - orig) * inv_d;
                let mut t2 = (hi - orig) * inv_d;
                if t1 > t2 {
                    std::mem::swap(&mut t1, &mut t2);
                }
                t_min = t_min.max(t1);
                t_max = t_max.min(t2);
                if t_min > t_max {
                    return None;
                }
            }
        }
        (t_min.is_finite() && t_min >= 0.0).then_some(t_min)
    }
}
