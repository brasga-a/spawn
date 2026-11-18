//! Foundation verification cases for `spawn-input`
//! (`docs/foundation/VERIFICATION.md`: FND-T03, FND-T04, plus deterministic
//! replay, rebinding, context switching, and extreme-input resilience).

use std::error::Error;
use std::time::Duration;

use spawn_core::{ClockError, FixedStepClock, FixedStepConfig, Tick};
use spawn_input::{
    ActionId, AxisConfig, AxisId, AxisSource, ButtonState, ButtonTrigger, ContextId, DeviceKind,
    InputCommand, InputError, InputMap, InputState, MotionAxis, PointerDelta, ScrollVector,
};
use spawn_platform::{
    ElementState, Event, KeyCode, KeyEvent, LogicalSize, MouseButton, PhysicalPosition,
    PhysicalSize, ScrollDelta, WindowDescriptor, WindowEvent, WindowId,
};

fn key_event(key: KeyCode, native_code: u32, state: ElementState) -> WindowEvent {
    WindowEvent::Key(KeyEvent {
        key,
        native_code,
        state,
    })
}

fn sample_due_ticks(
    clock: &mut FixedStepClock,
    input: &mut InputState,
    elapsed: Duration,
) -> Result<Vec<InputCommand>, ClockError> {
    let update = clock.advance(elapsed)?;
    Ok(update.ticks().map(|tick| input.sample_tick(tick)).collect())
}

/// FND-T03 (FR-09 / INV-10): when render frames run faster than the fixed
/// simulation rate (0 ticks on intermediate frames), press edges, pointer
/// motion, and scroll deltas accumulate across zero-tick frames and are
/// observed intact on the next simulation tick.
#[test]
fn fnd_t03_zero_tick_frames_preserve_press_and_motion_until_next_tick() -> Result<(), Box<dyn Error>>
{
    let confirm = ActionId::new(0)?;
    let pan_x = AxisId::new(0)?;
    let zoom = AxisId::new(1)?;

    let mut map = InputMap::new();
    map.bind_action(confirm, ButtonTrigger::Key(KeyCode::Enter))?;
    map.bind_axis(pan_x, AxisSource::PointerMotion(MotionAxis::X))?;
    map.set_axis_config(pan_x, AxisConfig::new().with_sensitivity(0.5)?);
    map.bind_axis(zoom, AxisSource::ScrollLines(MotionAxis::Y))?;

    let mut input = InputState::with_map(map);
    let mut clock = FixedStepClock::new(FixedStepConfig::new(Duration::from_millis(16), 4)?);

    // Frame 0 (4 ms -> 0 ticks): key press + initial pointer entry + move (+6).
    input.handle_window_event(&key_event(KeyCode::Enter, 28, ElementState::Pressed));
    input.handle_window_event(&WindowEvent::PointerEntered(PhysicalPosition::new(
        100.0, 100.0,
    )));
    input.handle_window_event(&WindowEvent::PointerMoved(PhysicalPosition::new(
        106.0, 100.0,
    )));
    assert!(sample_due_ticks(&mut clock, &mut input, Duration::from_millis(4))?.is_empty());

    // Frame 1 (5 ms -> cumulative 9 ms -> still 0 ticks): more pointer motion (+4) + scroll (-2).
    input.handle_window_event(&WindowEvent::PointerMoved(PhysicalPosition::new(
        110.0, 95.0,
    )));
    input.handle_window_event(&WindowEvent::Scroll(ScrollDelta::Lines { x: 0.0, y: -2.0 }));
    assert!(sample_due_ticks(&mut clock, &mut input, Duration::from_millis(5))?.is_empty());

    // Frame 2 (7 ms -> cumulative 16 ms -> Tick(0) runs!): consumes accumulated edge + deltas.
    let commands = sample_due_ticks(&mut clock, &mut input, Duration::from_millis(7))?;
    assert_eq!(commands.len(), 1);
    let cmd0 = commands[0];
    assert_eq!(cmd0.tick(), Tick::ZERO);
    assert_eq!(
        cmd0.action_state(confirm),
        ButtonState {
            pressed: true,
            held: true,
            released: false,
        }
    );
    assert_eq!(cmd0.axis(pan_x), 5.0); // (6 + 4) * 0.5
    assert_eq!(cmd0.axis(zoom), -2.0);
    assert_eq!(
        input.devices().pointer_delta(),
        PointerDelta::new(10.0, -5.0)
    );
    assert_eq!(input.devices().scroll_lines(), ScrollVector::new(0.0, -2.0));

    // Frame 3 (16 ms -> Tick(1)): still held, no duplicate press or delta.
    let commands = sample_due_ticks(&mut clock, &mut input, Duration::from_millis(16))?;
    assert_eq!(commands.len(), 1);
    let cmd1 = commands[0];
    assert_eq!(cmd1.tick(), Tick::new(1));
    assert_eq!(
        cmd1.action_state(confirm),
        ButtonState {
            pressed: false,
            held: true,
            released: false,
        }
    );
    assert_eq!(cmd1.axis(pan_x), 0.0);
    assert_eq!(cmd1.axis(zoom), 0.0);

    Ok(())
}

