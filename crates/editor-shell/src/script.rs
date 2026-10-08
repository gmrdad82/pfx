use crate::{Error, Mods, Script, Step};

fn point(number: usize, args: &[&str]) -> Result<[f32; 2], Error> {
    let [x, y] = args else {
        return Err(Error(format!(
            "line {number}: wants two coordinates, x and y"
        )));
    };
    let parse = |text: &str| match text.parse::<f32>() {
        Ok(value) if value.is_finite() && value >= 0.0 => Ok(value),
        _ => Err(Error(format!(
            "line {number}: {text} is not a coordinate in points"
        ))),
    };
    Ok([parse(x)?, parse(y)?])
}

fn modifier(mods: &mut Mods, word: &str) -> bool {
    match word {
        "ctrl" => mods.ctrl = true,
        "shift" => mods.shift = true,
        "alt" => mods.alt = true,
        _ => return false,
    }
    true
}

fn held(number: usize, command: &str, words: &[&str]) -> Result<Mods, Error> {
    let mut mods = Mods::default();
    for word in words {
        if !modifier(&mut mods, word) {
            return Err(Error(format!(
                "line {number}: {word} is not held in {command}; write ctrl, shift or alt"
            )));
        }
    }
    Ok(mods)
}

fn press(number: usize, args: &[&str]) -> Result<Step, Error> {
    if args.len() < 2 {
        return Err(Error(format!(
            "line {number}: press wants x y, then ctrl, shift or alt if held"
        )));
    }
    let (at, words) = args.split_at(2);
    Ok(Step::Press {
        at: point(number, at)?,
        mods: held(number, "a press", words)?,
    })
}

fn release(number: usize, args: &[&str]) -> Result<Step, Error> {
    if args.len() != 2 {
        return Err(Error(format!("line {number}: release wants x y")));
    }
    Ok(Step::Release(point(number, args)?))
}

fn key(number: usize, args: &[&str]) -> Result<Step, Error> {
    let [chord] = args else {
        return Err(Error(format!("line {number}: key wants one key name")));
    };
    let mut mods = Mods::default();
    let mut name = *chord;
    loop {
        let lower = name.to_ascii_lowercase();
        let Some((word, rest)) = lower.split_once('+') else {
            break;
        };
        if rest.is_empty() || !modifier(&mut mods, word) {
            break;
        }
        name = &name[word.len() + 1..];
    }
    if egui::Key::from_name(name).is_some() {
        Ok(Step::Key(name.to_string(), mods))
    } else {
        Err(Error(format!("line {number}: no key is named {name}")))
    }
}

fn drag(number: usize, args: &[&str]) -> Result<Step, Error> {
    if args.len() < 4 {
        return Err(Error(format!(
            "line {number}: drag wants x y to x y, then middle, ctrl, shift or alt if held"
        )));
    }
    let (from, rest) = args.split_at(2);
    let (to, words) = rest.split_at(2);
    let mut middle = false;
    let mut mods = Mods::default();
    for word in words {
        if *word == "middle" {
            middle = true;
        } else if !modifier(&mut mods, word) {
            return Err(Error(format!(
                "line {number}: {word} is not held in a drag; write middle, ctrl, shift or alt"
            )));
        }
    }
    Ok(Step::Drag {
        from: point(number, from)?,
        to: point(number, to)?,
        middle,
        mods,
    })
}

fn scroll(number: usize, args: &[&str]) -> Result<Step, Error> {
    let [x, y, lines] = args else {
        return Err(Error(format!(
            "line {number}: scroll wants x, y and the lines, positive to come nearer"
        )));
    };
    let lines = match lines.parse::<f32>() {
        Ok(value) if value.is_finite() => value,
        _ => {
            return Err(Error(format!(
                "line {number}: {lines} is not a number of lines"
            )));
        }
    };
    Ok(Step::Scroll {
        at: point(number, &[x, y])?,
        lines,
    })
}

