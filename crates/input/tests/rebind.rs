mod common;

use common::*;
use pfx_input::{
    Binding, BindingMap, Button, Conflict, Input, Key, MouseButton, OnConflict, RebindError, Set,
    Slot, Source,
};

#[test]
fn rebinding_a_free_key_moves_the_action() {
    let actions = actions();
    let mut map = BindingMap::defaults(&actions);
    let conflicts = map
        .rebind_key(
            &actions,
            "burst",
            Slot::keyboard(0, 0),
            Source::Key(Key::B),
            OnConflict::Refuse,
        )
        .unwrap();
    assert!(conflicts.is_empty());
    assert_eq!(
        map.bindings(Set::Keyboard, "burst")[0],
        Binding::key(Key::B)
    );
    let mut input = Input::new(actions, STEP_US);
    input.set_bindings(map);
    input.feed(key(Key::Space, true));
    input.step();
    assert!(!input.action("burst").held);
    input.feed(key(Key::B, true));
    input.step();
    assert!(input.action("burst").pressed);
}

#[test]
fn a_conflict_in_the_same_context_is_refused() {
    let actions = actions();
    let mut map = BindingMap::defaults(&actions);
    let error = map
        .rebind_key(
            &actions,
            "burst",
            Slot::keyboard(0, 0),
            Source::Key(Key::Digit1),
            OnConflict::Refuse,
        )
        .unwrap_err();
    assert_eq!(
        error,
        RebindError::Conflicts(vec![Conflict {
            action: "rule1".into(),
            slot: Slot::keyboard(0, 0),
            source: Source::Key(Key::Digit1),
        }])
    );
    assert_eq!(map, BindingMap::defaults(&actions));
    let error = map
        .rebind_key(
            &actions,
            "rule3",
            Slot::keyboard(0, 0),
            Source::Key(Key::D),
            OnConflict::Refuse,
        )
        .unwrap_err();
    let RebindError::Conflicts(conflicts) = error else {
        panic!("{error:?}");
    };
    assert_eq!(conflicts[0].action, "column");
    assert_eq!(conflicts[0].slot, Slot::keyboard(0, 1));
}

#[test]
fn other_contexts_may_share_a_key() {
    let actions = actions();
    let mut map = BindingMap::defaults(&actions);
    assert_eq!(
        map.bindings(Set::Keyboard, "pause"),
        map.bindings(Set::Keyboard, "back")
    );
    let conflicts = map
        .rebind_key(
            &actions,
            "confirm",
            Slot::keyboard(0, 0),
            Source::Key(Key::Space),
            OnConflict::Refuse,
        )
        .unwrap();
    assert!(conflicts.is_empty());
}

#[test]
fn swap_hands_the_old_key_to_the_other_action() {
    let actions = actions();
    let mut map = BindingMap::defaults(&actions);
    let conflicts = map
        .rebind_key(
            &actions,
            "rule1",
            Slot::keyboard(0, 0),
            Source::Key(Key::Digit2),
            OnConflict::Swap,
        )
        .unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(
        map.bindings(Set::Keyboard, "rule1")[0],
        Binding::key(Key::Digit2)
    );
    assert_eq!(
        map.bindings(Set::Keyboard, "rule2")[0],
        Binding::key(Key::Digit1)
    );
    map.rebind_key(
        &actions,
        "column",
        Slot::keyboard(0, 0),
        Source::Key(Key::D),
        OnConflict::Swap,
    )
    .unwrap();
    assert_eq!(
        map.bindings(Set::Keyboard, "column")[0],
        Binding::keys(Key::D, Key::A)
    );
}

#[test]
fn replace_unbinds_the_other_action() {
    let actions = actions();
    let mut map = BindingMap::defaults(&actions);
    map.rebind_key(
        &actions,
        "burst",
        Slot::keyboard(1, 0),
        Source::Key(Key::Digit3),
        OnConflict::Replace,
    )
    .unwrap();
    assert!(map.bindings(Set::Keyboard, "rule3").is_empty());
    assert_eq!(
        map.bindings(Set::Keyboard, "burst")[1],
        Binding::key(Key::Digit3)
    );
    map.add("burst", Set::Keyboard, Binding::key(Key::K));
    map.rebind_key(
        &actions,
        "burst",
        Slot::keyboard(2, 0),
        Source::Key(Key::Space),
        OnConflict::Replace,
    )
    .unwrap();
    assert_eq!(
        map.bindings(Set::Keyboard, "burst"),
        &[Binding::key(Key::Digit3), Binding::key(Key::Space)]
    );
}

#[test]
fn keyboard_rebinding_takes_only_keys_and_mouse_buttons() {
    let actions = actions();
    let mut map = BindingMap::defaults(&actions);
    assert_eq!(
        map.rebind_key(
            &actions,
            "burst",
            Slot::keyboard(0, 0),
            Source::Button(Button::South),
            OnConflict::Refuse
        ),
        Err(RebindError::WrongDevice(Source::Button(Button::South)))
    );
    assert!(
        map.rebind_key(
            &actions,
            "burst",
            Slot::keyboard(0, 0),
            Source::Mouse(MouseButton::Right),
            OnConflict::Refuse
        )
        .is_ok()
    );
    assert_eq!(
        map.rebind_key(
            &actions,
            "burst",
            Slot::keyboard(9, 0),
            Source::Key(Key::Q),
            OnConflict::Refuse
        ),
        Err(RebindError::NoSlot(Slot::keyboard(9, 0)))
    );
    assert!(matches!(
        map.rebind_key(
            &actions,
            "nothing",
            Slot::keyboard(0, 0),
            Source::Key(Key::Q),
            OnConflict::Refuse
        ),
        Err(RebindError::UnknownAction(_))
    ));
}

#[test]
fn the_map_saves_and_loads_through_serde() {
    let actions = actions();
    let mut map = BindingMap::defaults(&actions);
    map.rebind_key(
        &actions,
        "navigate",
        Slot::keyboard(0, 1),
        Source::Key(Key::S),
        OnConflict::Refuse,
    )
    .unwrap();
    let saved = serde_json::to_string(&map).unwrap();
    let mut loaded: BindingMap = serde_json::from_str(&saved).unwrap();
    assert_eq!(loaded, map);
    loaded.keyboard.remove("rule1");
    loaded
        .keyboard
        .insert("retired".into(), vec![Binding::key(Key::Z)]);
    let mut input = Input::new(actions, STEP_US);
    input.set_bindings(loaded);
    assert!(!input.bindings().keyboard.contains_key("retired"));
    assert_eq!(
        input.bindings().bindings(Set::Keyboard, "rule1"),
        &[Binding::key(Key::Digit1)]
    );
    assert_eq!(
        input.bindings().bindings(Set::Keyboard, "navigate")[0],
        Binding::key_stick(Key::Up, Key::S, Key::Left, Key::Right)
    );
    input
        .edit_bindings(|map, actions| map.reset(actions, "navigate"))
        .unwrap();
    assert_eq!(
        input.bindings().bindings(Set::Keyboard, "navigate")[0],
        Binding::key_stick(Key::Up, Key::Down, Key::Left, Key::Right)
    );
}
