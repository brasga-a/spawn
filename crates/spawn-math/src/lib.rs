//! `spawn-math`: engine-wide spatial, linear-algebra, geometric, and deterministic math primitives.
//!
//! ## Fixed Engine Conventions (ADR-0010, `SPN-INV-050..054`)
//!
//! 1. **Coordinate system:** Right-handed, `+X` right, `+Y` up, `-Z` forward (`+Z` back),
//!    matching glTF 2.0.
//! 2. **Units:** SI — meters (`m`), seconds (`s`), kilograms (`kg`). All Rust APIs accept and
//!    return angles in **radians**. External data and CLI boundaries using degrees must name
//!    the unit explicitly (for example `fov_degrees`) and convert once at the boundary via
//!    [`radians_from_degrees_f32`] / [`radians_from_degrees_f64`].
//! 3. **Matrices:** Column-major storage (`cols[col][row]`), multiplying column vectors on the
//!    right (`M * v`), matching SPIR-V and Slang defaults.
//! 4. **Rotations:** Unit [`Quat`] `(x, y, z, w)` is canonical. Euler helpers use intrinsic
//!    `YXZ` order: yaw around world `+Y`, then pitch around local `+X` (and optional roll
//!    around local `+Z`), with zero yaw/pitch looking down `-Z`.
//! 5. **Large-world coordinates (`SPN-INV-051`, `SPN-INV-052`):** Absolute world positions use
//!    [`WorldPos`] (`f64`, backed by [`DVec3`] displacements), strictly distinct at the type
//!    level from local `f32` [`Vec3`]. GPU transforms are camera-relative: subtract the `f64`
//!    camera [`WorldPos`] in `f64` via [`camera_relative_f32`] before narrowing to `f32`.
//! 6. **Depth and Vulkan Y-flip (`SPN-INV-053`):** [`Mat4::perspective_reversed_z_rh`] and
//!    [`Mat4::perspective_infinite_reversed_z_rh`] target Vulkan `[0, 1]` clip depth with
//!    **reversed-Z** (`near -> 1.0`, `far -> 0.0`) and apply the Vulkan NDC Y-flip (`y_clip = -g * y_e`)
//!    in this single site, preserving `VK_FRONT_FACE_COUNTER_CLOCKWISE` (`a > 0`) for standard
//!    glTF right-handed counter-clockwise meshes.
//! 7. **Determinism (`SPN-INV-044`):** [`det`] exposes IEEE-754 `sqrt` and pure-Rust `libm`
//!    wrappers (`default-features = false`) whose outputs match bit-for-bit across Linux and
//!    Windows for simulation, prediction, and replay paths.
//!
//! This crate depends on no other `spawn-*` crate (`SPN-INV-004`), exposes no third-party types
//! in its public API (`SPN-INV-011`), forbids `unsafe`, and never panics on degenerate inputs.

#![forbid(unsafe_code)]

pub mod det;
mod error;
mod geometry;
mod mat;
mod quat;
mod transform;
mod vec;
mod world;

pub use error::{MathError, MathErrorKind};
pub use geometry::{Aabb, Plane, Ray, Sphere};
pub use mat::{Mat3, Mat4};
pub use quat::Quat;
pub use transform::Transform;
pub use vec::{Vec2, Vec3, Vec4};
pub use world::{DVec3, WorldPos, camera_relative_f32};

/// Converts an external boundary angle from degrees to radians (`f32`; `SPN-INV-054`).
#[inline]
pub const fn radians_from_degrees_f32(degrees: f32) -> f32 {
    degrees.to_radians()
}

/// Converts an internal radian angle to degrees (`f32`; `SPN-INV-054`).
#[inline]
pub const fn degrees_from_radians_f32(radians: f32) -> f32 {
    radians.to_degrees()
}

/// Converts an external boundary angle from degrees to radians (`f64`; `SPN-INV-054`).
#[inline]
pub const fn radians_from_degrees_f64(degrees: f64) -> f64 {
    degrees.to_radians()
}

/// Converts an internal radian angle to degrees (`f64`; `SPN-INV-054`).
#[inline]
pub const fn degrees_from_radians_f64(radians: f64) -> f64 {
    radians.to_degrees()
}