/// FND-T03 (FR-09 / INV-10): when multiple catch-up ticks run in one wakeup
/// (including when backlog is dropped), press/release edges and transient
/// pointer/scroll deltas appear on the first tick of the wakeup only and are
/// never duplicated across subsequent catch-up ticks, while held actions and
/// held digital axes remain active on every catch-up tick.
#[test]
fn fnd_t03_multi_tick_catch_up_never_duplicates_edges_or_transient_deltas()
-> Result<(), Box<dyn Error>> {
    let primary = ActionId::new(0)?;
    let horizontal = AxisId::new(0)?;
    let pan_x = AxisId::new(1)?;

    let mut map = InputMap::new();
    map.bind_action(primary, ButtonTrigger::MouseButton(MouseButton::Left))?;
    map.bind_axis_digital_pair(
        horizontal,
        ButtonTrigger::Key(KeyCode::A),
        ButtonTrigger::Key(KeyCode::D),
    )?;
    map.set_axis_config(horizontal, AxisConfig::unit());
    map.bind_axis(pan_x, AxisSource::PointerRelative(MotionAxis::X))?;

    let mut input = InputState::with_map(map);
    let mut clock = FixedStepClock::new(FixedStepConfig::new(Duration::from_millis(10), 4)?);

    // Press Left button, hold D (+1), and move relative pointer by +24, then stall 200 ms (capped at 4 ticks).
    input.handle_window_event(&WindowEvent::PointerButton {
        button: MouseButton::Left,
        state: ElementState::Pressed,
    });
    input.handle_window_event(&key_event(KeyCode::D, 32, ElementState::Pressed));
    input.handle_relative_motion(24.0, -8.0);

    let catch_up = sample_due_ticks(&mut clock, &mut input, Duration::from_millis(200))?;
    assert_eq!(catch_up.len(), 4);

    // Tick 0: sees press edge AND transient relative motion delta AND held digital axis.
    assert_eq!(
        catch_up[0].action_state(primary),
        ButtonState {
            pressed: true,
            held: true,
            released: false,
        }
    );
    assert_eq!(catch_up[0].axis(horizontal), 1.0);
    assert_eq!(catch_up[0].axis(pan_x), 24.0);

    // Ticks 1..=3: held stays true and digital axis stays 1.0, while press edge
    // and relative motion delta are NOT duplicated.
    for cmd in &catch_up[1..] {
        assert_eq!(
            cmd.action_state(primary),
            ButtonState {
                pressed: false,
                held: true,
                released: false,
            }
        );
        assert_eq!(cmd.axis(horizontal), 1.0);
        assert_eq!(cmd.axis(pan_x), 0.0);
    }

    // Release Left button and D before a 3-tick catch-up frame (30 ms).
    input.handle_window_event(&WindowEvent::PointerButton {
        button: MouseButton::Left,
        state: ElementState::Released,
    });
    input.handle_window_event(&key_event(KeyCode::D, 32, ElementState::Released));

    let release_ticks = sample_due_ticks(&mut clock, &mut input, Duration::from_millis(30))?;
    assert_eq!(release_ticks.len(), 3);
    assert_eq!(
        release_ticks[0].action_state(primary),
        ButtonState {
            pressed: false,
            held: false,
            released: true,
        }
    );
    assert_eq!(release_ticks[0].axis(horizontal), 0.0);
    for cmd in &release_ticks[1..] {
        assert_eq!(cmd.action_state(primary), ButtonState::UP);
        assert_eq!(cmd.axis(horizontal), 0.0);
    }

    Ok(())
}

/// FND-T03 (FR-09 / INV-10): a rapid press+release completed before the next
/// tick runs preserves both `pressed` and `released` edges on that tick and
/// ends unheld for subsequent ticks.
#[test]
fn fnd_t03_rapid_press_and_release_within_one_tick_preserves_both_edges() -> Result<(), InputError>
{
    let activate = ActionId::new(0)?;
    let mut map = InputMap::new();
    map.bind_action(activate, ButtonTrigger::Key(KeyCode::Space))?;

    let mut input = InputState::with_map(map);

    // Press in 0-tick frame A, release in frame B before Tick(0) samples.
    input.handle_window_event(&key_event(KeyCode::Space, 57, ElementState::Pressed));
    input.handle_window_event(&key_event(KeyCode::Space, 57, ElementState::Released));

    let cmd0 = input.sample_tick(Tick::ZERO);
    assert_eq!(
        cmd0.action_state(activate),
        ButtonState {
            pressed: true,
            held: false,
            released: true,
        }
    );
    assert!(cmd0.was_active(activate));
    assert_eq!(
        input.devices().key_state(KeyCode::Space),
        ButtonState {
            pressed: true,
            held: false,
            released: true,
        }
    );

    let cmd1 = input.sample_tick(Tick::new(1));
    assert_eq!(cmd1.action_state(activate), ButtonState::UP);
    assert!(!cmd1.was_active(activate));
    assert_eq!(input.devices().key_state(KeyCode::Space), ButtonState::UP);

    Ok(())
}

/// FND-T03 (FR-09 / INV-10): while an action is already held, a rapid
/// release+repress before the next tick latches both `released` and `pressed`
/// and remains `held`.
#[test]
fn fnd_t03_rapid_release_and_repress_while_held_preserves_both_edges() -> Result<(), InputError> {
    let activate = ActionId::new(0)?;
    let mut map = InputMap::new();
    map.bind_action(activate, ButtonTrigger::Key(KeyCode::E))?;
    let mut input = InputState::with_map(map);

    input.handle_window_event(&key_event(KeyCode::E, 18, ElementState::Pressed));
    let _ = input.sample_tick(Tick::ZERO);

    input.handle_window_event(&key_event(KeyCode::E, 18, ElementState::Released));
    input.handle_window_event(&key_event(KeyCode::E, 18, ElementState::Pressed));
    let cmd1 = input.sample_tick(Tick::new(1));
    assert_eq!(
        cmd1.action_state(activate),
        ButtonState {
            pressed: true,
            held: true,
            released: true,
        }
    );

    let cmd2 = input.sample_tick(Tick::new(2));
    assert_eq!(
        cmd2.action_state(activate),
        ButtonState {
            pressed: false,
            held: true,
            released: false,
        }
    );
    Ok(())
}

