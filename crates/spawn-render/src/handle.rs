//! Generational resource handles and slot map (`SPN-INV-035`).
//!
//! Always compiled (`default-features = false`), `#![forbid(unsafe_code)]`, and
//! `#![deny(clippy::indexing_slicing)]`. Stale, destroyed, out-of-bounds, or
//! cross-renderer handles produce a located [`RenderError`] (`REND-0013`),
//! never a panic or undefined behavior.

#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]

use std::marker::PhantomData;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::error::{RenderError, RenderErrorKind};

static NEXT_POOL_OWNER_ID: AtomicU32 = AtomicU32::new(1);

fn next_pool_owner_id() -> u32 {
    loop {
        let current = NEXT_POOL_OWNER_ID.fetch_add(1, Ordering::Relaxed);
        if current != 0 {
            return current;
        }
    }
}

/// Raw generational coordinates shared by typed `spawn-render` handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct RawHandle {
    pub index: u32,
    pub generation: u32,
    pub owner: u32,
}

macro_rules! define_render_handle {
    ($(#[$meta:meta])* $name:ident, $kind_label:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name {
            raw: RawHandle,
        }

        impl $name {
            pub(crate) const KIND_LABEL: &'static str = $kind_label;

            pub(crate) const fn from_pool(raw: RawHandle) -> Self {
                Self { raw }
            }

            pub(crate) const fn raw(self) -> RawHandle {
                self.raw
            }

            /// Constructs an unowned handle from raw `(index, generation)` parts
            /// (for serialization round-trips or negative handle-validation tests).
            ///
            /// Because its pool owner tag is `0`, passing a forged handle to any
            /// live `Renderer` deterministically returns `REND-0013`.
            pub const fn from_raw(index: u32, generation: u32) -> Self {
                Self {
                    raw: RawHandle {
                        index,
                        generation,
                        owner: 0,
                    },
                }
            }

            /// Zero-based slot index within its owning resource pool.
            pub const fn index(self) -> u32 {
                self.raw.index
            }

            /// Non-zero generation counter assigned when the resource was created.
            pub const fn generation(self) -> u32 {
                self.raw.generation
            }
        }
    };
}

define_render_handle!(
    /// Generational handle referencing a validated shader loaded into a `Renderer`.
    ShaderHandle,
    "ShaderHandle"
);

define_render_handle!(
    /// Generational handle referencing a graphics pipeline compiled by a `Renderer`.
    PipelineHandle,
    "PipelineHandle"
);

define_render_handle!(
    /// Generational handle referencing an offscreen color + depth render target.
    OffscreenTargetHandle,
    "OffscreenTargetHandle"
);

define_render_handle!(
    /// Generational handle referencing an uploaded indexed 3D mesh (`DEVICE_LOCAL` vertex + index buffers).
    MeshHandle,
    "MeshHandle"
);

pub(crate) trait HandleType: Copy {
    const KIND: &'static str;
    fn from_raw_pool(raw: RawHandle) -> Self;
    fn into_raw_pool(self) -> RawHandle;
}

impl HandleType for ShaderHandle {
    const KIND: &'static str = Self::KIND_LABEL;
    fn from_raw_pool(raw: RawHandle) -> Self {
        Self::from_pool(raw)
    }
    fn into_raw_pool(self) -> RawHandle {
        self.raw()
    }
}

impl HandleType for PipelineHandle {
    const KIND: &'static str = Self::KIND_LABEL;
    fn from_raw_pool(raw: RawHandle) -> Self {
        Self::from_pool(raw)
    }
    fn into_raw_pool(self) -> RawHandle {
        self.raw()
    }
}

impl HandleType for OffscreenTargetHandle {
    const KIND: &'static str = Self::KIND_LABEL;
    fn from_raw_pool(raw: RawHandle) -> Self {
        Self::from_pool(raw)
    }
    fn into_raw_pool(self) -> RawHandle {
        self.raw()
    }
}

impl HandleType for MeshHandle {
    const KIND: &'static str = Self::KIND_LABEL;
    fn from_raw_pool(raw: RawHandle) -> Self {
        Self::from_pool(raw)
    }
    fn into_raw_pool(self) -> RawHandle {
        self.raw()
    }
}

#[derive(Debug)]
struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

/// Generational slot pool mapping typed handles `H` to owned resources `T`.
#[derive(Debug)]
#[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
pub(crate) struct GenerationalPool<H, T> {
    owner: u32,
    slots: Vec<Slot<T>>,
    free_indices: Vec<u32>,
    live_count: usize,
    _marker: PhantomData<fn() -> H>,
}

