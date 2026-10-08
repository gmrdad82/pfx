use std::cell::RefCell;
use std::rc::Rc;

use pfx_core::clock::Tick;
use pfx_input::Input;
use pfx_load::scene::BodyShape;

use super::{FRAME, Folder, headless};
use crate::{
    Game, Layers, ObjectId, Options, Phase, PlaySession, Query, QueryError, TriggerEvent, World,
    WorldEvent,
};

const PROJECT: &str = r#"[project]

[layers]
world = ["world", "player", "enemy"]
player = ["world", "pickup"]
enemy = ["world"]
pickup = ["player"]
gem = ["gem"]
ghost = []
volume = []
"#;

const OBJECTS: &str = r#"
[physics]
gravity = [0.0, 0.0, 0.0]

[[object]]
name = "gate"
mesh = "block"
at = [3.0, 1.0, -6.0]
scale = 0.1

[trigger.gate]
shape = "box"
half = [1.0, 1.0, 1.0]
layer = "volume"
mask = ["ghost"]

[[object]]
name = "wisp"
mesh = "block"
at = [0.0, 1.0, -6.0]
scale = 0.5

[body.wisp]
layer = "ghost"
shape = "sphere"
velocity = [3.0, 0.0, 0.0]

[[object]]
name = "shield"
mesh = "block"
at = [2.0, 1.0, 0.0]
scale = 1.0

[body.shield]
layer = "world"
kind = "fixed"

[[object]]
name = "target"
mesh = "block"
at = [4.0, 1.0, 0.0]
scale = 2.0

[body.target]
layer = "enemy"
kind = "fixed"

[[object]]
name = "zone"
mesh = "block"
at = [3.0, 1.0, 5.0]
scale = 0.1

[trigger.zone]
shape = "box"
half = [1.0, 1.0, 1.0]
layer = "volume"
mask = ["player"]

[[object]]
name = "echo"
mesh = "block"
at = [3.0, 1.0, 5.0]
scale = 0.1

[trigger.echo]
shape = "sphere"
radius = 0.4
layer = "volume"
mask = ["volume"]

[[object]]
name = "gem"
mesh = "block"
at = [3.0, 1.0, 5.0]
scale = 1.0

[body.gem]
layer = "gem"
kind = "fixed"

[[object]]
name = "probe"
mesh = "block"
at = [0.0, 1.0, 5.0]
scale = 0.5

[body.probe]
layer = "player"
shape = "sphere"
velocity = [3.0, 0.0, 0.0]

[trigger.probe]
shape = "sphere"
radius = 0.5
layer = "volume"
mask = ["gem"]

[[object]]
name = "decoy"
mesh = "block"
at = [0.0, 1.0, 5.6]
scale = 0.5

[body.decoy]
layer = "enemy"
shape = "sphere"
velocity = [3.0, 0.0, 0.0]

[[object]]
name = "anvil"
mesh = "block"
at = [2.0, 1.0, -3.0]
scale = 1.0

[body.anvil]
layer = "world"
kind = "fixed"

[[object]]
name = "bolt"
mesh = "block"
at = [0.0, 1.0, -3.0]
scale = 0.5

[body.bolt]
layer = "player"
shape = "sphere"
velocity = [4.0, 0.0, 0.0]
"#;

type Log = Rc<RefCell<Vec<(u64, WorldEvent)>>>;
type Polled = Rc<RefCell<Vec<(u64, Vec<WorldEvent>)>>>;

struct Recorder {
    log: Log,
    polled: Polled,
}

impl Recorder {
    fn new() -> (Self, Log, Polled) {
        let log = Log::default();
        let polled = Rc::new(RefCell::new(Vec::new()));
        (
            Self {
                log: log.clone(),
                polled: polled.clone(),
            },
            log,
            polled,
        )
    }
}

impl Game for Recorder {
    fn start(&mut self, _world: &mut World) {}

    fn tick(&mut self, world: &mut World, _tick: &Tick, _input: &Input) {
        self.polled
            .borrow_mut()
            .push((world.ticks(), world.events().to_vec()));
    }

    fn events(&mut self, world: &mut World, events: &[WorldEvent]) {
        assert_eq!(events, world.events());
        let mut log = self.log.borrow_mut();
        for event in events {
            log.push((world.ticks(), *event));
        }
    }
}

fn arena(name: &str) -> Folder {
    let folder = Folder::new(name, OBJECTS);
    std::fs::write(folder.root.join("project.toml"), PROJECT).unwrap();
    folder
}