pub fn parse(text: &str) -> Result<Script, Error> {
    let mut steps = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (command, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim_start();
        let args: Vec<&str> = rest.split_whitespace().collect();
        let step = match command {
            "key" => key(number, &args)?,
            "text" if rest.is_empty() => {
                return Err(Error(format!(
                    "line {number}: text wants something to type"
                )));
            }
            "text" => Step::Text(rest.to_string()),
            "click" => Step::Click(point(number, &args)?),
            "move" => Step::Move(point(number, &args)?),
            "press" => press(number, &args)?,
            "release" => release(number, &args)?,
            "drag" => drag(number, &args)?,
            "scroll" => scroll(number, &args)?,
            "frame" if args.is_empty() => Step::Frame,
            "frame" => return Err(Error(format!("line {number}: frame takes nothing"))),
            other => {
                return Err(Error(format!(
                    "line {number}: {other} is not a step; the steps are key, text, click, move, press, release, drag, scroll and frame"
                )));
            }
        };
        steps.push(step);
    }
    Ok(Script { steps })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_step_parses() {
        let script = parse(
            "# focus the outliner\nkey b\nkey 1\n\ntext  two words \nclick 12 40.5\nmove 0 0\nframe\n",
        )
        .unwrap();
        assert_eq!(
            script.steps,
            vec![
                Step::Key("b".into(), Mods::default()),
                Step::Key("1".into(), Mods::default()),
                Step::Text("two words".into()),
                Step::Click([12.0, 40.5]),
                Step::Move([0.0, 0.0]),
                Step::Frame,
            ]
        );
    }

    #[test]
    fn an_empty_script_has_no_steps() {
        assert_eq!(parse("").unwrap(), Script::default());
        assert_eq!(parse("\n  \n# nothing\n").unwrap(), Script::default());
    }

    #[test]
    fn mistakes_name_their_line() {
        for (text, line) in [
            ("frame\nkey nope", "line 2"),
            ("key", "line 1"),
            ("key a b", "line 1"),
            ("text", "line 1"),
            ("frame\n\nclick 1", "line 3"),
            ("click 1 x", "line 1"),
            ("move -1 2", "line 1"),
            ("move 1 inf", "line 1"),
            ("frame now", "line 1"),
            ("press 1", "line 1"),
            ("press 1 2 meta", "line 1"),
            ("release 1 2 3", "line 1"),
            ("key ctrl+nope", "line 1"),
        ] {
            let error = parse(text).unwrap_err();
            assert!(error.0.starts_with(line), "{text:?}: {error}");
        }
    }

    #[test]
    fn keys_use_egui_names() {
        let script = parse("key Escape\nkey ArrowDown\nkey Enter\nkey /").unwrap();
        assert_eq!(script.steps.len(), 4);
    }

    #[test]
    fn drags_and_scrolls_parse() {
        let script =
            parse("drag 10 20 30 40\ndrag 1 2 3 4 middle\ndrag 1 2 3 4 shift\nscroll 5 6 -2.5")
                .unwrap();
        assert_eq!(
            script.steps,
            vec![
                Step::Drag {
                    from: [10.0, 20.0],
                    to: [30.0, 40.0],
                    middle: false,
                    mods: Mods::default(),
                },
                Step::Drag {
                    from: [1.0, 2.0],
                    to: [3.0, 4.0],
                    middle: true,
                    mods: Mods::default(),
                },
                Step::Drag {
                    from: [1.0, 2.0],
                    to: [3.0, 4.0],
                    middle: false,
                    mods: Mods {
                        shift: true,
                        ..Mods::default()
                    },
                },
                Step::Scroll {
                    at: [5.0, 6.0],
                    lines: -2.5,
                },
            ]
        );
        for text in [
            "drag 1 2 3",
            "drag 1 2 3 4 left",
            "scroll 1 2",
            "scroll 1 2 far",
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn presses_releases_and_chords_parse() {
        let ctrl = Mods {
            ctrl: true,
            ..Mods::default()
        };
        let script = parse(
            "press 10 20\npress 1 2 ctrl\nrelease 3 4\nkey ctrl+z\nkey Ctrl+Shift+z\nkey Escape\ndrag 1 2 3 4 ctrl alt",
        )
        .unwrap();
        assert_eq!(
            script.steps,
            vec![
                Step::Press {
                    at: [10.0, 20.0],
                    mods: Mods::default(),
                },
                Step::Press {
                    at: [1.0, 2.0],
                    mods: ctrl,
                },
                Step::Release([3.0, 4.0]),
                Step::Key("z".into(), ctrl),
                Step::Key(
                    "z".into(),
                    Mods {
                        ctrl: true,
                        shift: true,
                        alt: false,
                    }
                ),
                Step::Key("Escape".into(), Mods::default()),
                Step::Drag {
                    from: [1.0, 2.0],
                    to: [3.0, 4.0],
                    middle: false,
                    mods: Mods {
                        ctrl: true,
                        shift: false,
                        alt: true,
                    },
                },
            ]
        );
    }
}