#[cfg_attr(not(feature = "vulkan"), allow(dead_code))]
impl<H: HandleType, T> GenerationalPool<H, T> {
    /// Creates an empty generational resource pool with a unique owner tag.
    pub(crate) fn new() -> Self {
        Self {
            owner: next_pool_owner_id(),
            slots: Vec::new(),
            free_indices: Vec::new(),
            live_count: 0,
            _marker: PhantomData,
        }
    }

    /// Number of currently live resources in the pool.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.live_count
    }

    /// Whether no resources are currently live in the pool.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.live_count == 0
    }

    /// Inserts `value` and returns its unique generational handle.
    #[track_caller]
    pub(crate) fn insert(&mut self, value: T) -> Result<H, RenderError> {
        if let Some(index) = self.free_indices.pop() {
            let idx = usize::try_from(index).map_err(|_| {
                RenderError::new(RenderErrorKind::InvalidResourceHandle).with_detail(format!(
                    "{} free slot index {index} overflows usize",
                    H::KIND
                ))
            })?;
            let slot = self.slots.get_mut(idx).ok_or_else(|| {
                RenderError::new(RenderErrorKind::InvalidResourceHandle)
                    .with_detail(format!("{} free slot {index} out of bounds", H::KIND))
            })?;
            slot.value = Some(value);
            self.live_count = self.live_count.saturating_add(1);
            return Ok(H::from_raw_pool(RawHandle {
                index,
                generation: slot.generation,
                owner: self.owner,
            }));
        }

        let index = u32::try_from(self.slots.len()).map_err(|_| {
            RenderError::new(RenderErrorKind::GpuMemoryExhausted).with_detail(format!(
                "{} pool capacity exhausted (u32::MAX slots)",
                H::KIND
            ))
        })?;
        let generation = 1;
        self.slots.push(Slot {
            generation,
            value: Some(value),
        });
        self.live_count = self.live_count.saturating_add(1);
        Ok(H::from_raw_pool(RawHandle {
            index,
            generation,
            owner: self.owner,
        }))
    }

    /// Borrows the live resource referenced by `handle`, or returns `REND-0013`.
    #[track_caller]
    pub(crate) fn get(&self, handle: H) -> Result<&T, RenderError> {
        let raw = handle.into_raw_pool();
        self.validate_owner_and_gen(raw)?;
        let idx =
            usize::try_from(raw.index).map_err(|_| invalid_handle::<H>(raw, "index overflow"))?;
        let slot = self
            .slots
            .get(idx)
            .ok_or_else(|| invalid_handle::<H>(raw, "index out of bounds"))?;
        if slot.generation != raw.generation {
            return Err(invalid_handle::<H>(raw, "stale generation"));
        }
        slot.value
            .as_ref()
            .ok_or_else(|| invalid_handle::<H>(raw, "slot already destroyed"))
    }

    /// Mutably borrows the live resource referenced by `handle`, or returns `REND-0013`.
    #[track_caller]
    pub(crate) fn get_mut(&mut self, handle: H) -> Result<&mut T, RenderError> {
        let raw = handle.into_raw_pool();
        self.validate_owner_and_gen(raw)?;
        let idx =
            usize::try_from(raw.index).map_err(|_| invalid_handle::<H>(raw, "index overflow"))?;
        let slot = self
            .slots
            .get_mut(idx)
            .ok_or_else(|| invalid_handle::<H>(raw, "index out of bounds"))?;
        if slot.generation != raw.generation {
            return Err(invalid_handle::<H>(raw, "stale generation"));
        }
        slot.value
            .as_mut()
            .ok_or_else(|| invalid_handle::<H>(raw, "slot already destroyed"))
    }

    /// Removes and returns the live resource referenced by `handle`, advancing
    /// the slot's generation so any copied handles immediately become stale.
    #[track_caller]
    pub(crate) fn remove(&mut self, handle: H) -> Result<T, RenderError> {
        let raw = handle.into_raw_pool();
        self.validate_owner_and_gen(raw)?;
        let idx =
            usize::try_from(raw.index).map_err(|_| invalid_handle::<H>(raw, "index overflow"))?;
        let slot = self
            .slots
            .get_mut(idx)
            .ok_or_else(|| invalid_handle::<H>(raw, "index out of bounds"))?;
        if slot.generation != raw.generation {
            return Err(invalid_handle::<H>(raw, "stale generation"));
        }
        let value = slot
            .value
            .take()
            .ok_or_else(|| invalid_handle::<H>(raw, "slot already destroyed"))?;
        self.live_count = self.live_count.saturating_sub(1);
        // Retire the slot permanently if generation reaches `u32::MAX` so
        // generations never wrap around to an earlier valid value.
        if let Some(next_gen) = slot.generation.checked_add(1) {
            slot.generation = next_gen;
            self.free_indices.push(raw.index);
        }
        Ok(value)
    }

    /// Drains all remaining live resources in reverse slot order for teardown,
    /// invalidating any outstanding handles.
    pub(crate) fn drain_all(&mut self) -> Vec<T> {
        let mut out = Vec::with_capacity(self.live_count);
        for (idx, slot) in self.slots.iter_mut().enumerate().rev() {
            if let Some(value) = slot.value.take() {
                out.push(value);
                if let Some(next_gen) = slot.generation.checked_add(1) {
                    slot.generation = next_gen;
                    if let Ok(index) = u32::try_from(idx) {
                        self.free_indices.push(index);
                    }
                }
            }
        }
        self.live_count = 0;
        out
    }

    #[track_caller]
    fn validate_owner_and_gen(&self, raw: RawHandle) -> Result<(), RenderError> {
        if raw.generation == 0 {
            return Err(invalid_handle::<H>(raw, "zero generation"));
        }
        if raw.owner != self.owner {
            return Err(invalid_handle::<H>(raw, "forged or foreign pool owner"));
        }
        Ok(())
    }
}