fn play(folder: &Folder) -> (PlaySession, Log, Polled) {
    let (game, log, polled) = Recorder::new();
    let session = headless(&folder.scene(), Box::new(game), Options::default());
    (session, log, polled)
}

fn taken(log: &Log) -> Vec<(u64, WorldEvent)> {
    log.borrow().clone()
}

fn ready(folder: &Folder) -> PlaySession {
    let (session, _, _) = play(folder);
    session
}

fn run(session: &mut PlaySession, ticks: u64) {
    while session.world().ticks() < ticks {
        session.advance(FRAME);
    }
}

fn id(world: &World, name: &str) -> ObjectId {
    world.object(name).unwrap()
}

fn triggers(log: &Log, world: &World, trigger: &str, other: &str) -> Vec<(u64, Phase)> {
    let (trigger, other) = (id(world, trigger), id(world, other));
    log.borrow()
        .iter()
        .filter_map(|(tick, event)| match event {
            WorldEvent::Trigger(TriggerEvent {
                phase,
                trigger: t,
                other: o,
            }) if *t == trigger && *o == other => Some((*tick, *phase)),
            _ => None,
        })
        .collect()
}

fn near(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-3)
}

#[test]
fn a_raycast_names_the_object_it_hits_with_its_point_normal_and_distance() {
    let folder = arena("query ray");
    let session = ready(&folder);
    let world = session.world();
    let hit = world
        .raycast([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], 20.0, &Query::all())
        .unwrap()
        .unwrap();
    assert_eq!(hit.name.as_deref(), Some("shield"));
    assert_eq!(hit.object, world.object("shield"));
    assert!(!hit.trigger);
    assert!((hit.distance - 1.5).abs() < 1e-3, "{hit:?}");
    assert!(near(hit.point, [1.5, 1.0, 0.0]), "{hit:?}");
    assert!(near(hit.normal, [-1.0, 0.0, 0.0]), "{hit:?}");
    let short = world
        .raycast([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], 1.0, &Query::all())
        .unwrap();
    assert_eq!(short, None);
    let none = world
        .raycast([0.0, 1.0, 0.0], [0.0, 0.0, 0.0], 5.0, &Query::all())
        .unwrap();
    assert_eq!(none, None);
}

#[test]
fn queries_filter_by_named_layers_and_exclusions() {
    let folder = arena("query layers");
    let session = ready(&folder);
    let world = session.world();
    let ray = |query: Query| {
        world
            .raycast([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], 20.0, &query)
            .unwrap()
            .and_then(|hit| hit.name)
    };
    assert_eq!(ray(Query::all()).as_deref(), Some("shield"));
    assert_eq!(ray(Query::layers(&["world"])).as_deref(), Some("shield"));
    assert_eq!(ray(Query::layers(&["enemy"])).as_deref(), Some("target"));
    assert_eq!(
        ray(Query::layers(&["world", "enemy"])).as_deref(),
        Some("shield")
    );
    assert_eq!(ray(Query::layers(&["player"])), None);
    assert_eq!(ray(Query::layers::<&str>(&[])), None);
    assert_eq!(
        ray(Query::all().excluding("shield")).as_deref(),
        Some("target")
    );
    let bit = world.layers().bit("enemy").unwrap();
    assert_eq!(ray(Query::all().mask(bit)).as_deref(), Some("target"));
    assert_eq!(world.layers().names_of(bit), vec!["enemy"]);
    assert_eq!(world.layers().collides("player", "world"), Some(true));
    assert_eq!(world.layers().collides("player", "enemy"), Some(false));
    assert_eq!(world.layers().collides("pickup", "world"), Some(false));
}

#[test]
fn a_query_with_an_unknown_layer_or_object_is_refused_by_name() {
    let folder = arena("query refused");
    let session = ready(&folder);
    let world = session.world();
    let error = world
        .raycast([0.0; 3], [1.0, 0.0, 0.0], 5.0, &Query::layers(&["walls"]))
        .unwrap_err();
    let QueryError::Layer(message) = &error else {
        panic!("{error:?}");
    };
    assert!(message.contains("\"walls\""), "{message}");
    assert!(
        message.contains("enemy, gem, ghost, pickup, player, volume, world"),
        "{message}"
    );
    let error = world
        .overlaps_point([0.0; 3], &Query::all().excluding("nobody"))
        .unwrap_err();
    assert_eq!(error, QueryError::Object("nobody".into()));
    let error = world
        .shape_cast(
            BodyShape::Sphere { radius: 0.0 },
            [0.0; 3],
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
            5.0,
            &Query::all(),
        )
        .unwrap_err();
    assert_eq!(error, QueryError::Shape);
}