/// FND-T03 (FR-09): duplicate raw press/release transitions and overlapping
/// alternative bindings for the same semantic action never double-trigger
/// edges or release prematurely.
#[test]
fn fnd_t03_duplicate_raw_edges_and_overlapping_bindings_never_double_fire() -> Result<(), InputError>
{
    let confirm = ActionId::new(0)?;
    let mut map = InputMap::new();
    map.bind_action(confirm, ButtonTrigger::Key(KeyCode::Enter))?;
    map.bind_action(confirm, ButtonTrigger::Key(KeyCode::NumpadEnter))?;
    let mut input = InputState::with_map(map);

    // Press Enter; action activates once.
    input.handle_window_event(&key_event(KeyCode::Enter, 28, ElementState::Pressed));
    assert!(input.sample_tick(Tick::ZERO).is_pressed(confirm));

    // Redundant Pressed on Enter AND pressing NumpadEnter while Enter is already
    // down must not emit another press edge.
    input.handle_window_event(&key_event(KeyCode::Enter, 28, ElementState::Pressed));
    input.handle_window_event(&key_event(KeyCode::NumpadEnter, 96, ElementState::Pressed));
    let cmd1 = input.sample_tick(Tick::new(1));
    assert_eq!(
        cmd1.action_state(confirm),
        ButtonState {
            pressed: false,
            held: true,
            released: false,
        }
    );

    // Releasing Enter while NumpadEnter remains held keeps the action held.
    input.handle_window_event(&key_event(KeyCode::Enter, 28, ElementState::Released));
    let cmd2 = input.sample_tick(Tick::new(2));
    assert_eq!(
        cmd2.action_state(confirm),
        ButtonState {
            pressed: false,
            held: true,
            released: false,
        }
    );

    // Releasing NumpadEnter finally releases the action; a stray duplicate
    // Released afterwards does nothing.
    input.handle_window_event(&key_event(KeyCode::NumpadEnter, 96, ElementState::Released));
    assert_eq!(
        input.sample_tick(Tick::new(3)).action_state(confirm),
        ButtonState {
            pressed: false,
            held: false,
            released: true,
        }
    );
    input.handle_window_event(&key_event(KeyCode::NumpadEnter, 96, ElementState::Released));
    assert_eq!(
        input.sample_tick(Tick::new(4)).action_state(confirm),
        ButtonState::UP
    );

    Ok(())
}

/// FND-T04 (FR-09 / INV-11): losing window focus (`WindowEvent::Focused(false)`)
/// releases all held keyboard keys, mouse buttons, active actions, and held
/// digital axes on the next tick so no input remains stuck, and ignores any
/// late physical release arriving afterwards.
#[test]
fn fnd_t04_focus_loss_releases_all_held_keys_buttons_actions_and_axes() -> Result<(), InputError> {
    let act_key = ActionId::new(0)?;
    let act_mouse = ActionId::new(1)?;
    let unpressed_action = ActionId::new(2)?;
    let vertical = AxisId::new(0)?;

    let mut map = InputMap::new();
    map.bind_action(act_key, ButtonTrigger::Key(KeyCode::ShiftLeft))?;
    map.bind_action(act_mouse, ButtonTrigger::MouseButton(MouseButton::Right))?;
    map.bind_action(unpressed_action, ButtonTrigger::Key(KeyCode::Tab))?;
    map.bind_axis_digital_pair(
        vertical,
        ButtonTrigger::Key(KeyCode::S),
        ButtonTrigger::Key(KeyCode::W),
    )?;
    map.set_axis_config(vertical, AxisConfig::unit());

    let mut input = InputState::with_map(map);
    input.handle_window_event(&WindowEvent::Focused(true));
    assert!(input.devices().is_focused());

    input.handle_window_event(&key_event(KeyCode::ShiftLeft, 42, ElementState::Pressed));
    input.handle_window_event(&key_event(KeyCode::W, 17, ElementState::Pressed));
    input.handle_window_event(&WindowEvent::PointerButton {
        button: MouseButton::Right,
        state: ElementState::Pressed,
    });
    input.handle_window_event(&WindowEvent::PointerEntered(PhysicalPosition::new(
        50.0, 50.0,
    )));

    let cmd0 = input.sample_tick(Tick::ZERO);
    assert!(cmd0.is_held(act_key));
    assert!(cmd0.is_held(act_mouse));
    assert_eq!(cmd0.axis(vertical), 1.0);

    // Focus lost while keys and mouse button are held.
    input.handle_window_event(&WindowEvent::Focused(false));
    assert!(!input.devices().is_focused());

    let cmd1 = input.sample_tick(Tick::new(1));
    assert_eq!(
        cmd1.action_state(act_key),
        ButtonState {
            pressed: false,
            held: false,
            released: true,
        }
    );
    assert_eq!(
        cmd1.action_state(act_mouse),
        ButtonState {
            pressed: false,
            held: false,
            released: true,
        }
    );
    // Unheld action gets no bogus release edge.
    assert_eq!(cmd1.action_state(unpressed_action), ButtonState::UP);
    assert_eq!(cmd1.axis(vertical), 0.0);
    assert_eq!(
        input.devices().key_state(KeyCode::ShiftLeft),
        ButtonState {
            pressed: false,
            held: false,
            released: true,
        }
    );
    assert_eq!(
        input.devices().mouse_button_state(MouseButton::Right),
        ButtonState {
            pressed: false,
            held: false,
            released: true,
        }
    );

    // Late physical release and cursor re-anchor after focus return cause no
    // duplicate release or cursor jump delta.
    input.handle_window_event(&key_event(KeyCode::ShiftLeft, 42, ElementState::Released));
    input.handle_window_event(&WindowEvent::Focused(true));
    input.handle_window_event(&WindowEvent::PointerMoved(PhysicalPosition::new(
        400.0, 300.0,
    )));
    let cmd2 = input.sample_tick(Tick::new(2));
    assert_eq!(cmd2.action_state(act_key), ButtonState::UP);
    assert_eq!(cmd2.action_state(act_mouse), ButtonState::UP);
    assert_eq!(input.devices().cursor_delta(), PointerDelta::ZERO);

    Ok(())
}

