const RENDER_NOTES: &[&str] = &[
    "clip masters",
    "still --derive",
    "--samples N",
    "asset recipes",
    "a logo,",
    "an app renders",
];

const SCENE_NOTES: &[&str] = &[".pitofx is"];

pub fn wants(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--help" || arg == "-h")
}

pub fn asked(args: &[String]) -> Vec<String> {
    match args.split_first() {
        Some((first, rest)) if first == "help" && !rest.is_empty() => {
            let mut words = rest.to_vec();
            words.push("--help".to_string());
            words
        }
        _ => args.to_vec(),
    }
}

fn forms<'a>(usage: &'a str, topic: &str) -> Vec<&'a str> {
    let head = format!("pfx {topic}");
    usage
        .lines()
        .map(str::trim)
        .filter(|line| {
            line.strip_prefix(&head)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '|']))
        })
        .collect()
}

fn topic(usage: &str, words: &[&str]) -> Option<String> {
    let mut words: Vec<&str> = words
        .iter()
        .copied()
        .take_while(|word| !word.starts_with('-'))
        .collect();
    while !words.is_empty() {
        let topic = words.join(" ");
        if !forms(usage, &topic).is_empty() {
            return Some(topic);
        }
        words.pop();
    }
    None
}

fn notes<'a>(usage: &'a str, topic: &str) -> Vec<&'a str> {
    let verb = topic.split(' ').next().unwrap_or(topic);
    let extra: &[&str] = match verb {
        "render" => RENDER_NOTES,
        "scene" => SCENE_NOTES,
        _ => &[],
    };
    usage
        .lines()
        .filter(|line| !line.starts_with(' ') && !line.starts_with("usage:"))
        .filter(|line| {
            line.strip_prefix(topic)
                .or_else(|| line.strip_prefix(verb).filter(|_| topic == verb))
                .is_some_and(|rest| rest.starts_with(' '))
                || extra.iter().any(|start| line.starts_with(start))
        })
        .collect()
}

fn value(tokens: &[&str]) -> Option<String> {
    let first = tokens.first().filter(|next| {
        !next.starts_with('-') && **next != "|" && **next != "..." && !next.starts_with("pfx")
    })?;
    if !first.starts_with('<') || first.contains('>') {
        return Some(first.trim_end_matches("...").to_string());
    }
    let end = tokens.iter().position(|token| token.contains('>'))?;
    Some(tokens[..=end].join(" "))
}

fn options(lines: &[&str], extra: &[&str]) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut add = |flag: &str, value: Option<&str>| {
        let flag = flag.trim_end_matches("...");
        if flag == "--help"
            || found
                .iter()
                .any(|seen| seen.split(' ').next() == Some(flag))
        {
            return;
        }
        found.push(match value {
            Some(value) => format!("{flag} {value}"),
            None => flag.to_string(),
        });
    };
    for line in lines {
        let flat = line.replace(['[', ']', '(', ')'], " ");
        let tokens: Vec<&str> = flat.split_whitespace().collect();
        for (index, token) in tokens.iter().enumerate() {
            let flags: Vec<&str> = token.split('|').collect();
            if !flags.iter().all(|flag| flag.starts_with("--")) {
                continue;
            }
            let value = (flags.len() == 1)
                .then(|| value(&tokens[index + 1..]))
                .flatten();
            for flag in flags {
                add(flag, value.as_deref());
            }
        }
    }
    for flag in extra {
        add(flag, None);
    }
    found
}

pub fn verb(usage: &str, args: &[String], extra: &[&str]) -> Option<String> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let topic = topic(usage, &words)?;
    let lines = forms(usage, &topic);
    let mut text = String::new();
    for note in notes(usage, &topic) {
        text.push_str(note);
        text.push('\n');
    }
    if !text.is_empty() {
        text.push('\n');
    }
    for (index, line) in lines.iter().enumerate() {
        text.push_str(if index == 0 { "Usage: " } else { "       " });
        text.push_str(line);
        text.push('\n');
    }
    text.push_str("\nOptions:\n");
    for option in options(&lines, extra) {
        text.push_str(&format!("      {option}\n"));
    }
    text.push_str("  -h, --help  Print help");
    Some(text)
}
