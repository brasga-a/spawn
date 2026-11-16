//! Foundation verification cases for the fixed-step clock
//! (docs/foundation/VERIFICATION.md: FND-T01, FND-T02).

use std::time::Duration;

use spawn_core::{ClockError, FixedStepClock, FixedStepConfig, Tick};

/// Runs `elapsed` through a fresh clock and returns every executed tick index,
/// in order, plus the total dropped time.
fn run(config: FixedStepConfig, elapsed: &[Duration]) -> Result<(Vec<u64>, Duration), ClockError> {
    let mut clock = FixedStepClock::new(config);
    let mut ticks = Vec::new();
    let mut dropped = Duration::ZERO;
    for &step in elapsed {
        let update = clock.advance(step)?;
        assert!(update.tick_count() <= config.max_ticks_per_update());
        ticks.extend(update.ticks().map(Tick::get));
        dropped += update.dropped();
    }
    Ok((ticks, dropped))
}

/// Deterministic pseudo-random frame durations in `[1, max_ms]` milliseconds
/// (64-bit LCG; no external dependency).
fn frame_durations(seed: u64, count: usize, max_ms: u64) -> Vec<Duration> {
    let mut state = seed;
    (0..count)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            Duration::from_millis(1 + (state >> 33) % max_ms)
        })
        .collect()
}

/// FND-T01 (FR-07 / INV-07): identical elapsed-duration sequences produce the
/// same number and order of fixed ticks.
#[test]
fn fnd_t01_identical_sequences_produce_identical_ticks() -> Result<(), ClockError> {
    let config = FixedStepConfig::default();
    let frames = frame_durations(0x5EED, 2_000, 40);
    let first = run(config, &frames)?;
    let second = run(config, &frames)?;
    assert_eq!(first, second);
    Ok(())
}

/// FND-T01 (FR-07 / INV-07): tick count and order depend only on total elapsed
/// time, not on how it was split into render frames (while no backlog is
/// dropped).
#[test]
fn fnd_t01_ticks_are_independent_of_render_cadence() -> Result<(), ClockError> {
    let config = FixedStepConfig::new(Duration::from_millis(10), 4)?;

    // 1 second rendered at 100 fps, 40 fps and 25 fps.
    let at_100_fps = vec![Duration::from_millis(10); 100];
    let at_40_fps = vec![Duration::from_millis(25); 40];
    let at_25_fps = vec![Duration::from_millis(40); 25];

    let expected: Vec<u64> = (0..100).collect();
    for frames in [&at_100_fps, &at_40_fps, &at_25_fps] {
        let (ticks, dropped) = run(config, frames)?;
        assert_eq!(ticks, expected);
        assert_eq!(dropped, Duration::ZERO);
    }
    Ok(())
}

/// FND-T01 (FR-07 / INV-07): for irregular frame times within the catch-up
/// budget, total ticks equal total elapsed time divided by the tick duration,
/// with consecutive indices.
#[test]
fn fnd_t01_irregular_frames_produce_exact_tick_total() -> Result<(), ClockError> {
    let config = FixedStepConfig::default();
    // Frames up to 60 ms stay within 4 ticks at 60 Hz (4 * 16.67 ms = 66.7 ms).
    let frames = frame_durations(42, 5_000, 60);
    let total: Duration = frames.iter().sum();

    let (ticks, dropped) = run(config, &frames)?;

    let expected_count = total.as_nanos() / config.tick_duration().as_nanos();
    assert_eq!(dropped, Duration::ZERO);
    assert_eq!(ticks.len() as u128, expected_count);
    assert!(ticks.iter().copied().eq(0..ticks.len() as u64));
    Ok(())
}

/// FND-T02 (FR-08 / INV-08): a large pause executes at most the configured
/// catch-up limit, drops whole ticks beyond it, and the tick index continues
/// without gaps or reset.
#[test]
fn fnd_t02_large_pause_is_bounded_and_reported() -> Result<(), ClockError> {
    let config = FixedStepConfig::new(Duration::from_millis(10), 4)?;
    let mut clock = FixedStepClock::new(config);

    // Normal frame: 1 tick.
    let update = clock.advance(Duration::from_millis(10))?;
    assert_eq!(update.tick_count(), 1);

    // 10 s debugger pause: 1000 ticks due, only 4 executed, 996 dropped.
    let update = clock.advance(Duration::from_secs(10))?;
    assert_eq!(update.tick_count(), 4);
    assert_eq!(update.first_tick(), Tick::new(1));
    assert_eq!(update.dropped(), Duration::from_millis(9_960));
    assert_eq!(clock.pending(), Duration::ZERO);

    // Next frame continues from tick 5; nothing replays the dropped backlog.
    let update = clock.advance(Duration::from_millis(10))?;
    assert_eq!(update.tick_count(), 1);
    assert_eq!(update.first_tick(), Tick::new(5));
    assert_eq!(update.dropped(), Duration::ZERO);
    Ok(())
}

/// FND-T02 (FR-08 / INV-08): the sub-tick remainder survives a dropped backlog.
#[test]
fn fnd_t02_keeps_sub_tick_remainder_after_drop() -> Result<(), ClockError> {
    let config = FixedStepConfig::new(Duration::from_millis(10), 4)?;
    let mut clock = FixedStepClock::new(config);

    let update = clock.advance(Duration::from_millis(1_007))?;
    assert_eq!(update.tick_count(), 4);
    assert_eq!(update.dropped(), Duration::from_millis(960));
    assert_eq!(clock.pending(), Duration::from_millis(7));

    let update = clock.advance(Duration::from_millis(3))?;
    assert_eq!(update.tick_count(), 1);
    Ok(())
}

/// FND-T02 (FR-08 / INV-08): extreme elapsed values never overflow, panic or
/// exceed the limit.
#[test]
fn fnd_t02_extreme_elapsed_values_are_bounded() -> Result<(), ClockError> {
    let config = FixedStepConfig::default();
    let mut clock = FixedStepClock::new(config);

    for _ in 0..3 {
        let update = clock.advance(Duration::MAX)?;
        assert_eq!(update.tick_count(), config.max_ticks_per_update());
        assert!(clock.pending() < config.tick_duration());
    }
    assert_eq!(clock.next_tick(), Tick::new(12));
    Ok(())
}
