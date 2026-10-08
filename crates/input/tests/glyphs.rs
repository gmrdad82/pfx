use pfx_input::glyph::shape::{self, Shape};
use pfx_input::glyph::{ATLAS_DIGEST, BOX, Glyph, Mark, catalogue, shape_of};
use pfx_input::{
    Button, Control, Family, GlyphAtlas, GlyphStyle, Key, MouseButton, Stick, Wheel, glyph,
    glyph_for,
};

fn inside(shape: &Shape, x: f32, y: f32) -> bool {
    let mut inside = false;
    for line in shape.polylines() {
        for i in 0..line.len() {
            let a = line[i];
            let b = line[(i + 1) % line.len()];
            if (a[1] > y) != (b[1] > y) && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0] {
                inside = !inside;
            }
        }
    }
    inside
}

#[test]
fn every_glyph_is_closed_contours_that_never_touch() {
    for glyph in catalogue() {
        let shape = shape_of(glyph);
        assert!(!shape.contours.is_empty(), "{glyph:?} is empty");
        for contour in &shape.contours {
            assert!(contour.len() >= 3, "{glyph:?} has a degenerate contour");
            for point in contour {
                assert!(
                    point.x >= 0.0 && point.x <= shape.width,
                    "{glyph:?} leaves its box"
                );
                assert!(
                    point.y >= 0.0 && point.y <= shape.height,
                    "{glyph:?} leaves its box"
                );
            }
        }
        assert_eq!(
            shape::crossings(&shape),
            0,
            "{glyph:?} has crossing contours"
        );
        assert!(
            shape::min_gap(&shape) >= 0.9,
            "{glyph:?} has contours {} px apart",
            shape::min_gap(&shape)
        );
    }
}

#[test]
fn face_buttons_follow_each_family() {
    let south = |family| glyph(family, Control::Button(Button::South));
    assert_eq!(south(Family::Xbox), Glyph::Disc(Mark::Text("A")));
    assert_eq!(south(Family::SteamDeck), Glyph::Disc(Mark::Text("A")));
    assert_eq!(south(Family::SteamController), Glyph::Disc(Mark::Text("A")));
    assert_eq!(south(Family::DualSense), Glyph::Disc(Mark::Cross));
    assert_eq!(south(Family::DualShock4), Glyph::Disc(Mark::Cross));
    assert_eq!(south(Family::SwitchPro), Glyph::Disc(Mark::Text("B")));
    assert_eq!(
        glyph(Family::DualSense, Control::Button(Button::East)),
        Glyph::Disc(Mark::Circle)
    );
    assert_eq!(
        glyph(Family::DualSense, Control::Button(Button::West)),
        Glyph::Disc(Mark::Square)
    );
    assert_eq!(
        glyph(Family::DualSense, Control::Button(Button::North)),
        Glyph::Disc(Mark::Triangle)
    );
    assert_eq!(
        glyph(Family::SwitchPro, Control::Button(Button::East)),
        Glyph::Disc(Mark::Text("A"))
    );
    assert_eq!(
        glyph(Family::Xbox, Control::Button(Button::LeftBumper)),
        Glyph::Bumper("LB")
    );
    assert_eq!(
        glyph(Family::DualSense, Control::Button(Button::LeftBumper)),
        Glyph::Bumper("L1")
    );
    assert_eq!(
        glyph(Family::DualSense, Control::Button(Button::RightTrigger)),
        Glyph::Trigger("R2")
    );
    assert_eq!(
        glyph(Family::SwitchPro, Control::Button(Button::LeftTrigger)),
        Glyph::Trigger("ZL")
    );
    assert_eq!(
        glyph(Family::SteamDeck, Control::Button(Button::LeftGripLower)),
        Glyph::Grip("L5")
    );
    assert_eq!(
        glyph(Family::SteamController, Control::Button(Button::RightGrip)),
        Glyph::Grip("RG")
    );
    assert_ne!(
        glyph(Family::Xbox, Control::Button(Button::Menu)),
        glyph(Family::DualSense, Control::Button(Button::Menu))
    );
    for family in Family::ALL {
        let faces: Vec<Glyph> = [Button::South, Button::East, Button::West, Button::North]
            .into_iter()
            .map(|button| glyph(family, Control::Button(button)))
            .collect();
        for (i, a) in faces.iter().enumerate() {
            for b in &faces[i + 1..] {
                assert_ne!(a, b, "{family:?} repeats a face glyph");
            }
        }
    }
}

