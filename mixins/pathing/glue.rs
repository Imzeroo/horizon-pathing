// Pathing in the game (not in the Cargo library): the game's blocks read
// through `WorldView`, and the pathfinder's controls applied the way the old
// `horizon-ctl goto` did it, by holding keys and turning the view.
//
// This reads fields of `Game` that are not hook points (the world, the
// keys, the view angle, the screen), which a module inside `crate::game` may
// do: it is why the mod is built on the commit it was tested on.
use crate::game::pathing as m;
use crate::game::{Game, Screen};
use horizon_client::blocks::Fluid;
use std::cell::{Cell, RefCell};
use winit::keyboard::KeyCode;

const MOVE_KEYS: [KeyCode; 6] = [
    KeyCode::KeyW,
    KeyCode::KeyS,
    KeyCode::KeyA,
    KeyCode::KeyD,
    KeyCode::Space,
    KeyCode::ControlLeft,
];

thread_local! {
    static PATHER: RefCell<Option<m::Pather>> = const { RefCell::new(None) };
    static OPTIONS: RefCell<Option<m::Options>> = const { RefCell::new(None) };
    static SPRINT: Cell<bool> = const { Cell::new(true) };
}

struct GameWorld<'g, 'a>(&'g Game<'a>);

impl m::WorldView for GameWorld<'_, '_> {
    fn cell(&self, p: m::Pos) -> m::Cell {
        let g = self.0;
        if !g.world.chunks.contains_key(&(p[0] >> 4, p[2] >> 4)) {
            return m::Cell::Unloaded;
        }
        let facts = g.res.facts().get(g.world.block(p[0], p[1], p[2]));
        let liquid = match facts.fluid {
            Fluid::None => m::Liquid::None,
            Fluid::Water => m::Liquid::Water,
            Fluid::Lava => m::Liquid::Lava,
        };
        m::classify(&facts.name, facts.full_collision || facts.legacy_solid, liquid)
    }
}

fn release(game: &mut Game<'_>) {
    for k in MOVE_KEYS {
        game.keys.remove(&k);
    }
}

fn apply(game: &mut Game<'_>, c: m::Controls) {
    release(game);
    if c.forward {
        game.keys.insert(KeyCode::KeyW);
    }
    if c.back {
        game.keys.insert(KeyCode::KeyS);
    }
    if c.left {
        game.keys.insert(KeyCode::KeyA);
    }
    if c.right {
        game.keys.insert(KeyCode::KeyD);
    }
    if c.sprint {
        game.keys.insert(KeyCode::ControlLeft);
    }
    if c.jump {
        game.keys.insert(KeyCode::Space);
    }
    // Only `/path look on` turns the camera; otherwise the keys do the steering.
    if let Some(yaw) = c.yaw {
        game.yaw = yaw;
    }
}

fn options() -> m::Options {
    OPTIONS.with(|o| o.borrow_mut().get_or_insert_with(m::Options::default).clone())
}

fn goal_text(goal: m::Goal) -> String {
    match goal {
        m::Goal::Exact(p) => format!("{} {} {}", p[0], p[1], p[2]),
        m::Goal::Near(p, r) => format!("{} {} {} (within {r})", p[0], p[1], p[2]),
        m::Goal::Column(x, z) => format!("{x} ~ {z}"),
    }
}

/// Once a tick, in the window's game: plans a little, or takes a step.
pub(super) fn tick(game: &mut Game<'_>) {
    if !game.native_on(m::ID) || !game.native_window() {
        return;
    }
    let Some(mut pather) = PATHER.with(|p| p.borrow_mut().take()) else {
        return;
    };
    let Some(player) = game.native_player() else {
        release(game);
        return;
    };
    if game.screen != Screen::Playing {
        release(game);
        game.native_chat("Path stopped: a screen opened");
        return;
    }
    let pos = [player.x, player.y, player.z];
    let on_ground = game.player.as_ref().is_some_and(|p| p.ground.on_ground);
    let world = GameWorld(&*game);
    let in_water = m::WorldView::cell(&world, m::node_at(pos)) == m::Cell::Water;
    let view_yaw = if options().turn_camera { None } else { Some(player.yaw) };
    let tick = pather.tick(&world, pos, on_ground, in_water, view_yaw);
    if let Some(note) = pather.take_note() {
        game.native_chat(&note);
    }
    match tick {
        m::Tick::Controls(c) => {
            apply(game, c);
        }
        m::Tick::Idle => {
            release(game);
        }
        m::Tick::Arrived => {
            release(game);
            game.native_chat("Arrived");
            return;
        }
        m::Tick::Failed => {
            release(game);
            game.native_chat(&format!(
                "No way found to {}: is it loaded and reachable? (/path debug on shows each plan)",
                goal_text(pather.goal())
            ));
            return;
        }
    }
    PATHER.with(|p| *p.borrow_mut() = Some(pather));
}

/// A chat command: `/goto` and `/path`.
pub(super) fn command(game: &mut Game<'_>, command: &str, ran: &mut bool) {
    if !game.native_on(m::ID) {
        return;
    }
    let here = game.native_player().map(|p| [p.x, p.y, p.z]);
    let Some(parsed) = m::parse_command(command, here.unwrap_or([0.0; 3])) else {
        return;
    };
    *ran = true;
    let cmd = match parsed {
        Ok(cmd) => cmd,
        Err(message) => {
            game.native_chat(&message);
            return;
        }
    };
    match cmd {
        m::Cmd::Goto(goal) => {
            if here.is_none() {
                game.native_chat("Join a world first");
                return;
            }
            let mut pather = m::Pather::new(goal, options());
            pather.sprint = SPRINT.with(Cell::get);
            PATHER.with(|p| *p.borrow_mut() = Some(pather));
            game.native_chat(&format!("Going to {}", goal_text(goal)));
        }
        m::Cmd::Stop => {
            let was = PATHER.with(|p| p.borrow_mut().take()).is_some();
            release(game);
            game.native_chat(if was { "Path stopped" } else { "No path to stop" });
        }
        m::Cmd::Status => {
            let line = PATHER.with(|p| p.borrow().as_ref().map(|p| p.describe()));
            game.native_chat(&line.unwrap_or_else(|| "Not going anywhere".to_string()));
        }
        m::Cmd::Set(setting) => {
            let mut opts = options();
            let mut sprint = SPRINT.with(Cell::get);
            m::apply_setting(&mut opts, &mut sprint, &setting);
            OPTIONS.with(|o| *o.borrow_mut() = Some(opts));
            SPRINT.with(|s| s.set(sprint));
            game.native_chat("Set (it applies to the next /goto)");
        }
        m::Cmd::Help => {
            for line in m::HELP {
                game.native_chat(line);
            }
        }
    }
}

/// The commands the chat box should know.
pub(super) fn commands(game: &Game<'_>, commands: &mut Vec<&'static str>) {
    if game.native_on(m::ID) {
        commands.push("goto");
        commands.push("path");
    }
}