/// FND-T04 (INV-11): window close requests and out-of-order multi-window focus
/// transfers never leave held keys stuck.
#[test]
fn fnd_t04_window_close_and_multi_window_focus_transfer_release_held_inputs()
-> Result<(), InputError> {
    let hold_action = ActionId::new(0)?;
    let mut map = InputMap::new();
    map.bind_action(hold_action, ButtonTrigger::Key(KeyCode::Space))?;
    let mut input = InputState::with_map(map);

    let win1 = WindowId::from_raw(1);
    let win2 = WindowId::from_raw(2);

    input.handle_event(&Event::Window {
        window: win1,
        event: WindowEvent::Focused(true),
    });
    input.handle_event(&Event::Window {
        window: win1,
        event: key_event(KeyCode::Space, 57, ElementState::Pressed),
    });
    assert!(input.sample_tick(Tick::ZERO).is_held(hold_action));

    // Out-of-order focus transfer: win2 reports Focused(true) before win1
    // reports Focused(false).
    input.handle_event(&Event::Window {
        window: win2,
        event: WindowEvent::Focused(true),
    });
    input.handle_event(&Event::Window {
        window: win1,
        event: WindowEvent::Focused(false),
    });
    assert!(input.devices().is_focused());
    assert_eq!(input.devices().focused_window(), Some(win2));
    assert_eq!(
        input.sample_tick(Tick::new(1)).action_state(hold_action),
        ButtonState {
            pressed: false,
            held: false,
            released: true,
        }
    );

    // Press again in win2, then deliver CloseRequested -> held state releases.
    input.handle_event(&Event::Window {
        window: win2,
        event: key_event(KeyCode::Space, 57, ElementState::Pressed),
    });
    assert!(input.sample_tick(Tick::new(2)).is_held(hold_action));

    input.handle_event(&Event::Window {
        window: win2,
        event: WindowEvent::CloseRequested,
    });
    assert_eq!(
        input.sample_tick(Tick::new(3)).action_state(hold_action),
        ButtonState {
            pressed: false,
            held: false,
            released: true,
        }
    );
    Ok(())
}

/// FND-T04 (FR-09 / INV-11): disconnecting a single device (`Keyboard` or
/// `Mouse`) releases only that device's held inputs and actions satisfied
/// solely by it, while keeping actions satisfied by a connected device held.
#[test]
fn fnd_t04_device_disconnect_releases_only_disconnected_device_inputs() -> Result<(), InputError> {
    let kb_only = ActionId::new(0)?;
    let mouse_only = ActionId::new(1)?;
    let either = ActionId::new(2)?;

    let mut map = InputMap::new();
    map.bind_action(kb_only, ButtonTrigger::Key(KeyCode::F))?;
    map.bind_action(mouse_only, ButtonTrigger::MouseButton(MouseButton::Left))?;
    map.bind_action(either, ButtonTrigger::Key(KeyCode::Space))?;
    map.bind_action(either, ButtonTrigger::MouseButton(MouseButton::Right))?;

    let mut input = InputState::with_map(map);
    input.handle_window_event(&key_event(KeyCode::F, 33, ElementState::Pressed));
    input.handle_window_event(&key_event(KeyCode::Space, 57, ElementState::Pressed));
    input.handle_window_event(&WindowEvent::PointerButton {
        button: MouseButton::Left,
        state: ElementState::Pressed,
    });
    input.handle_window_event(&WindowEvent::PointerButton {
        button: MouseButton::Right,
        state: ElementState::Pressed,
    });
    input.handle_window_event(&WindowEvent::PointerEntered(PhysicalPosition::new(
        12.0, 34.0,
    )));
    let _ = input.sample_tick(Tick::ZERO);

    // Disconnect keyboard: kb_only releases; mouse_only and either (held via Right) stay held.
    input.disconnect_device(DeviceKind::Keyboard);
    let cmd1 = input.sample_tick(Tick::new(1));
    assert_eq!(
        cmd1.action_state(kb_only),
        ButtonState {
            pressed: false,
            held: false,
            released: true,
        }
    );
    assert!(cmd1.is_held(mouse_only));
    assert!(cmd1.is_held(either));
    assert!(!cmd1.is_released(either));

    // Re-press F on keyboard, then emit `PointerLeft` (e.g. pointer focus
    // revoked or pointer seat capability lost while keyboard stays focused):
    // mouse buttons release while keyboard keys stay held.
    input.handle_window_event(&key_event(KeyCode::F, 33, ElementState::Pressed));
    input.handle_window_event(&WindowEvent::PointerLeft);
    assert_eq!(input.devices().cursor_position(), None);
    let cmd2 = input.sample_tick(Tick::new(2));
    assert!(cmd2.is_held(kb_only));
    assert_eq!(
        cmd2.action_state(mouse_only),
        ButtonState {
            pressed: false,
            held: false,
            released: true,
        }
    );
    assert_eq!(
        cmd2.action_state(either),
        ButtonState {
            pressed: false,
            held: false,
            released: true,
        }
    );

    // Explicit mouse disconnect when re-pressed also releases mouse buttons.
    input.handle_window_event(&WindowEvent::PointerButton {
        button: MouseButton::Left,
        state: ElementState::Pressed,
    });
    let _ = input.sample_tick(Tick::new(3));
    input.disconnect_device(DeviceKind::Mouse);
    let cmd4 = input.sample_tick(Tick::new(4));
    assert_eq!(
        cmd4.action_state(mouse_only),
        ButtonState {
            pressed: false,
            held: false,
            released: true,
        }
    );
    assert!(cmd4.is_held(kb_only));

    Ok(())
}