#[test]
fn a_shape_cast_stops_at_the_first_object_in_its_way() {
    let folder = arena("query cast");
    let session = ready(&folder);
    let world = session.world();
    let sphere = BodyShape::Sphere { radius: 0.25 };
    let identity = [0.0, 0.0, 0.0, 1.0];
    let hit = world
        .shape_cast(
            sphere,
            [0.0, 1.0, 0.0],
            identity,
            [1.0, 0.0, 0.0],
            20.0,
            &Query::all(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(hit.name.as_deref(), Some("shield"));
    assert!((hit.distance - 1.25).abs() < 1e-3, "{hit:?}");
    assert!(near(hit.normal, [-1.0, 0.0, 0.0]), "{hit:?}");
    let past = world
        .shape_cast(
            sphere,
            [0.0, 1.0, 0.0],
            identity,
            [1.0, 0.0, 0.0],
            20.0,
            &Query::layers(&["enemy"]),
        )
        .unwrap()
        .unwrap();
    assert_eq!(past.name.as_deref(), Some("target"));
    assert!((past.distance - 2.75).abs() < 1e-3, "{past:?}");
    let miss = world
        .shape_cast(
            sphere,
            [0.0, 1.0, 0.0],
            identity,
            [-1.0, 0.0, 0.0],
            20.0,
            &Query::all(),
        )
        .unwrap();
    assert_eq!(miss, None);
}

#[test]
fn overlaps_list_the_objects_in_a_shape_in_scene_order_and_filter_by_layer() {
    let folder = arena("query overlaps");
    let session = ready(&folder);
    let world = session.world();
    let wide = BodyShape::Box {
        half: [3.0, 0.4, 0.4],
    };
    let identity = [0.0, 0.0, 0.0, 1.0];
    let names = |query: Query| -> Vec<String> {
        world
            .overlaps(wide, [3.0, 1.0, 0.0], identity, &query)
            .unwrap()
            .into_iter()
            .map(|overlap| overlap.name)
            .collect()
    };
    assert_eq!(names(Query::all()), vec!["shield", "target"]);
    assert_eq!(names(Query::layers(&["world"])), vec!["shield"]);
    assert_eq!(names(Query::layers(&["enemy"])), vec!["target"]);
    assert!(names(Query::layers(&["pickup"])).is_empty());
    let at_point = world
        .overlaps_point([2.2, 1.0, 0.0], &Query::all())
        .unwrap();
    assert_eq!(at_point.len(), 1);
    assert_eq!(at_point[0].name, "shield");
    let ids: Vec<usize> = world
        .overlaps(wide, [3.0, 1.0, 0.0], identity, &Query::all())
        .unwrap()
        .iter()
        .map(|overlap| overlap.object.index())
        .collect();
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]), "{ids:?}");
}

#[test]
fn queries_see_triggers_only_when_asked() {
    let folder = arena("query triggers");
    let session = ready(&folder);
    let world = session.world();
    let across = world
        .raycast([3.0, 1.0, 8.0], [0.0, 0.0, -1.0], 20.0, &Query::all())
        .unwrap();
    assert_eq!(across.and_then(|hit| hit.name).as_deref(), Some("gem"));
    let seen = world
        .overlaps_point([2.6, 1.0, 5.0], &Query::all().with_triggers())
        .unwrap();
    let names: Vec<(&str, bool)> = seen
        .iter()
        .map(|overlap| (overlap.name.as_str(), overlap.trigger))
        .collect();
    assert_eq!(names, vec![("zone", true), ("gem", false)]);
    let plain = world
        .overlaps_point([2.6, 1.0, 5.0], &Query::all())
        .unwrap();
    assert_eq!(plain.len(), 1);
    assert!(world.is_trigger(id(world, "zone")));
    assert!(!world.is_trigger(id(world, "gem")));
}