#[track_caller]
fn invalid_handle<H: HandleType>(raw: RawHandle, reason: &str) -> RenderError {
    RenderError::new(RenderErrorKind::InvalidResourceHandle).with_detail(format!(
        "{}(index={}, generation={}): {reason}",
        H::KIND,
        raw.index,
        raw.generation
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normalize_location(report: &str) -> String {
        report
            .lines()
            .map(|line| {
                if let Some(rest) = line.strip_prefix("  --> ") {
                    let file_only = rest
                        .rsplit_once(':')
                        .and_then(|(prefix, _col)| prefix.rsplit_once(':'))
                        .map_or(rest, |(file, _line)| file)
                        .replace('\\', "/");
                    format!("  --> {file_only}:<LINE>:<COL>")
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn generational_pool_rejects_stale_forged_zero_and_foreign_handles() {
        let mut pool_a: GenerationalPool<ShaderHandle, &'static str> = GenerationalPool::new();
        let mut pool_b: GenerationalPool<ShaderHandle, &'static str> = GenerationalPool::new();
        assert!(pool_a.is_empty());

        let h1 = pool_a.insert("vs_first").unwrap();
        assert_eq!(pool_a.len(), 1);
        assert_eq!(h1.index(), 0);
        assert_eq!(h1.generation(), 1);
        assert_eq!(*pool_a.get(h1).unwrap(), "vs_first");

        // Foreign pool with same slot/generation must reject `h1`.
        let _hb = pool_b.insert("other").unwrap();
        let foreign_err = pool_b.get(h1).unwrap_err();
        assert_eq!(foreign_err.kind(), RenderErrorKind::InvalidResourceHandle);
        assert_eq!(foreign_err.code(), "REND-0013");

        // Forged handles (`owner == 0` or `generation == 0` or out-of-bounds) fail cleanly.
        for bad in [
            ShaderHandle::from_raw(0, 0),
            ShaderHandle::from_raw(0, 1),
            ShaderHandle::from_raw(99, 1),
        ] {
            assert_eq!(
                pool_a.get(bad).unwrap_err().kind(),
                RenderErrorKind::InvalidResourceHandle
            );
        }

        // Removing `h1` invalidates `h1` and double-remove fails; re-inserting reuses slot 0 with gen 2.
        assert_eq!(pool_a.remove(h1).unwrap(), "vs_first");
        let stale_err = pool_a.get(h1).unwrap_err();
        assert_eq!(stale_err.kind(), RenderErrorKind::InvalidResourceHandle);
        assert_eq!(
            pool_a.remove(h1).unwrap_err().kind(),
            RenderErrorKind::InvalidResourceHandle
        );

        let h2 = pool_a.insert("vs_second").unwrap();
        assert_eq!(h2.index(), 0);
        assert_eq!(h2.generation(), 2);
        assert_eq!(*pool_a.get(h2).unwrap(), "vs_second");
        let stale_after_reuse = pool_a.get(h1).unwrap_err();
        insta::assert_snapshot!(
            normalize_location(&stale_after_reuse.to_string()),
            @r#"
error[REND-0013]: invalid or stale render resource handle
  --> crates/spawn-render/src/handle.rs:<LINE>:<COL>
   = detail: ShaderHandle(index=0, generation=1): stale generation
"#
        );
    }
}