/// Disabling an input context or live-rebinding an action while its trigger is
/// held releases the action cleanly instead of leaving it permanently held.
#[test]
fn fnd_t04_context_deactivation_and_live_rebinding_release_affected_actions()
-> Result<(), InputError> {
    let overlay_ctx = ContextId::new(1)?;
    let confirm = ActionId::new(0)?;
    let secondary = ActionId::new(1)?;

    let mut map = InputMap::new();
    map.bind_action_in(overlay_ctx, confirm, ButtonTrigger::Key(KeyCode::Enter))?;
    map.bind_action(secondary, ButtonTrigger::Key(KeyCode::Q))?;

    let mut input = InputState::with_map(map);
    input.set_context_active(overlay_ctx, true);

    input.handle_window_event(&key_event(KeyCode::Enter, 28, ElementState::Pressed));
    input.handle_window_event(&key_event(KeyCode::Q, 16, ElementState::Pressed));
    let cmd0 = input.sample_tick(Tick::ZERO);
    assert!(cmd0.is_held(confirm));
    assert!(cmd0.is_held(secondary));

    // Deactivate overlay_ctx and rebind `secondary` away from Q to E while both
    // Enter and Q remain physically held.
    input.set_context_active(overlay_ctx, false);
    input.mutate_map(|m| m.rebind_action(secondary, ButtonTrigger::Key(KeyCode::E)))?;

    let cmd1 = input.sample_tick(Tick::new(1));
    assert_eq!(
        cmd1.action_state(confirm),
        ButtonState {
            pressed: false,
            held: false,
            released: true,
        }
    );
    assert_eq!(
        cmd1.action_state(secondary),
        ButtonState {
            pressed: false,
            held: false,
            released: true,
        }
    );

    // Pressing the newly rebound key E activates `secondary`.
    input.handle_window_event(&key_event(KeyCode::E, 18, ElementState::Pressed));
    assert!(input.sample_tick(Tick::new(2)).is_pressed(secondary));

    Ok(())
}

/// Explicit `reset()` clears all pending edges, held state, and deltas
/// immediately without discarding the configured bindings.
#[test]
fn fnd_t04_explicit_reset_clears_pending_and_held_state_preserving_bindings()
-> Result<(), InputError> {
    let confirm = ActionId::new(0)?;
    let mut map = InputMap::new();
    map.bind_action(confirm, ButtonTrigger::Key(KeyCode::Space))?;
    let mut input = InputState::with_map(map);

    input.handle_window_event(&key_event(KeyCode::Space, 57, ElementState::Pressed));
    input.handle_relative_motion(12.0, -4.0);
    input.reset();

    assert_eq!(input.last_command(), None);
    let cmd0 = input.sample_tick(Tick::ZERO);
    assert_eq!(cmd0.action_state(confirm), ButtonState::UP);
    assert_eq!(input.devices().pointer_delta(), PointerDelta::ZERO);

    // Configured Space -> confirm binding still works after reset.
    input.handle_window_event(&key_event(KeyCode::Space, 57, ElementState::Pressed));
    assert!(input.sample_tick(Tick::new(1)).is_pressed(confirm));
    Ok(())
}