#[test]
fn a_trigger_reports_enter_stay_and_exit_in_tick_order_and_filters_by_layer() {
    let folder = arena("trigger order");
    let (mut session, log, polled) = play(&folder);
    run(&mut session, 100);
    let world = session.world();
    let seen = triggers(&log, world, "zone", "probe");
    let enter = seen.first().copied().unwrap();
    let exit = seen.last().copied().unwrap();
    assert_eq!(enter.1, Phase::Enter, "{seen:?}");
    assert_eq!(exit.1, Phase::Exit, "{seen:?}");
    assert!((33..=37).contains(&enter.0), "{seen:?}");
    assert!((83..=87).contains(&exit.0), "{seen:?}");
    let stays = &seen[1..seen.len() - 1];
    assert_eq!(stays.len() as u64, exit.0 - enter.0 - 1, "{seen:?}");
    assert!(stays.iter().all(|(_, phase)| *phase == Phase::Stay));
    assert!(
        stays
            .iter()
            .zip(enter.0 + 1..)
            .all(|((tick, _), expected)| *tick == expected),
        "{seen:?}"
    );
    assert!(triggers(&log, world, "zone", "decoy").is_empty());
    assert!(triggers(&log, world, "zone", "bolt").is_empty());
    assert!(world.inside(id(world, "zone")).is_empty());
    let polled = polled.borrow();
    let delivered = log.borrow();
    for (tick, events) in polled.iter().filter(|(_, events)| !events.is_empty()) {
        let before: Vec<WorldEvent> = delivered
            .iter()
            .filter(|(at, _)| *at + 1 == *tick)
            .map(|(_, event)| *event)
            .collect();
        assert_eq!(events, &before, "tick {tick}");
    }
}

#[test]
fn a_trigger_is_inside_between_enter_and_exit_and_a_body_trigger_follows_its_body() {
    let folder = arena("trigger inside");
    let (mut session, log, _) = play(&folder);
    run(&mut session, 60);
    let world = session.world();
    assert_eq!(world.inside(id(world, "zone")), vec![id(world, "probe")]);
    let seen = triggers(&log, world, "probe", "gem");
    assert_eq!(seen.first().map(|(_, phase)| *phase), Some(Phase::Enter));
    assert!(
        seen.first()
            .is_some_and(|(tick, _)| (38..=42).contains(tick)),
        "{seen:?}"
    );
    assert_eq!(world.inside(id(world, "probe")), vec![id(world, "gem")]);
    run(&mut session, 100);
    let world = session.world();
    let seen = triggers(&log, world, "probe", "gem");
    assert_eq!(seen.last().map(|(_, phase)| *phase), Some(Phase::Exit));
    assert!(
        seen.last()
            .is_some_and(|(tick, _)| (78..=82).contains(tick)),
        "{seen:?}"
    );
    assert!(world.inside(id(world, "probe")).is_empty());
}

#[test]
fn a_trigger_object_that_moves_takes_its_trigger_with_it() {
    let folder = arena("trigger moves");
    let (mut session, log, _) = play(&folder);
    let zone = id(session.world(), "zone");
    run(&mut session, 60);
    assert_eq!(
        session.world().inside(zone),
        vec![id(session.world(), "probe")]
    );
    session.world_mut().move_to(zone, [3.0, 1.0, 40.0]);
    run(&mut session, 63);
    let seen = triggers(&log, session.world(), "zone", "probe");
    assert_eq!(
        seen.last().map(|(_, phase)| *phase),
        Some(Phase::Exit),
        "{seen:?}"
    );
    assert!(
        seen.last()
            .is_some_and(|(tick, _)| (61..=63).contains(tick)),
        "{seen:?}"
    );
    assert!(session.world().inside(zone).is_empty());
    session.world_mut().move_to(zone, [3.0, 1.0, 5.0]);
    run(&mut session, 66);
    let seen = triggers(&log, session.world(), "zone", "probe");
    let back = seen.iter().find(|(tick, _)| *tick > 63).copied();
    assert_eq!(back, Some((64, Phase::Enter)), "{seen:?}");
}

#[test]
fn a_contact_arrives_as_an_event_with_its_impulse() {
    let folder = arena("contact events");
    let (mut session, log, _) = play(&folder);
    run(&mut session, 40);
    let world = session.world();
    let (bolt, anvil) = (id(world, "bolt"), id(world, "anvil"));
    let hits: Vec<(u64, f32)> = log
        .borrow()
        .iter()
        .filter_map(|(tick, event)| match event {
            WorldEvent::Contact(contact)
                if (contact.a == bolt && contact.b == anvil)
                    || (contact.a == anvil && contact.b == bolt) =>
            {
                Some((*tick, contact.impulse))
            }
            _ => None,
        })
        .collect();
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].1 > 0.0, "{hits:?}");
    assert!((17..=22).contains(&hits[0].0), "{hits:?}");
}

