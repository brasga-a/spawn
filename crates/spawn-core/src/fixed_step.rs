//! Fixed-step simulation clock (ADR-0003).
//!
//! The clock turns measured elapsed time into a bounded number of fixed
//! simulation ticks. It never reads the operating-system clock: the caller (the
//! root composition binary, or a test) measures monotonic elapsed time and
//! passes it in, so the clock is fully deterministic and headless-testable.
//!
//! Backlog policy (ADR-0003): when more ticks are due than the per-update
//! limit allows, the whole ticks beyond the limit are **dropped** and reported
//! in [`FixedStepUpdate::dropped`]; only the sub-tick remainder is kept.
//! Simulation then runs slower than real time for that moment instead of
//! entering an unbounded catch-up loop. `spawn-core` does not log: the caller
//! emits the diagnostic.

use std::time::Duration;

use crate::error::{ClockError, ClockErrorKind};

const NANOS_PER_SECOND: u64 = 1_000_000_000;

/// Index of a simulation tick.
///
/// Monotonically increasing, incremented exactly once per executed tick, and
/// never reset by resize, surface recovery or dropped backlog (ADR-0003). Input
/// sampling and, later, networking refer to ticks by this index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Tick(u64);

impl Tick {
    /// The first tick of a simulation.
    pub const ZERO: Self = Self(0);

    /// Creates a tick from its raw index.
    pub const fn new(index: u64) -> Self {
        Self(index)
    }

    /// The raw tick index.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Fixed-step configuration. Fixed for the lifetime of a [`FixedStepClock`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedStepConfig {
    tick_duration: Duration,
    max_ticks_per_update: u32,
}

impl FixedStepConfig {
    /// Default tick rate for tests and the foundation milestone (ADR-0003).
    pub const DEFAULT_TICK_RATE_HZ: u32 = 60;

    /// Default maximum number of ticks executed by one update (ADR-0003).
    pub const DEFAULT_MAX_TICKS_PER_UPDATE: u32 = 4;

    /// Creates a configuration from an explicit tick duration.
    ///
    /// # Errors
    ///
    /// - [`ClockErrorKind::ZeroTickDuration`] if `tick_duration` is zero.
    /// - [`ClockErrorKind::ZeroMaxTicksPerUpdate`] if `max_ticks_per_update` is zero.
    #[track_caller]
    pub fn new(tick_duration: Duration, max_ticks_per_update: u32) -> Result<Self, ClockError> {
        if tick_duration.is_zero() {
            return Err(ClockError::new(ClockErrorKind::ZeroTickDuration));
        }
        if max_ticks_per_update == 0 {
            return Err(ClockError::new(ClockErrorKind::ZeroMaxTicksPerUpdate));
        }
        Ok(Self {
            tick_duration,
            max_ticks_per_update,
        })
    }

    /// Creates a configuration from a tick rate in ticks per second.
    ///
    /// The tick duration is `1 s / tick_rate_hz`, truncated to whole
    /// nanoseconds (60 Hz gives 16 666 666 ns), so the delivered rate can be
    /// marginally higher than requested.
    ///
    /// # Errors
    ///
    /// - [`ClockErrorKind::ZeroTickRate`] if `tick_rate_hz` is zero.
    /// - [`ClockErrorKind::ZeroTickDuration`] if `tick_rate_hz > 1_000_000_000`
    ///   (truncates to zero nanoseconds).
    /// - [`ClockErrorKind::ZeroMaxTicksPerUpdate`] if `max_ticks_per_update` is zero.
    #[track_caller]
    pub fn from_tick_rate(
        tick_rate_hz: u32,
        max_ticks_per_update: u32,
    ) -> Result<Self, ClockError> {
        if tick_rate_hz == 0 {
            return Err(ClockError::new(ClockErrorKind::ZeroTickRate));
        }
        let nanos = NANOS_PER_SECOND / u64::from(tick_rate_hz);
        Self::new(Duration::from_nanos(nanos), max_ticks_per_update)
    }

    /// Duration of one simulation tick.
    pub const fn tick_duration(&self) -> Duration {
        self.tick_duration
    }

    /// Maximum number of ticks a single [`FixedStepClock::advance`] executes.
    pub const fn max_ticks_per_update(&self) -> u32 {
        self.max_ticks_per_update
    }
}

impl Default for FixedStepConfig {
    /// 60 Hz with at most 4 ticks per update (ADR-0003 test defaults).
    fn default() -> Self {
        Self {
            tick_duration: Duration::from_nanos(
                NANOS_PER_SECOND / Self::DEFAULT_TICK_RATE_HZ as u64,
            ),
            max_ticks_per_update: Self::DEFAULT_MAX_TICKS_PER_UPDATE,
        }
    }
}