/// Identical synthetic event and frame-duration streams replayed through two
/// independent `(FixedStepClock, InputState)` instances yield bit-identical
/// command sequences, and every generated command passes schema validation.
#[test]
fn deterministic_replay_produces_bit_identical_validated_commands() -> Result<(), Box<dyn Error>> {
    struct ReplayFrame<'a> {
        dt: Duration,
        events: &'a [WindowEvent],
        relative_motion: Option<(f64, f64)>,
    }

    fn build_input() -> Result<InputState, InputError> {
        let act_a = ActionId::new(0)?;
        let act_b = ActionId::new(1)?;
        let move_x = AxisId::new(0)?;
        let look_x = AxisId::new(1)?;
        let look_y = AxisId::new(2)?;
        let wheel = AxisId::new(3)?;

        let mut map = InputMap::new();
        map.bind_action(act_a, ButtonTrigger::Key(KeyCode::Space))?;
        map.bind_action(act_b, ButtonTrigger::MouseButton(MouseButton::Left))?;
        map.bind_axis_digital_pair(
            move_x,
            ButtonTrigger::Key(KeyCode::A),
            ButtonTrigger::Key(KeyCode::D),
        )?;
        map.set_axis_config(move_x, AxisConfig::unit().with_dead_zone(0.1)?);
        map.bind_axis(look_x, AxisSource::PointerMotion(MotionAxis::X))?;
        map.set_axis_config(look_x, AxisConfig::new().with_sensitivity(0.25)?);
        map.bind_axis(look_y, AxisSource::PointerMotion(MotionAxis::Y))?;
        map.set_axis_config(
            look_y,
            AxisConfig::new()
                .with_sensitivity(0.25)?
                .with_inverted(true),
        );
        map.bind_axis(wheel, AxisSource::ScrollLines(MotionAxis::Y))?;

        Ok(InputState::with_map(map))
    }

    fn run_trace() -> Result<(Vec<InputCommand>, InputMap), Box<dyn Error>> {
        let mut input = build_input()?;
        let mut clock = FixedStepClock::new(FixedStepConfig::default());
        let win = WindowId::from_raw(1);

        let frames: &[ReplayFrame<'_>] = &[
            ReplayFrame {
                dt: Duration::from_millis(5),
                events: &[
                    WindowEvent::Focused(true),
                    key_event(KeyCode::D, 32, ElementState::Pressed),
                    key_event(KeyCode::Space, 57, ElementState::Pressed),
                ],
                relative_motion: Some((8.0, -4.0)),
            },
            ReplayFrame {
                dt: Duration::from_millis(12),
                events: &[WindowEvent::Scroll(ScrollDelta::Lines { x: 0.0, y: 1.0 })],
                relative_motion: Some((4.0, 0.0)),
            },
            ReplayFrame {
                dt: Duration::from_millis(35),
                events: &[
                    key_event(KeyCode::Space, 57, ElementState::Released),
                    WindowEvent::PointerButton {
                        button: MouseButton::Left,
                        state: ElementState::Pressed,
                    },
                    WindowEvent::PointerButton {
                        button: MouseButton::Left,
                        state: ElementState::Released,
                    },
                ],
                relative_motion: None,
            },
            ReplayFrame {
                dt: Duration::from_millis(18),
                events: &[WindowEvent::Focused(false)],
                relative_motion: Some((0.0, 0.0)),
            },
            ReplayFrame {
                dt: Duration::from_millis(18),
                events: &[],
                relative_motion: None,
            },
        ];

        let mut recorded = Vec::new();
        for frame in frames {
            for ev in frame.events {
                input.handle_event(&Event::Window {
                    window: win,
                    event: *ev,
                });
            }
            if let Some((dx, dy)) = frame.relative_motion {
                input.handle_relative_motion(dx, dy);
            }
            for tick in clock.advance(frame.dt)?.ticks() {
                let cmd = input.sample_tick(tick);
                input.map().validate_command(&cmd)?;
                recorded.push(cmd);
            }
        }
        Ok((recorded, input.map().clone()))
    }

    let (first, map) = run_trace()?;
    let (second, _) = run_trace()?;
    assert!(!first.is_empty());
    assert_eq!(first, second);

    // Check exact bit equality on every axis slot (including canonical +0.0 on
    // inverted `look_y`).
    for (a, b) in first.iter().zip(second.iter()) {
        assert_eq!(a.tick(), b.tick());
        assert_eq!(a.pressed_mask(), b.pressed_mask());
        assert_eq!(a.held_mask(), b.held_mask());
        assert_eq!(a.released_mask(), b.released_mask());
        for (&ax_a, &ax_b) in a.axes().iter().zip(b.axes().iter()) {
            assert_eq!(ax_a.to_bits(), ax_b.to_bits());
        }
        map.validate_command(a)?;
    }

    Ok(())
}

/// Non-input `WindowEvent` variants (`Resized`, `ScaleFactorChanged`,
/// `Occluded`) and extreme finite or non-finite coordinates never panic or
/// produce invalid `InputCommand` values.
#[test]
fn non_input_window_events_and_extreme_floats_are_harmless() -> Result<(), Box<dyn Error>> {
    let _ = WindowDescriptor::new("headless check", LogicalSize::new(800, 600))?;

    let pan_x = AxisId::new(0)?;
    let scroll_y = AxisId::new(1)?;
    let mut map = InputMap::new();
    map.bind_axis(pan_x, AxisSource::PointerMotion(MotionAxis::X))?;
    map.bind_axis(scroll_y, AxisSource::ScrollPixels(MotionAxis::Y))?;

    let mut input = InputState::with_map(map);
    input.handle_window_event(&WindowEvent::Resized(PhysicalSize::new(1920, 1080)));
    input.handle_window_event(&WindowEvent::ScaleFactorChanged { scale_factor: 1.5 });
    input.handle_window_event(&WindowEvent::Occluded(true));
    input.handle_window_event(&WindowEvent::Occluded(false));

    // Extreme finite coordinates whose difference exceeds f64::MAX must clamp
    // cleanly without producing infinity or NaN.
    input.handle_window_event(&WindowEvent::PointerEntered(PhysicalPosition::new(
        -f64::MAX,
        0.0,
    )));
    input.handle_window_event(&WindowEvent::PointerMoved(PhysicalPosition::new(
        f64::MAX,
        0.0,
    )));
    input.handle_window_event(&WindowEvent::Scroll(ScrollDelta::Pixels {
        x: f64::NAN,
        y: f64::INFINITY,
    }));
    input.handle_relative_motion(f64::NAN, f64::NEG_INFINITY);

    let cmd = input.sample_tick(Tick::ZERO);
    input.map().validate_command(&cmd)?;
    assert!(cmd.axis(pan_x).is_finite());
    assert_eq!(cmd.axis(scroll_y), 0.0);
    Ok(())
}