#[test]
fn keys_and_mouse_have_their_own_glyphs() {
    assert_eq!(glyph(Family::Xbox, Control::Key(Key::W)), Glyph::Keycap(48));
    assert_eq!(
        glyph(Family::DualSense, Control::Key(Key::Space)),
        Glyph::Keycap(80)
    );
    assert_eq!(
        glyph(Family::Generic, Control::Key(Key::ShiftLeft)),
        glyph(Family::Generic, Control::Key(Key::ShiftRight))
    );
    let mut seen = std::collections::BTreeSet::new();
    for button in [
        MouseButton::Left,
        MouseButton::Right,
        MouseButton::Middle,
        MouseButton::Back,
        MouseButton::Forward,
    ] {
        assert!(seen.insert(glyph(Family::Generic, Control::Mouse(button))));
    }
    for wheel in [Wheel::Up, Wheel::Down, Wheel::Left, Wheel::Right] {
        assert!(seen.insert(glyph(Family::Generic, Control::Wheel(wheel))));
    }
    assert!(seen.insert(glyph(Family::Generic, Control::MouseMove)));
    let space = shape_of(Glyph::Keycap(80));
    assert!(space.width > BOX);
    let w = shape_of(Glyph::Keycap(48));
    assert_eq!(w.width, BOX);
    assert!(inside(&w, 3.5, BOX / 2.0));
    assert!(!inside(&w, 8.0, BOX / 2.0));
    assert!(!inside(&w, BOX / 2.0, BOX / 2.0));
}

#[test]
fn knocked_out_marks_leave_holes() {
    let disc = shape_of(Glyph::Disc(Mark::Cross));
    assert!(!inside(&disc, BOX / 2.0, BOX / 2.0));
    assert!(inside(&disc, BOX / 2.0, 4.0));
    let stick = shape_of(Glyph::Stick("L"));
    assert!(inside(&stick, BOX / 2.0, 3.0));
    assert!(!inside(&stick, BOX / 2.0 + 7.0, BOX / 2.0 - 9.0));
}

#[test]
fn the_atlas_matches_its_pinned_digest() {
    let atlas = GlyphAtlas::build().unwrap();
    assert_eq!(atlas.len(), catalogue().len());
    let level = &atlas.atlas.levels[0];
    assert_eq!(atlas.atlas.channels, 3);
    for family in Family::ALL {
        for button in Button::ALL {
            assert!(atlas.cell(family, Control::Button(button)).is_some());
        }
        assert!(atlas.cell(family, Control::Stick(Stick::Left)).is_some());
        assert!(atlas.cell(family, Control::DPad).is_some());
    }
    for key in Key::ALL {
        assert!(atlas.cell(Family::Generic, Control::Key(key)).is_some());
    }
    for (glyph, cell) in atlas.iter() {
        assert!(cell.rect[0] + cell.rect[2] <= level.width, "{glyph:?}");
        assert!(cell.rect[1] + cell.rect[3] <= level.height, "{glyph:?}");
        let shape = shape_of(*glyph);
        for line in shape.polylines() {
            for p in line {
                assert!(
                    p[0] > cell.offset[0] && p[0] < cell.offset[0] + cell.size[0],
                    "{glyph:?}"
                );
                assert!(
                    p[1] > cell.offset[1] && p[1] < cell.offset[1] + cell.size[1],
                    "{glyph:?}"
                );
            }
        }
        assert_eq!(cell.box_size, [shape.width, shape.height]);
    }
    let cell = atlas
        .cell(Family::Xbox, Control::Button(Button::South))
        .unwrap();
    let texel = |x: f32, y: f32| {
        let px = (cell.rect[0] as f32 + x - cell.offset[0]) as u32;
        let py = (cell.rect[1] as f32 + y - cell.offset[1]) as u32;
        let at = ((py * level.width + px) * 3) as usize;
        let rgb = &level.bytes[at..at + 3];
        pfx_text::msdf_coverage(
            [
                rgb[0] as f32 / 255.0,
                rgb[1] as f32 / 255.0,
                rgb[2] as f32 / 255.0,
            ],
            8.0,
        )
    };
    assert!(texel(BOX / 2.0, 4.0) > 0.9);
    assert!(texel(1.0, 1.0) < 0.1);
    assert_eq!(
        atlas.digest(),
        ATLAS_DIGEST,
        "atlas {}x{}",
        level.width,
        level.height
    );
}

#[test]
fn every_hole_winds_opposite_to_its_parent() {
    for glyph in catalogue() {
        let shape = shape_of(glyph);
        let areas: Vec<f32> = shape
            .polylines()
            .iter()
            .map(|line| shape::signed_area(line))
            .collect();
        let parents = shape.parents();
        let depths = shape.depths();
        assert!(depths.contains(&0), "{glyph:?}");
        for (index, parent) in parents.iter().enumerate() {
            assert!(areas[index] != 0.0, "{glyph:?} contour {index} has no area");
            match parent {
                Some(parent) => assert!(
                    (areas[index] > 0.0) != (areas[*parent] > 0.0),
                    "{glyph:?} contour {index} winds like its parent {parent}"
                ),
                None => assert!(
                    areas[index] > 0.0,
                    "{glyph:?} outer contour {index} is not clockwise in font units"
                ),
            }
        }
    }
    let disc = shape_of(Glyph::Disc(Mark::Circle));
    assert_eq!(disc.depths(), vec![0, 1, 2]);
    let keycap = shape_of(Glyph::Keycap(48));
    assert_eq!(keycap.depths(), vec![0, 1]);
}