#[test]
fn the_same_scene_gives_the_same_events_at_the_same_ticks() {
    let first = {
        let folder = arena("events a");
        let (mut session, log, _) = play(&folder);
        run(&mut session, 120);
        taken(&log)
    };
    let second = {
        let folder = arena("events b");
        let (mut session, log, _) = play(&folder);
        for _ in 0..400 {
            session.advance(FRAME * 0.3);
            if session.world().ticks() >= 120 {
                break;
            }
        }
        taken(&log)
    };
    assert!(!first.is_empty());
    let upto = |events: &[(u64, WorldEvent)]| -> Vec<(u64, WorldEvent)> {
        events
            .iter()
            .filter(|(tick, _)| *tick < 120)
            .copied()
            .collect()
    };
    assert_eq!(upto(&first), upto(&second));
    let ticks: Vec<u64> = first
        .iter()
        .filter_map(|(tick, event)| match event {
            WorldEvent::Trigger(event) if event.phase != Phase::Stay => Some(*tick),
            _ => None,
        })
        .collect();
    assert_eq!(ticks, EXPECTED_TICKS);
}

const EXPECTED_TICKS: [u64; 9] = [1, 35, 35, 40, 42, 78, 80, 85, 85];

#[test]
fn layers_come_from_the_scene_in_name_order_and_name_what_collides() {
    let folder = arena("scene layers");
    let session = ready(&folder);
    let layers = session.world().layers();
    assert_eq!(
        layers.names().collect::<Vec<_>>(),
        vec![
            "enemy", "gem", "ghost", "pickup", "player", "volume", "world"
        ]
    );
    assert_eq!(layers.bit("enemy"), Some(1));
    assert_eq!(layers.bit("world"), Some(64));
    assert_eq!(layers.mask(&["world", "gem"]), Ok(66));
    assert_eq!(layers.collision_mask(layers.bit("player").unwrap()), 64 | 8);
    assert_eq!(layers.collision_mask(layers.bit("ghost").unwrap()), 0);
    assert_eq!(layers.collides("world", "player"), Some(true));
    assert_eq!(layers.collides("enemy", "pickup"), Some(false));
    assert_eq!(layers.collides("enemy", "nowhere"), None);
    assert_eq!(layers.names_of(66), vec!["gem", "world"]);
    assert!(layers.mask(&["nowhere"]).unwrap_err().contains("nowhere"));
}

#[test]
fn layers_of_a_scene_are_refused_past_32_or_naming_a_missing_layer() {
    let mut map = std::collections::BTreeMap::new();
    for at in 0..33 {
        map.insert(format!("layer_{at:02}"), Vec::new());
    }
    let error = Layers::from_scene(&map).unwrap_err();
    assert!(error.contains("at most 32"), "{error}");
    map.pop_last();
    let layers = Layers::from_scene(&map).unwrap();
    assert_eq!(layers.len(), 32);
    assert_eq!(layers.bit("layer_31"), Some(1 << 31));
    map.insert("layer_00".into(), vec!["ghost".into()]);
    let error = Layers::from_scene(&map).unwrap_err();
    assert!(error.contains("ghost"), "{error}");
}

#[test]
fn a_scene_naming_an_unknown_layer_is_refused_with_its_file_and_line() {
    let folder = arena("unknown layer");
    let text = std::fs::read_to_string(folder.root.join("room.scene.toml")).unwrap();
    let line = text
        .lines()
        .position(|line| line == "layer = \"world\"")
        .unwrap()
        + 1;
    folder.edit(
        "room.scene.toml",
        "layer = \"world\"",
        "layer = \"nowhere\"",
    );
    let error = pfx_load::scene::Scene::open(folder.root.join("room.scene.toml")).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("nowhere"), "{message}");
    assert!(message.contains("room.scene.toml"), "{message}");
    assert!(message.contains(&format!(":{line}")), "{message}");
}

#[test]
fn bodies_on_layers_that_do_not_collide_pass_through_each_other() {
    let folder = arena("pass through");
    std::fs::write(
        folder.root.join("project.toml"),
        PROJECT
            .replace(
                "world = [\"world\", \"player\", \"enemy\"]",
                "world = [\"world\", \"enemy\"]",
            )
            .replace("player = [\"world\", \"pickup\"]", "player = [\"pickup\"]"),
    )
    .unwrap();
    let (mut session, log, _) = play(&folder);
    run(&mut session, 60);
    let world = session.world();
    let bolt = world.at(id(world, "bolt"));
    assert!(bolt[0] > 3.5, "{bolt:?}");
    assert!(
        log.borrow()
            .iter()
            .all(|(_, event)| !matches!(event, WorldEvent::Contact(_)))
    );
}