/// Replacing `InputMap` via `set_map` while actions are held or pending clears
/// bits absent from the new map's schema while still emitting `released` for
/// actions registered in the new map, keeping `map().validate_command(&cmd)`
/// valid on the next tick. Also covers unilateral `[0.0, 1.0]` / `[-1.0, 0.0]`
/// axis clamp ranges at rest and under load.
#[test]
fn set_map_preserves_command_validity_and_unilateral_ranges_validate() -> Result<(), Box<dyn Error>>
{
    let dropped_action = ActionId::new(0)?;
    let kept_action = ActionId::new(1)?;
    let positive_only = AxisId::new(0)?;
    let negative_only = AxisId::new(1)?;

    let mut old_map = InputMap::new();
    old_map.bind_action(dropped_action, ButtonTrigger::Key(KeyCode::Q))?;
    old_map.bind_action(kept_action, ButtonTrigger::Key(KeyCode::E))?;

    let mut input = InputState::with_map(old_map);
    input.handle_window_event(&key_event(KeyCode::Q, 16, ElementState::Pressed));
    input.handle_window_event(&key_event(KeyCode::E, 18, ElementState::Pressed));
    let cmd0 = input.sample_tick(Tick::ZERO);
    assert!(cmd0.is_held(dropped_action));
    assert!(cmd0.is_held(kept_action));

    // Also latch a pending press on another unregistered action before swapping maps.
    let transient_action = ActionId::new(2)?;
    input.mutate_map(|m| m.bind_action(transient_action, ButtonTrigger::Key(KeyCode::R)))?;
    input.handle_window_event(&key_event(KeyCode::R, 19, ElementState::Pressed));
    input.handle_window_event(&key_event(KeyCode::R, 19, ElementState::Released));

    // Build a new map that registers `kept_action` (unbound) and unilateral
    // `[0.0, 1.0]` / `[-1.0, 0.0]` axes, omitting `dropped_action` and `transient_action`.
    let mut new_map = InputMap::new();
    new_map.register_action(kept_action);
    new_map.bind_axis(
        positive_only,
        AxisSource::Button {
            trigger: ButtonTrigger::Key(KeyCode::W),
            scale: 1.0,
        },
    )?;
    new_map.set_axis_config(positive_only, AxisConfig::new().with_range(0.0, 1.0)?);
    new_map.bind_axis(
        negative_only,
        AxisSource::Button {
            trigger: ButtonTrigger::Key(KeyCode::S),
            scale: -1.0,
        },
    )?;
    new_map.set_axis_config(negative_only, AxisConfig::new().with_range(-1.0, 0.0)?);

    input.set_map(new_map);

    let cmd1 = input.sample_tick(Tick::new(1));
    // Must pass `new_map`'s strict schema validation!
    input.map().validate_command(&cmd1)?;
    assert_eq!(cmd1.action_state(dropped_action), ButtonState::UP);
    assert_eq!(cmd1.action_state(transient_action), ButtonState::UP);
    assert_eq!(
        cmd1.action_state(kept_action),
        ButtonState {
            pressed: false,
            held: false,
            released: true,
        }
    );
    assert_eq!(cmd1.axis(positive_only), 0.0);
    assert_eq!(cmd1.axis(negative_only), 0.0);

    // Activate both unilateral axes and verify clamping + validation.
    input.handle_window_event(&key_event(KeyCode::W, 17, ElementState::Pressed));
    input.handle_window_event(&key_event(KeyCode::S, 31, ElementState::Pressed));
    let cmd2 = input.sample_tick(Tick::new(2));
    input.map().validate_command(&cmd2)?;
    assert_eq!(cmd2.axis(positive_only), 1.0);
    assert_eq!(cmd2.axis(negative_only), -1.0);

    Ok(())
}