/// Result of one [`FixedStepClock::advance`]: which ticks to execute and how
/// much backlog was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedStepUpdate {
    first_tick: Tick,
    tick_count: u32,
    dropped: Duration,
}

impl FixedStepUpdate {
    /// Number of ticks to execute now; never more than the configured limit.
    pub const fn tick_count(&self) -> u32 {
        self.tick_count
    }

    /// Index of the first tick to execute (meaningful when `tick_count > 0`).
    pub const fn first_tick(&self) -> Tick {
        self.first_tick
    }

    /// The ticks to execute, in order.
    pub fn ticks(&self) -> Ticks {
        Ticks {
            next: self.first_tick.0,
            remaining: self.tick_count,
        }
    }

    /// Simulation time discarded by the backlog policy in this update; zero
    /// unless more ticks were due than the limit allows. A non-zero value
    /// should be reported as a diagnostic by the caller.
    pub const fn dropped(&self) -> Duration {
        self.dropped
    }
}

/// Iterator over the ticks of a [`FixedStepUpdate`], in increasing order.
#[derive(Debug, Clone)]
pub struct Ticks {
    next: u64,
    remaining: u32,
}

impl Iterator for Ticks {
    type Item = Tick;

    fn next(&mut self) -> Option<Tick> {
        if self.remaining == 0 {
            return None;
        }
        let tick = Tick(self.next);
        self.remaining -= 1;
        // `advance` guarantees `first_tick + tick_count` fits in `u64`; the
        // wrapping add only affects the value after the last returned tick.
        self.next = self.next.wrapping_add(1);
        Some(tick)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.remaining as usize;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for Ticks {}

/// Fixed-step simulation clock.
///
/// ```
/// use std::time::Duration;
/// use spawn_core::{FixedStepClock, FixedStepConfig};
///
/// # fn main() -> Result<(), spawn_core::ClockError> {
/// let mut clock = FixedStepClock::new(FixedStepConfig::default());
///
/// // The caller measures monotonic elapsed time since the previous wakeup.
/// let update = clock.advance(Duration::from_millis(40))?;
/// for tick in update.ticks() {
///     // run simulation tick `tick`
///     let _ = tick;
/// }
/// assert_eq!(update.tick_count(), 2); // 40 ms at 60 Hz
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct FixedStepClock {
    config: FixedStepConfig,
    accumulator: Duration,
    next_tick: Tick,
}

impl FixedStepClock {
    /// Creates a clock starting at [`Tick::ZERO`] with an empty accumulator.
    pub fn new(config: FixedStepConfig) -> Self {
        Self::starting_at(config, Tick::ZERO)
    }

    /// Creates a clock whose next executed tick is `next_tick`.
    pub fn starting_at(config: FixedStepConfig, next_tick: Tick) -> Self {
        Self {
            config,
            accumulator: Duration::ZERO,
            next_tick,
        }
    }

    /// The configuration of this clock.
    pub const fn config(&self) -> &FixedStepConfig {
        &self.config
    }

    /// Index of the next tick that will be executed.
    pub const fn next_tick(&self) -> Tick {
        self.next_tick
    }

    /// Accumulated time not yet consumed by a tick; always less than one tick
    /// duration after a successful [`advance`](Self::advance).
    pub const fn pending(&self) -> Duration {
        self.accumulator
    }

    /// Adds `elapsed` monotonic time and returns the ticks to execute now.
    ///
    /// At most [`FixedStepConfig::max_ticks_per_update`] ticks are returned.
    /// Whole ticks beyond that limit are dropped and reported in
    /// [`FixedStepUpdate::dropped`]; the sub-tick remainder is kept for the
    /// next call. Arbitrarily large `elapsed` values (including
    /// [`Duration::MAX`]) are handled without overflow.
    ///
    /// # Errors
    ///
    /// [`ClockErrorKind::TickIndexExhausted`] if executing the due ticks would
    /// overflow the `u64` tick index (the index after the last executed tick
    /// must fit in `u64`, so the last executable tick is `u64::MAX - 1`). The
    /// clock is left unchanged and `elapsed` is not accumulated; the index
    /// never wraps.
    #[track_caller]
    pub fn advance(&mut self, elapsed: Duration) -> Result<FixedStepUpdate, ClockError> {
        let tick_nanos = self.config.tick_duration.as_nanos();
        let total_nanos = self
            .accumulator
            .as_nanos()
            .saturating_add(elapsed.as_nanos());

        let due = total_nanos / tick_nanos;
        let remainder = total_nanos % tick_nanos;
        let max = u128::from(self.config.max_ticks_per_update);
        let executed = due.min(max);
        let dropped_ticks = due - executed;

        // `executed <= max_ticks_per_update`, which is a `u32`.
        let tick_count = u32::try_from(executed).unwrap_or(self.config.max_ticks_per_update);
        let after_last = self
            .next_tick
            .0
            .checked_add(u64::from(tick_count))
            .ok_or_else(|| ClockError::new(ClockErrorKind::TickIndexExhausted))?;

        let update = FixedStepUpdate {
            first_tick: self.next_tick,
            tick_count,
            dropped: nanos_to_duration(dropped_ticks.saturating_mul(tick_nanos)),
        };
        self.next_tick = Tick(after_last);
        self.accumulator = nanos_to_duration(remainder);
        Ok(update)
    }
}

/// Converts nanoseconds to a `Duration`, saturating at `Duration::MAX`.
fn nanos_to_duration(nanos: u128) -> Duration {
    let nanos_per_second = u128::from(NANOS_PER_SECOND);
    let seconds = nanos / nanos_per_second;
    let subsec = (nanos % nanos_per_second) as u32;
    match u64::try_from(seconds) {
        Ok(seconds) => Duration::new(seconds, subsec),
        Err(_) => Duration::MAX,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn default_config_is_60_hz_with_four_catch_up_ticks() {
        let config = FixedStepConfig::default();
        assert_eq!(config.tick_duration(), Duration::from_nanos(16_666_666));
        assert_eq!(config.max_ticks_per_update(), 4);
        assert_eq!(FixedStepConfig::from_tick_rate(60, 4).unwrap(), config);
    }

    #[test]
    fn rejects_zero_tick_duration() {
        let error = FixedStepConfig::new(Duration::ZERO, 4).unwrap_err();
        assert_eq!(error.kind(), ClockErrorKind::ZeroTickDuration);
        assert_eq!(error.code(), "CORE-0001");
    }

    #[test]
    fn rejects_zero_tick_rate() {
        let error = FixedStepConfig::from_tick_rate(0, 4).unwrap_err();
        assert_eq!(error.kind(), ClockErrorKind::ZeroTickRate);
        assert_eq!(error.code(), "CORE-0002");
    }

    #[test]
    fn rejects_sub_nanosecond_tick_rate() {
        let one_ghz = FixedStepConfig::from_tick_rate(1_000_000_000, 4).unwrap();
        assert_eq!(one_ghz.tick_duration(), Duration::from_nanos(1));

        let line = line!() + 1;
        let error = FixedStepConfig::from_tick_rate(1_000_000_001, 4).unwrap_err();
        assert_eq!(error.kind(), ClockErrorKind::ZeroTickDuration);
        assert_eq!(error.code(), "CORE-0001");
        assert_eq!(error.location().line(), line);

        let max_error = FixedStepConfig::from_tick_rate(u32::MAX, 4).unwrap_err();
        assert_eq!(max_error.kind(), ClockErrorKind::ZeroTickDuration);
        assert_eq!(max_error.code(), "CORE-0001");
    }

    #[test]
    fn rejects_zero_max_ticks_per_update() {
        let error = FixedStepConfig::new(ms(10), 0).unwrap_err();
        assert_eq!(error.kind(), ClockErrorKind::ZeroMaxTicksPerUpdate);
        assert_eq!(error.code(), "CORE-0003");
    }

    #[test]
    fn less_than_one_tick_accumulates_without_ticking() {
        let mut clock = FixedStepClock::new(FixedStepConfig::new(ms(10), 4).unwrap());
        let update = clock.advance(ms(4)).unwrap();
        assert_eq!(update.tick_count(), 0);
        assert_eq!(clock.pending(), ms(4));
        let update = clock.advance(ms(6)).unwrap();
        assert_eq!(update.tick_count(), 1);
        assert_eq!(update.first_tick(), Tick::ZERO);
        assert_eq!(clock.pending(), Duration::ZERO);
        assert_eq!(clock.next_tick(), Tick::new(1));
    }

    #[test]
    fn zero_elapsed_produces_no_ticks_and_changes_nothing() {
        let mut clock = FixedStepClock::new(FixedStepConfig::default());
        for _ in 0..10 {
            let update = clock.advance(Duration::ZERO).unwrap();
            assert_eq!(update.tick_count(), 0);
            assert_eq!(update.ticks().count(), 0);
            assert_eq!(update.dropped(), Duration::ZERO);
        }
        assert_eq!(clock.next_tick(), Tick::ZERO);
        assert_eq!(clock.pending(), Duration::ZERO);
    }

    #[test]
    fn exactly_one_tick_duration_executes_one_tick_with_no_remainder() {
        let config = FixedStepConfig::default();
        let mut clock = FixedStepClock::new(config);
        let update = clock.advance(config.tick_duration()).unwrap();
        assert_eq!(update.tick_count(), 1);
        assert_eq!(update.first_tick(), Tick::ZERO);
        assert_eq!(clock.pending(), Duration::ZERO);

        // One nanosecond short of a tick does not execute it.
        let update = clock
            .advance(config.tick_duration() - Duration::from_nanos(1))
            .unwrap();
        assert_eq!(update.tick_count(), 0);
        // The missing nanosecond completes it.
        let update = clock.advance(Duration::from_nanos(1)).unwrap();
        assert_eq!(update.tick_count(), 1);
        assert_eq!(update.first_tick(), Tick::new(1));
    }

    #[test]
    fn partial_durations_accumulate_exactly() {
        let mut clock = FixedStepClock::new(FixedStepConfig::new(ms(10), 4).unwrap());
        let mut executed = 0;
        // 3 ms frames: ticks become due at 12, 21, 30 ms (cumulative 10, 20, 30).
        for _ in 0..10 {
            executed += clock.advance(ms(3)).unwrap().tick_count();
        }
        assert_eq!(executed, 3);
        assert_eq!(clock.pending(), Duration::ZERO);
        assert_eq!(clock.next_tick(), Tick::new(3));
    }

    #[test]
    fn multiple_ticks_in_one_advance_keep_remainder() {
        let mut clock = FixedStepClock::new(FixedStepConfig::new(ms(10), 4).unwrap());
        let update = clock.advance(ms(37)).unwrap();
        assert_eq!(update.tick_count(), 3);
        assert_eq!(update.dropped(), Duration::ZERO);
        assert_eq!(clock.pending(), ms(7));
    }

    #[test]
    fn last_executable_tick_is_u64_max_minus_one() {
        let config = FixedStepConfig::new(ms(10), 4).unwrap();
        let mut clock = FixedStepClock::starting_at(config, Tick::new(u64::MAX - 1));
        let update = clock.advance(ms(10)).unwrap();
        assert_eq!(update.first_tick(), Tick::new(u64::MAX - 1));
        assert_eq!(clock.next_tick(), Tick::new(u64::MAX));
        // Time below one tick is still accepted at the end of the range.
        assert_eq!(clock.advance(ms(5)).unwrap().tick_count(), 0);
        // Any further tick would need index u64::MAX + 1.
        let error = clock.advance(ms(5)).unwrap_err();
        assert_eq!(error.kind(), ClockErrorKind::TickIndexExhausted);
        assert_eq!(clock.pending(), ms(5));
    }

    #[test]
    fn ticks_iterator_yields_consecutive_indices() {
        let mut clock =
            FixedStepClock::starting_at(FixedStepConfig::new(ms(10), 4).unwrap(), Tick::new(41));
        let update = clock.advance(ms(30)).unwrap();
        let ticks: Vec<u64> = update.ticks().map(Tick::get).collect();
        assert_eq!(ticks, vec![41, 42, 43]);
        assert_eq!(update.ticks().len(), 3);
    }

    #[test]
    fn exhausted_tick_index_is_an_error_and_leaves_clock_unchanged() {
        let config = FixedStepConfig::new(ms(10), 4).unwrap();
        let mut clock = FixedStepClock::starting_at(config, Tick::new(u64::MAX - 1));
        let error = clock.advance(ms(30)).unwrap_err();
        assert_eq!(error.kind(), ClockErrorKind::TickIndexExhausted);
        assert_eq!(error.code(), "CORE-0004");
        assert_eq!(clock.next_tick(), Tick::new(u64::MAX - 1));
        assert_eq!(clock.pending(), Duration::ZERO);
    }

    #[test]
    fn error_report_is_compiler_like() {
        let error = FixedStepConfig::new(Duration::ZERO, 4).unwrap_err();
        let report = error.to_string();
        let mut lines = report.lines();
        assert_eq!(
            lines.next(),
            Some("error[CORE-0001]: fixed-step tick duration must be greater than zero")
        );
        let location = lines.next().unwrap();
        assert!(location.starts_with("  --> "), "{location}");
        assert!(location.contains("fixed_step.rs:"), "{location}");
        assert_eq!(error.location().file(), file!());
    }

    #[test]
    fn nanos_to_duration_saturates() {
        assert_eq!(nanos_to_duration(u128::MAX), Duration::MAX);
        assert_eq!(
            nanos_to_duration(1_500_000_000),
            Duration::new(1, 500_000_000)
        );
    }
}