#[test]
fn a_trigger_senses_another_trigger_on_a_layer_its_mask_names() {
    let folder = arena("trigger senses trigger");
    let (mut session, log, _) = play(&folder);
    run(&mut session, 100);
    let world = session.world();
    let zone = triggers(&log, world, "echo", "zone");
    assert_eq!(zone.first(), Some(&(1, Phase::Enter)), "{zone:?}");
    assert!(zone.iter().skip(1).all(|(_, phase)| *phase == Phase::Stay));
    let probe = triggers(&log, world, "echo", "probe");
    assert_eq!(
        probe.first().map(|(_, phase)| *phase),
        Some(Phase::Enter),
        "{probe:?}"
    );
    assert_eq!(
        probe.last().map(|(_, phase)| *phase),
        Some(Phase::Exit),
        "{probe:?}"
    );
    assert!(triggers(&log, world, "zone", "echo").is_empty());
}

type Seen = Rc<RefCell<Vec<(u64, Option<String>)>>>;

struct Looker {
    seen: Seen,
}

impl Looker {
    fn look(&self, world: &World) {
        let hit = world
            .raycast([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], 20.0, &Query::all())
            .unwrap()
            .and_then(|hit| hit.name);
        self.seen.borrow_mut().push((world.ticks(), hit));
    }
}

impl Game for Looker {
    fn start(&mut self, world: &mut World) {
        self.look(world);
    }

    fn tick(&mut self, world: &mut World, _tick: &Tick, _input: &Input) {
        self.look(world);
    }
}

#[test]
fn queries_work_from_start_and_in_the_first_tick() {
    let folder = arena("query start");
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut session = headless(
        &folder.scene(),
        Box::new(Looker { seen: seen.clone() }),
        Options::default(),
    );
    run(&mut session, 2);
    let seen = seen.borrow();
    assert_eq!(seen[0], (0, Some("shield".to_string())));
    assert_eq!(seen[1], (1, Some("shield".to_string())));
    assert_eq!(seen[2], (2, Some("shield".to_string())));
}

#[test]
fn queries_see_a_live_edit_of_an_object_at_once() {
    let folder = arena("query edit");
    let mut session = ready(&folder);
    let ray = |session: &PlaySession| {
        session
            .world()
            .raycast([0.0, 1.0, 3.0], [1.0, 0.0, 0.0], 20.0, &Query::all())
            .unwrap()
            .map(|hit| (hit.name.unwrap(), hit.distance))
    };
    assert_eq!(ray(&session), None);
    session
        .edit(crate::Edit::scene(
            pfx_load::scene::Target::Object("shield".into()),
            &["at"],
            [2.0f32, 1.0, 3.0],
        ))
        .unwrap();
    let (name, distance) = ray(&session).unwrap();
    assert_eq!(name, "shield");
    assert!((distance - 1.5).abs() < 1e-3, "{distance}");
}

#[test]
fn a_layer_that_collides_with_nothing_is_seen_by_layer_queries_and_triggers() {
    let folder = arena("ghost layer");
    let (mut session, log, _) = play(&folder);
    let world = session.world();
    assert_eq!(
        world
            .layers()
            .collision_mask(world.layers().bit("ghost").unwrap()),
        0
    );
    let hit = world
        .raycast(
            [-3.0, 1.0, -6.0],
            [1.0, 0.0, 0.0],
            20.0,
            &Query::layers(&["ghost"]),
        )
        .unwrap()
        .unwrap();
    assert_eq!(hit.name.as_deref(), Some("wisp"));
    let miss = world
        .raycast(
            [-3.0, 1.0, -6.0],
            [1.0, 0.0, 0.0],
            20.0,
            &Query::layers(&["world", "player"]),
        )
        .unwrap();
    assert_eq!(miss, None);
    run(&mut session, 100);
    let seen = triggers(&log, session.world(), "gate", "wisp");
    assert_eq!(seen.first(), Some(&(35, Phase::Enter)), "{seen:?}");
    assert_eq!(seen.last(), Some(&(85, Phase::Exit)), "{seen:?}");
}