/// Out-of-order multi-window pointer events (`win2` `PointerEntered` + button
/// press before `win1` `PointerLeft`, or `PointerMoved` switching windows)
/// never clobber the active pointer window's held buttons or compute a
/// cross-window cursor delta.
#[test]
fn fnd_t04_multi_window_pointer_focus_isolates_cursor_and_buttons() -> Result<(), Box<dyn Error>> {
    let click = ActionId::new(0)?;
    let mut map = InputMap::new();
    map.bind_action(click, ButtonTrigger::MouseButton(MouseButton::Left))?;
    let mut input = InputState::with_map(map);

    let win1 = WindowId::from_raw(1);
    let win2 = WindowId::from_raw(2);

    input.handle_event(&Event::Window {
        window: win1,
        event: WindowEvent::PointerEntered(PhysicalPosition::new(10.0, 20.0)),
    });
    assert_eq!(input.devices().pointer_window(), Some(win1));

    // Out-of-order pointer transfer: win2 gets PointerEntered + button press
    // BEFORE win1's stale PointerLeft arrives.
    input.handle_event(&Event::Window {
        window: win2,
        event: WindowEvent::PointerEntered(PhysicalPosition::new(300.0, 400.0)),
    });
    input.handle_event(&Event::Window {
        window: win2,
        event: WindowEvent::PointerButton {
            button: MouseButton::Left,
            state: ElementState::Pressed,
        },
    });
    // Late stale PointerLeft and CloseRequested on win1 must NOT disturb win2!
    input.handle_event(&Event::Window {
        window: win1,
        event: WindowEvent::PointerLeft,
    });
    input.handle_event(&Event::Window {
        window: win2,
        event: WindowEvent::Focused(true),
    });
    input.handle_event(&Event::Window {
        window: win1,
        event: WindowEvent::CloseRequested,
    });

    assert_eq!(input.devices().pointer_window(), Some(win2));
    assert_eq!(
        input.devices().cursor_position(),
        Some(PhysicalPosition::new(300.0, 400.0))
    );
    let cmd0 = input.sample_tick(Tick::ZERO);
    assert_eq!(
        cmd0.action_state(click),
        ButtonState {
            pressed: true,
            held: true,
            released: false,
        }
    );
    assert_eq!(input.devices().cursor_delta(), PointerDelta::ZERO);

    // Synthetic PointerMoved directly on win1 without PointerEntered re-anchors
    // rather than computing (50 - 300, 60 - 400) across windows.
    input.handle_event(&Event::Window {
        window: win1,
        event: WindowEvent::PointerMoved(PhysicalPosition::new(50.0, 60.0)),
    });
    input.handle_event(&Event::Window {
        window: win1,
        event: WindowEvent::PointerMoved(PhysicalPosition::new(55.0, 64.0)),
    });
    let _ = input.sample_tick(Tick::new(1));
    assert_eq!(input.devices().pointer_window(), Some(win1));
    assert_eq!(input.devices().cursor_delta(), PointerDelta::new(5.0, 4.0));

    Ok(())
}

/// Phase 2b -> Phase 4 end-to-end integration: `Event::Window` carrying
/// `WindowEvent::PointerRelativeMotion { dx, dy }` and
/// `WindowEvent::CursorGrabChanged` drives semantic look axes, prefers
/// unaccelerated relative deltas over `PointerMoved` displacement when both
/// arrive in the same tick, does not duplicate across catch-up ticks, and
/// isolates relative motion to the active `pointer_window`.
#[test]
fn phase_2b_pointer_relative_motion_events_drive_axes_end_to_end() -> Result<(), Box<dyn Error>> {
    use spawn_platform::CursorGrabMode;

    let look_x = AxisId::new(0)?;
    let look_y = AxisId::new(1)?;
    let mut map = InputMap::new();
    map.bind_axis(look_x, AxisSource::PointerMotion(MotionAxis::X))?;
    map.set_axis_config(look_x, AxisConfig::new().with_sensitivity(0.5)?);
    map.bind_axis(look_y, AxisSource::PointerMotion(MotionAxis::Y))?;
    map.set_axis_config(
        look_y,
        AxisConfig::new().with_sensitivity(0.5)?.with_inverted(true),
    );

    let mut input = InputState::with_map(map);
    let win1 = WindowId::from_raw(1);
    let win2 = WindowId::from_raw(2);

    // Unlocked/confined coexistence: both PointerMoved and PointerRelativeMotion
    // arrive before tick 0; pointer_delta uses unaccelerated relative motion
    // without double-counting surface displacement.
    input.handle_events(&[
        Event::Window {
            window: win1,
            event: WindowEvent::PointerEntered(PhysicalPosition::new(100.0, 100.0)),
        },
        Event::Window {
            window: win1,
            event: WindowEvent::PointerMoved(PhysicalPosition::new(130.0, 120.0)),
        },
        Event::Window {
            window: win1,
            event: WindowEvent::PointerRelativeMotion { dx: 10.0, dy: -4.0 },
        },
        Event::Window {
            window: win1,
            event: WindowEvent::CursorGrabChanged(CursorGrabMode::Locked),
        },
        Event::Window {
            window: win1,
            event: WindowEvent::PointerRelativeMotion { dx: 6.0, dy: -2.0 },
        },
        // Stale relative motion from another window is ignored.
        Event::Window {
            window: win2,
            event: WindowEvent::PointerRelativeMotion {
                dx: 500.0,
                dy: 500.0,
            },
        },
    ]);

    // Total relative delta on win1: dx = 16.0, dy = -6.0.
    // With sensitivity 0.5 (and inverted Y): look_x = 8.0, look_y = 3.0.
    let cmd0 = input.sample_tick(Tick::ZERO);
    input.map().validate_command(&cmd0)?;
    assert_eq!(
        input.devices().relative_delta(),
        PointerDelta::new(16.0, -6.0)
    );
    assert_eq!(
        input.devices().pointer_delta(),
        PointerDelta::new(16.0, -6.0)
    );
    assert_eq!(cmd0.axis(look_x), 8.0);
    assert_eq!(cmd0.axis(look_y), 3.0);

    // Catch-up tick with no new events does not duplicate relative delta.
    let cmd1 = input.sample_tick(Tick::new(1));
    input.map().validate_command(&cmd1)?;
    assert_eq!(input.devices().pointer_delta(), PointerDelta::ZERO);
    assert_eq!(cmd1.axis(look_x), 0.0);
    assert_eq!(cmd1.axis(look_y), 0.0);

    // Unlock notification is harmless.
    input.handle_event(&Event::Window {
        window: win1,
        event: WindowEvent::CursorGrabChanged(CursorGrabMode::None),
    });
    let cmd2 = input.sample_tick(Tick::new(2));
    input.map().validate_command(&cmd2)?;
    assert_eq!(cmd2.axis(look_x), 0.0);

    Ok(())
}