#[test]
fn nonzero_fills_exactly_like_even_odd() {
    for glyph in catalogue() {
        let shape = shape_of(glyph);
        let lines = shape.polylines();
        let mut y = 0.31;
        while y < shape.height {
            let mut x = 0.17;
            while x < shape.width {
                let winding = shape::winding(&lines, [x, y]);
                let parity = lines
                    .iter()
                    .filter(|line| shape::contains(line, [x, y]))
                    .count()
                    % 2
                    == 1;
                assert_eq!(winding != 0, parity, "{glyph:?} at {x} {y}");
                assert!(winding.abs() <= 1, "{glyph:?} at {x} {y}");
                x += 0.5;
            }
            y += 0.5;
        }
    }
}

fn every_pad_control() -> Vec<Control> {
    let mut controls: Vec<Control> = Button::ALL.into_iter().map(Control::Button).collect();
    controls.push(Control::Stick(Stick::Left));
    controls.push(Control::Stick(Stick::Right));
    controls.push(Control::DPad);
    controls
}

#[test]
fn every_control_of_both_sets_maps_to_an_atlas_glyph() {
    let atlas = GlyphAtlas::build().unwrap();
    for style in GlyphStyle::ALL {
        for control in every_pad_control() {
            let glyph = glyph_for(style, control);
            assert!(atlas.get(glyph).is_some(), "{style:?} {control:?}");
        }
    }
}

#[test]
fn the_playstation_set_has_shapes_and_numbered_shoulders() {
    let face = |button| glyph_for(GlyphStyle::PlayStation, Control::Button(button));
    assert_eq!(face(Button::South), Glyph::Disc(Mark::Cross));
    assert_eq!(face(Button::East), Glyph::Disc(Mark::Circle));
    assert_eq!(face(Button::West), Glyph::Disc(Mark::Square));
    assert_eq!(face(Button::North), Glyph::Disc(Mark::Triangle));
    assert_eq!(face(Button::LeftBumper), Glyph::Bumper("L1"));
    assert_eq!(face(Button::LeftTrigger), Glyph::Trigger("L2"));
    assert_eq!(face(Button::RightBumper), Glyph::Bumper("R1"));
    assert_eq!(face(Button::RightTrigger), Glyph::Trigger("R2"));
    assert_eq!(face(Button::LeftStick), Glyph::Disc(Mark::Text("L3")));
    assert_eq!(face(Button::RightStick), Glyph::Disc(Mark::Text("R3")));
    assert_eq!(face(Button::Touchpad), Glyph::Touchpad);
    assert_eq!(face(Button::Menu), Glyph::Pill(Mark::Bars));
    assert_eq!(face(Button::View), Glyph::Pill(Mark::Rays));
}

#[test]
fn the_standard_set_has_letters_and_lettered_shoulders() {
    let face = |button| glyph_for(GlyphStyle::Standard, Control::Button(button));
    assert_eq!(face(Button::South), Glyph::Disc(Mark::Text("A")));
    assert_eq!(face(Button::East), Glyph::Disc(Mark::Text("B")));
    assert_eq!(face(Button::West), Glyph::Disc(Mark::Text("X")));
    assert_eq!(face(Button::North), Glyph::Disc(Mark::Text("Y")));
    assert_eq!(face(Button::LeftBumper), Glyph::Bumper("LB"));
    assert_eq!(face(Button::LeftTrigger), Glyph::Trigger("LT"));
    assert_eq!(face(Button::RightBumper), Glyph::Bumper("RB"));
    assert_eq!(face(Button::RightTrigger), Glyph::Trigger("RT"));
    assert_eq!(face(Button::LeftStick), Glyph::Disc(Mark::Text("LS")));
    assert_eq!(face(Button::RightStick), Glyph::Disc(Mark::Text("RS")));
    assert_eq!(face(Button::Menu), Glyph::Disc(Mark::Bars));
    assert_eq!(face(Button::View), Glyph::Disc(Mark::Windows));
}

#[test]
fn the_two_sets_differ_on_every_face_and_shoulder() {
    for button in [
        Button::South,
        Button::East,
        Button::West,
        Button::North,
        Button::LeftBumper,
        Button::RightBumper,
        Button::LeftTrigger,
        Button::RightTrigger,
        Button::LeftStick,
        Button::RightStick,
        Button::Menu,
        Button::View,
    ] {
        let control = Control::Button(button);
        assert_ne!(
            glyph_for(GlyphStyle::PlayStation, control),
            glyph_for(GlyphStyle::Standard, control),
            "{button:?}"
        );
    }
}

#[test]
fn keys_and_mouse_ignore_the_style() {
    for control in [
        Control::Key(Key::Space),
        Control::Key(Key::Up),
        Control::Mouse(MouseButton::Left),
        Control::MouseMove,
    ] {
        assert_eq!(
            glyph_for(GlyphStyle::PlayStation, control),
            glyph_for(GlyphStyle::Standard, control)
        );
    }
}
