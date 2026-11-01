// Pathing: a Baritone-style pathfinder core for Horizon native mods.
//
// This file is both the mod's Cargo library (its tests run with `cargo test`)
// and the module the bundle adds to the game as crate::game::pathing. It has
// no dependency on the game: the world is read through `WorldView`, and the
// result is a set of `Controls` (keys and a yaw) the adapter applies.
//
// Coordinates: a node is the block a player's feet are in. Yaw is Minecraft's:
// 0 faces south (+z), 90 faces west (-x), -90 faces east (+x).

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

#[allow(dead_code)]
pub const ID: &str = "pathing";
#[allow(dead_code)]
pub const VERSION: &str = "0.1.0";

#[allow(dead_code)]
pub type Pos = [i32; 3];

#[allow(dead_code)]
const SQRT2: f64 = std::f64::consts::SQRT_2;

#[allow(dead_code)]
/// What the pathfinder needs to know about one block position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    /// Nothing that stops a body (air, grass, flowers, open doors...).
    Air,
    /// Full collision: can be stood on and blocks the body.
    Solid,
    Water,
    Lava,
    /// Hurts or traps on contact (fire, cactus, magma, berry bushes, webs...).
    Hazard,
    /// Not loaded, so unknown: the search never goes there.
    Unloaded,
}

#[allow(dead_code)]
pub trait WorldView {
    fn cell(&self, p: Pos) -> Cell;
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct Options {
    /// Longest drop the search plans, in blocks. 3 never hurts.
    pub max_fall: i32,
    /// Sprint jumps across gaps of one or two blocks. Off by default: the
    /// follower's jump timing has to be tuned in the game first.
    pub allow_parkour: bool,
    pub allow_diagonal: bool,
    pub allow_swim: bool,
    /// Cost of a step in water, against 1.0 for a step on land.
    pub water_cost: f64,
    /// Most nodes one search expands before it settles for a partial path.
    pub max_nodes: usize,
    /// Turn the camera toward the target and walk forward, instead of leaving
    /// the camera alone and steering with W, A, S and D.
    pub turn_camera: bool,
}

#[allow(dead_code)]
impl Default for Options {
    fn default() -> Self {
        Options {
            max_fall: 3,
            allow_parkour: false,
            allow_diagonal: true,
            allow_swim: true,
            water_cost: 2.0,
            max_nodes: 20_000,
            turn_camera: false,
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Traverse,
    Diagonal,
    Ascend,
    Descend,
    Parkour,
    Swim,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub pos: Pos,
    pub kind: Kind,
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    /// The nodes after the start, in order.
    pub steps: Vec<Step>,
    /// Whether the last step reaches the goal; if not, it is the closest the
    /// search got, and the caller plans again from there.
    pub complete: bool,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Goal {
    /// Stand in this block.
    Exact(Pos),
    /// Get within `radius` blocks (horizontally) and one block (vertically).
    Near(Pos, f64),
    /// Reach this column at any height.
    Column(i32, i32),
}

#[allow(dead_code)]
fn octile(dx: i32, dz: i32) -> f64 {
    let a = dx.abs() as f64;
    let b = dz.abs() as f64;
    a.max(b) + (SQRT2 - 1.0) * a.min(b)
}

#[allow(dead_code)]
impl Goal {
    pub fn reached(&self, p: Pos) -> bool {
        match *self {
            Goal::Exact(g) => p == g,
            Goal::Near(g, r) => {
                let dx = (p[0] - g[0]) as f64;
                let dz = (p[2] - g[2]) as f64;
                (dx * dx + dz * dz).sqrt() <= r.max(0.0) && (p[1] - g[1]).abs() <= 1
            }
            Goal::Column(x, z) => p[0] == x && p[2] == z,
        }
    }

    /// A lower bound on the cost left (never overestimates).
    pub fn heuristic(&self, p: Pos) -> f64 {
        match *self {
            Goal::Exact(g) => octile(p[0] - g[0], p[2] - g[2]),
            Goal::Near(g, r) => (octile(p[0] - g[0], p[2] - g[2]) - r.max(0.0) * SQRT2).max(0.0),
            Goal::Column(x, z) => octile(p[0] - x, p[2] - z),
        }
    }
}

#[allow(dead_code)]
/// The block a player at `pos` (feet, in blocks) is standing in.
pub fn node_at(pos: [f64; 3]) -> Pos {
    [
        pos[0].floor() as i32,
        (pos[1] + 0.01).floor() as i32,
        pos[2].floor() as i32,
    ]
}

#[allow(dead_code)]
fn up(p: Pos, dy: i32) -> Pos {
    [p[0], p[1] + dy, p[2]]
}

#[allow(dead_code)]
fn body_ok<W: WorldView>(w: &W, o: &Options, p: Pos) -> bool {
    match w.cell(p) {
        Cell::Air => true,
        Cell::Water => o.allow_swim,
        _ => false,
    }
}

#[allow(dead_code)]
/// Whether a player can have their feet in `p`: room for feet and head, and
/// something to stand on (or water to float in).
fn valid<W: WorldView>(w: &W, o: &Options, p: Pos) -> bool {
    body_ok(w, o, p)
        && body_ok(w, o, up(p, 1))
        && (w.cell(up(p, -1)) == Cell::Solid || w.cell(p) == Cell::Water)
}

#[allow(dead_code)]
const CARDINALS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
#[allow(dead_code)]
const DIAGONALS: [(i32, i32); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];

#[allow(dead_code)]
fn neighbors<W: WorldView>(w: &W, o: &Options, p: Pos, out: &mut Vec<(Pos, Kind, f64)>) {
    out.clear();
    let [x, y, z] = p;
    let here_water = w.cell(p) == Cell::Water;
    let land = |dest: Pos, base: f64| -> f64 {
        if here_water || w.cell(dest) == Cell::Water {
            base * o.water_cost
        } else {
            base
        }
    };
    let kind_for = |dest: Pos, plain: Kind| -> Kind {
        if here_water || w.cell(dest) == Cell::Water {
            Kind::Swim
        } else {
            plain
        }
    };

    for &(dx, dz) in &CARDINALS {
        let dest = [x + dx, y, z + dz];
        // Traverse.
        if valid(w, o, dest) {
            out.push((dest, kind_for(dest, Kind::Traverse), land(dest, 1.0)));
            continue;
        }
        // Ascend: one block up, with room to jump.
        let hi = [x + dx, y + 1, z + dz];
        if body_ok(w, o, up(p, 2)) && valid(w, o, hi) {
            out.push((hi, kind_for(hi, Kind::Ascend), land(hi, 2.0)));
        }
        // Descend: walk off the edge and fall up to `max_fall` blocks.
        if body_ok(w, o, dest) && body_ok(w, o, up(dest, 1)) {
            let mut k = 1;
            while k <= o.max_fall {
                let f = [x + dx, y - k, z + dz];
                if !body_ok(w, o, f) {
                    break;
                }
                if valid(w, o, f) {
                    out.push((f, kind_for(f, Kind::Descend), land(f, 1.0 + 0.4 * k as f64)));
                    break;
                }
                if matches!(
                    w.cell(up(f, -1)),
                    Cell::Lava | Cell::Hazard | Cell::Unloaded
                ) {
                    break;
                }
                k += 1;
            }
        }
        // Parkour: a flat sprint jump over a gap of one or two blocks.
        if o.allow_parkour && !here_water && body_ok(w, o, up(p, 2)) {
            for gap in 1..=2 {
                let landing = [x + dx * (gap + 1), y, z + dz * (gap + 1)];
                let mut clear = true;
                for i in 1..=gap {
                    let c = [x + dx * i, y, z + dz * i];
                    if !(body_ok(w, o, c)
                        && body_ok(w, o, up(c, 1))
                        && body_ok(w, o, up(c, 2))
                        && w.cell(up(c, -1)) != Cell::Hazard)
                    {
                        clear = false;
                        break;
                    }
                }
                if clear && valid(w, o, landing) && w.cell(landing) != Cell::Water {
                    out.push((landing, Kind::Parkour, 1.0 + 1.2 * (gap + 1) as f64));
                }
            }
        }
    }

    // Swim up and down.
    if o.allow_swim && here_water {
        for dy in [1, -1] {
            let dest = up(p, dy);
            if valid(w, o, dest) && w.cell(dest) == Cell::Water {
                out.push((dest, Kind::Swim, o.water_cost));
            }
        }
    }

    // Diagonals need both side blocks clear, so no corner is cut.
    if o.allow_diagonal {
        for &(dx, dz) in &DIAGONALS {
            let dest = [x + dx, y, z + dz];
            let a = [x + dx, y, z];
            let b = [x, y, z + dz];
            if valid(w, o, dest)
                && body_ok(w, o, a)
                && body_ok(w, o, up(a, 1))
                && body_ok(w, o, b)
                && body_ok(w, o, up(b, 1))
            {
                out.push((dest, kind_for(dest, Kind::Diagonal), land(dest, SQRT2)));
            }
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Searching,
    /// The goal was reached: `take_path` gives the whole way.
    Found,
    /// The node limit was hit or nothing more is reachable: `take_path` gives
    /// the way to the closest node, if that is any progress.
    Settled,
    /// No progress at all is possible from the start.
    Failed,
}

#[allow(dead_code)]
struct Entry {
    f: f64,
    g: f64,
    pos: Pos,
}

#[allow(dead_code)]
impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
#[allow(dead_code)]
impl Eq for Entry {}
#[allow(dead_code)]
impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
#[allow(dead_code)]
impl Ord for Entry {
    // BinaryHeap pops the greatest: the lowest f, then the deepest node.
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .f
            .total_cmp(&self.f)
            .then_with(|| self.g.total_cmp(&other.g))
            .then_with(|| self.pos.cmp(&other.pos))
    }
}

#[allow(dead_code)]
/// An A* search that can be run a few hundred nodes a tick.
pub struct Search {
    goal: Goal,
    opts: Options,
    start: Pos,
    open: BinaryHeap<Entry>,
    g: HashMap<Pos, f64>,
    parent: HashMap<Pos, (Pos, Kind)>,
    expanded: usize,
    best: Pos,
    best_h: f64,
    best_g: f64,
    result: Option<(Status, Pos)>,
    scratch: Vec<(Pos, Kind, f64)>,
}

#[allow(dead_code)]
impl Search {
    pub fn new(start: Pos, goal: Goal, opts: Options) -> Search {
        let h = goal.heuristic(start);
        let mut open = BinaryHeap::new();
        open.push(Entry { f: h, g: 0.0, pos: start });
        let mut g = HashMap::new();
        g.insert(start, 0.0);
        Search {
            goal,
            opts,
            start,
            open,
            g,
            parent: HashMap::new(),
            expanded: 0,
            best: start,
            best_h: h,
            best_g: 0.0,
            result: None,
            scratch: Vec::new(),
        }
    }

    pub fn expanded(&self) -> usize {
        self.expanded
    }

    /// Expands up to `budget` nodes.
    pub fn step<W: WorldView>(&mut self, world: &W, budget: usize) -> Status {
        if let Some((status, _)) = self.result {
            return status;
        }
        let mut left = budget;
        while left > 0 {
            let Some(entry) = self.open.pop() else {
                return self.settle();
            };
            if entry.g > self.g.get(&entry.pos).copied().unwrap_or(f64::INFINITY) {
                continue; // a stale entry: a cheaper way was found since
            }
            if self.goal.reached(entry.pos) {
                self.result = Some((Status::Found, entry.pos));
                return Status::Found;
            }
            if self.expanded >= self.opts.max_nodes {
                return self.settle();
            }
            self.expanded += 1;
            left -= 1;
            let mut out = std::mem::take(&mut self.scratch);
            neighbors(world, &self.opts, entry.pos, &mut out);
            for &(pos, kind, cost) in &out {
                let g = entry.g + cost;
                if g < self.g.get(&pos).copied().unwrap_or(f64::INFINITY) {
                    self.g.insert(pos, g);
                    self.parent.insert(pos, (entry.pos, kind));
                    let h = self.goal.heuristic(pos);
                    if h < self.best_h - 1e-9 || (h <= self.best_h + 1e-9 && g < self.best_g) {
                        self.best = pos;
                        self.best_h = h;
                        self.best_g = g;
                    }
                    self.open.push(Entry { f: g + h, g, pos });
                }
            }
            self.scratch = out;
        }
        Status::Searching
    }

    fn settle(&mut self) -> Status {
        let status = if self.best == self.start {
            Status::Failed
        } else {
            Status::Settled
        };
        self.result = Some((status, self.best));
        status
    }

    /// The path, once `step` has stopped returning `Searching`.
    pub fn take_path(&self) -> Option<Path> {
        let (status, end) = self.result?;
        if status == Status::Failed {
            return None;
        }
        let mut steps = Vec::new();
        let mut at = end;
        while at != self.start {
            let &(from, kind) = self.parent.get(&at)?;
            steps.push(Step { pos: at, kind });
            at = from;
        }
        steps.reverse();
        Some(Path {
            steps,
            complete: status == Status::Found,
        })
    }
}

#[allow(dead_code)]
/// One-shot search, for tests and tools.
pub fn find_path<W: WorldView>(world: &W, start: Pos, goal: Goal, opts: Options) -> Option<Path> {
    let mut s = Search::new(start, goal, opts);
    while s.step(world, 1_000) == Status::Searching {}
    s.take_path()
}

#[allow(dead_code)]
/// The yaw that faces (dx, dz).
pub fn yaw_to(dx: f64, dz: f64) -> f32 {
    (-dx).atan2(dz).to_degrees() as f32
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Controls {
    pub forward: bool,
    pub back: bool,
    pub left: bool,
    pub right: bool,
    pub jump: bool,
    pub sprint: bool,
    /// Where to turn the camera: only when asked to (`Options::turn_camera`).
    pub yaw: Option<f32>,
}

#[allow(dead_code)]
#[allow(dead_code)]
/// Which of W, S, A and D (forward, back, left, right) move a player looking
/// along `view_yaw` toward (dx, dz): the nearest of the eight directions.
pub fn wasd(view_yaw: f32, dx: f64, dz: f64) -> (bool, bool, bool, bool) {
    let len = (dx * dx + dz * dz).sqrt();
    if len < 1e-6 {
        return (false, false, false, false);
    }
    let (dx, dz) = (dx / len, dz / len);
    let r = f64::from(view_yaw).to_radians();
    let ahead = -dx * r.sin() + dz * r.cos();
    let to_right = -dx * r.cos() - dz * r.sin();
    const SIN_22_5: f64 = 0.3827;
    (
        ahead > SIN_22_5,
        ahead < -SIN_22_5,
        to_right < -SIN_22_5,
        to_right > SIN_22_5,
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Follow {
    Moving(Controls),
    Arrived,
    /// No progress for three seconds, or far off the path: plan again.
    Stuck,
}

#[allow(dead_code)]
/// Walks a `Path`, tick by tick.
pub struct Follower {
    steps: Vec<Step>,
    next: usize,
    from: Pos,
    best: f64,
    stalled: u32,
    sprint: bool,
}

#[allow(dead_code)]
impl Follower {
    pub fn new(start: Pos, steps: Vec<Step>, sprint: bool) -> Follower {
        Follower {
            steps,
            next: 0,
            from: start,
            best: f64::INFINITY,
            stalled: 0,
            sprint,
        }
    }

    pub fn remaining(&self) -> usize {
        self.steps.len() - self.next
    }

    /// `view_yaw` is where the camera points now: with it, the controls steer
    /// with W, A, S and D and never turn the camera; without it, they ask for
    /// a yaw and walk forward.
    pub fn update(
        &mut self,
        pos: [f64; 3],
        on_ground: bool,
        in_water: bool,
        view_yaw: Option<f32>,
    ) -> Follow {
        let horizontal = |p: Pos| -> f64 {
            let dx = p[0] as f64 + 0.5 - pos[0];
            let dz = p[2] as f64 + 0.5 - pos[2];
            (dx * dx + dz * dz).sqrt()
        };
        loop {
            if self.next >= self.steps.len() {
                return Follow::Arrived;
            }
            let s = self.steps[self.next];
            let tol = if s.kind == Kind::Parkour { 0.45 } else { 0.3 };
            let dy = pos[1] - s.pos[1] as f64;
            if horizontal(s.pos) < tol && dy.abs() < 0.75 && (on_ground || in_water) {
                self.from = s.pos;
                self.next += 1;
                self.stalled = 0;
                self.best = f64::INFINITY;
                continue;
            }
            break;
        }
        let s = self.steps[self.next];
        let dh = horizontal(s.pos);
        let dy = s.pos[1] as f64 - pos[1];

        if dh > 4.5 && horizontal(self.from) > 4.5 {
            return Follow::Stuck;
        }
        let progress = dh + dy.abs();
        if progress < self.best - 0.02 {
            self.best = progress;
            self.stalled = 0;
        } else {
            self.stalled += 1;
            if self.stalled > 60 {
                return Follow::Stuck;
            }
        }

        let (to_x, to_z) = (s.pos[0] as f64 + 0.5 - pos[0], s.pos[2] as f64 + 0.5 - pos[2]);
        let (forward, back, left, right, yaw) = match view_yaw {
            Some(view) => {
                let (f, b, l, r) = if dh > 0.05 {
                    wasd(view, to_x, to_z)
                } else {
                    (false, false, false, false)
                };
                (f, b, l, r, None)
            }
            None => (true, false, false, false, if dh > 0.05 { Some(yaw_to(to_x, to_z)) } else { None }),
        };
        let jump = match s.kind {
            Kind::Ascend => dy > 0.5 && dh < 1.6 && on_ground,
            Kind::Parkour => {
                let span = (s.pos[0] - self.from[0])
                    .abs()
                    .max((s.pos[2] - self.from[2]).abs());
                let gap = (span - 1).max(0) as f64;
                on_ground && dh <= gap + 0.65
            }
            Kind::Swim => dy > 0.2 || (in_water && dy > -0.5),
            Kind::Traverse | Kind::Diagonal => in_water,
            Kind::Descend => false,
        };
        // Sprinting needs the forward key.
        let sprint = ((self.sprint && s.kind != Kind::Swim) || s.kind == Kind::Parkour) && forward;
        Follow::Moving(Controls {
            forward,
            back,
            left,
            right,
            jump,
            sprint,
            yaw,
        })
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tick {
    /// Searching: stand still.
    Idle,
    Controls(Controls),
    Arrived,
    Failed,
}

#[allow(dead_code)]
enum Phase {
    Fresh,
    Searching(Search),
    Following { follower: Follower, complete: bool, from: Pos },
    Done(Tick),
}

#[allow(dead_code)]
/// Plans, follows, and plans again until the goal is reached or no progress
/// is possible: what a `/goto` command runs.
pub struct Pather {
    goal: Goal,
    opts: Options,
    phase: Phase,
    replans: u32,
    per_tick: usize,
    pub sprint: bool,
}

#[allow(dead_code)]
impl Pather {
    pub fn new(goal: Goal, opts: Options) -> Pather {
        Pather {
            goal,
            opts,
            phase: Phase::Fresh,
            replans: 0,
            per_tick: 400,
            sprint: true,
        }
    }

    /// Nodes expanded a tick while searching.
    pub fn with_budget(mut self, per_tick: usize) -> Pather {
        self.per_tick = per_tick.max(1);
        self
    }

    pub fn goal(&self) -> Goal {
        self.goal
    }

    /// One line for `/path status`.
    pub fn describe(&self) -> String {
        match &self.phase {
            Phase::Fresh => "about to plan".to_string(),
            Phase::Searching(s) => format!("planning ({} nodes so far)", s.expanded()),
            Phase::Following { follower, complete, .. } => format!(
                "{} steps to go{}",
                follower.remaining(),
                if *complete { "" } else { " on the first leg" }
            ),
            Phase::Done(Tick::Arrived) => "arrived".to_string(),
            Phase::Done(_) => "no way found".to_string(),
        }
    }

    pub fn tick<W: WorldView>(
        &mut self,
        world: &W,
        pos: [f64; 3],
        on_ground: bool,
        in_water: bool,
        view_yaw: Option<f32>,
    ) -> Tick {
        let here = node_at(pos);
        if let Phase::Done(t) = self.phase {
            return t;
        }
        if self.goal.reached(here) {
            self.phase = Phase::Done(Tick::Arrived);
            return Tick::Arrived;
        }
        if let Phase::Fresh = self.phase {
            self.phase = Phase::Searching(Search::new(here, self.goal, self.opts.clone()));
        }
        if let Phase::Searching(search) = &mut self.phase {
            match search.step(world, self.per_tick) {
                Status::Searching => return Tick::Idle,
                Status::Failed => {
                    self.phase = Phase::Done(Tick::Failed);
                    return Tick::Failed;
                }
                Status::Found | Status::Settled => {
                    let path = search.take_path();
                    match path {
                        Some(p) => {
                            self.phase = Phase::Following {
                                follower: Follower::new(here, p.steps, self.sprint),
                                complete: p.complete,
                                from: here,
                            };
                        }
                        None => {
                            self.phase = Phase::Done(Tick::Failed);
                            return Tick::Failed;
                        }
                    }
                }
            }
        }
        let mut replan = false;
        let mut finished = None;
        if let Phase::Following { follower, complete, from } = &mut self.phase {
            match follower.update(pos, on_ground, in_water, view_yaw) {
                Follow::Moving(c) => return Tick::Controls(c),
                Follow::Arrived => {
                    if *complete {
                        finished = Some(Tick::Arrived);
                    } else if here == *from {
                        finished = Some(Tick::Failed); // a partial path that went nowhere
                    } else {
                        replan = true;
                    }
                }
                Follow::Stuck => {
                    self.replans += 1;
                    if self.replans > 3 {
                        finished = Some(Tick::Failed);
                    } else {
                        replan = true;
                    }
                }
            }
        }
        if let Some(t) = finished {
            self.phase = Phase::Done(t);
            return t;
        }
        if replan {
            self.phase = Phase::Searching(Search::new(here, self.goal, self.opts.clone()));
        }
        Tick::Idle
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Liquid {
    None,
    Water,
    Lava,
}

#[allow(dead_code)]
/// What a block is to the search. `name` is the block's id (`minecraft:cactus`),
/// `solid` whether it has full collision. Partial blocks (slabs, stairs, fences,
/// doors) are not modelled yet: they count as air unless the game calls them solid.
pub fn classify(name: &str, solid: bool, liquid: Liquid) -> Cell {
    let short = name.strip_prefix("minecraft:").unwrap_or(name);
    let hazard = matches!(
        short,
        "fire"
            | "soul_fire"
            | "cactus"
            | "magma_block"
            | "sweet_berry_bush"
            | "cobweb"
            | "powder_snow"
            | "wither_rose"
            | "campfire"
            | "soul_campfire"
            | "pointed_dripstone"
            | "lava_cauldron"
    );
    if hazard {
        Cell::Hazard
    } else if liquid == Liquid::Lava {
        Cell::Lava
    } else if solid {
        Cell::Solid
    } else if liquid == Liquid::Water {
        Cell::Water
    } else {
        Cell::Air
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub enum Setting {
    Parkour(bool),
    Swim(bool),
    Diagonal(bool),
    Sprint(bool),
    Look(bool),
    MaxFall(i32),
    MaxNodes(usize),
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Goto(Goal),
    Stop,
    Status,
    Set(Setting),
    Help,
}

#[allow(dead_code)]
pub const HELP: [&str; 7] = [
    "/goto <x> <y> <z>  walk to a block; ~ is relative to you (/goto ~10 ~ ~)",
    "/goto <x> <z>  walk to a column, at any height",
    "/goto <x> <y> <z> <radius>  get within a radius of a block",
    "/path stop | status",
    "/path parkour|swim|diagonal|sprint|look on|off",
    "  look on: turn the camera toward the target (off: steer with W A S D; sprint-jumps then need the gap in front of you)",
    "/path fall <blocks> | nodes <count>",
];

#[allow(dead_code)]
fn coord(word: &str, here: f64) -> Option<i32> {
    if let Some(rest) = word.strip_prefix('~') {
        let d = if rest.is_empty() { 0.0 } else { rest.parse::<f64>().ok()? };
        Some((here + d).floor() as i32)
    } else {
        Some(word.parse::<f64>().ok()?.floor() as i32)
    }
}

#[allow(dead_code)]
fn flag(word: &str) -> Option<bool> {
    match word {
        "on" | "true" | "yes" => Some(true),
        "off" | "false" | "no" => Some(false),
        _ => None,
    }
}

#[allow(dead_code)]
/// Reads a chat command (without its slash). `None` when it is not ours, so
/// the game carries on with it; `here` is the player's position.
pub fn parse_command(command: &str, here: [f64; 3]) -> Option<Result<Cmd, String>> {
    let mut words = command.split_whitespace();
    let head = words.next()?;
    let rest: Vec<&str> = words.collect();
    match head {
        "goto" => Some(parse_goto(&rest, here)),
        "path" => Some(parse_path(&rest)),
        _ => None,
    }
}

#[allow(dead_code)]
fn parse_goto(rest: &[&str], here: [f64; 3]) -> Result<Cmd, String> {
    let usage = || "Usage: /goto <x> <y> <z> [radius], or /goto <x> <z>".to_string();
    let bad = |w: &str| format!("\"{w}\" is not a coordinate");
    let c = |i: usize, axis: usize| coord(rest[i], here[axis]).ok_or_else(|| bad(rest[i]));
    match rest.len() {
        2 => Ok(Cmd::Goto(Goal::Column(c(0, 0)?, c(1, 2)?))),
        3 => Ok(Cmd::Goto(Goal::Exact([c(0, 0)?, c(1, 1)?, c(2, 2)?]))),
        4 => {
            let r: f64 = rest[3].parse().map_err(|_| bad(rest[3]))?;
            if !(0.0..=1000.0).contains(&r) {
                return Err("The radius is 0 to 1000 blocks".to_string());
            }
            Ok(Cmd::Goto(Goal::Near([c(0, 0)?, c(1, 1)?, c(2, 2)?], r)))
        }
        _ => Err(usage()),
    }
}

#[allow(dead_code)]
fn parse_path(rest: &[&str]) -> Result<Cmd, String> {
    match rest {
        [] | ["help"] => Ok(Cmd::Help),
        ["stop"] => Ok(Cmd::Stop),
        ["status"] => Ok(Cmd::Status),
        [name @ ("parkour" | "swim" | "diagonal" | "sprint" | "look"), value] => {
            let on = flag(value).ok_or_else(|| format!("/path {name} takes on or off"))?;
            Ok(Cmd::Set(match *name {
                "parkour" => Setting::Parkour(on),
                "swim" => Setting::Swim(on),
                "diagonal" => Setting::Diagonal(on),
                "look" => Setting::Look(on),
                _ => Setting::Sprint(on),
            }))
        }
        ["fall", n] => match n.parse::<i32>() {
            Ok(n) if (0..=20).contains(&n) => Ok(Cmd::Set(Setting::MaxFall(n))),
            _ => Err("/path fall takes 0 to 20 blocks (3 never hurts)".to_string()),
        },
        ["nodes", n] => match n.parse::<usize>() {
            Ok(n) if (100..=500_000).contains(&n) => Ok(Cmd::Set(Setting::MaxNodes(n))),
            _ => Err("/path nodes takes 100 to 500000".to_string()),
        },
        _ => Err("Unknown /path option: try /path help".to_string()),
    }
}

#[allow(dead_code)]
/// Applies one setting to the options (and says whether sprinting changed).
pub fn apply_setting(opts: &mut Options, sprint: &mut bool, setting: &Setting) {
    match *setting {
        Setting::Parkour(v) => opts.allow_parkour = v,
        Setting::Swim(v) => opts.allow_swim = v,
        Setting::Diagonal(v) => opts.allow_diagonal = v,
        Setting::Sprint(v) => *sprint = v,
        Setting::Look(v) => opts.turn_camera = v,
        Setting::MaxFall(v) => opts.max_fall = v,
        Setting::MaxNodes(v) => opts.max_nodes = v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A flat world with a solid floor at y <= 0 (feet at y = 1), bounded by
    // `limit` blocks from the origin, with cells overridden by `cells`.
    struct Mock {
        cells: HashMap<Pos, Cell>,
        limit: i32,
    }

    impl Mock {
        fn new() -> Mock {
            Mock { cells: HashMap::new(), limit: 12 }
        }
        fn set(&mut self, p: Pos, c: Cell) {
            self.cells.insert(p, c);
        }
    }

    impl WorldView for Mock {
        fn cell(&self, p: Pos) -> Cell {
            if p[0].abs() > self.limit || p[2].abs() > self.limit {
                return Cell::Unloaded;
            }
            match self.cells.get(&p) {
                Some(c) => *c,
                None if p[1] <= 0 => Cell::Solid,
                None => Cell::Air,
            }
        }
    }

    const START: Pos = [0, 1, 0];

    fn kinds(p: &Path) -> Vec<Kind> {
        p.steps.iter().map(|s| s.kind).collect()
    }

    #[test]
    fn straight_line_on_flat_ground() {
        let w = Mock::new();
        let p = find_path(&w, START, Goal::Exact([5, 1, 0]), Options::default()).unwrap();
        assert!(p.complete);
        assert_eq!(p.steps.len(), 5);
        assert!(kinds(&p).iter().all(|k| *k == Kind::Traverse));
        assert_eq!(p.steps.last().unwrap().pos, [5, 1, 0]);
    }

    #[test]
    fn diagonals_are_used_in_open_ground() {
        let w = Mock::new();
        let p = find_path(&w, START, Goal::Exact([3, 1, 3]), Options::default()).unwrap();
        assert_eq!(kinds(&p), vec![Kind::Diagonal; 3]);
    }

    #[test]
    fn goes_around_a_wall() {
        let mut w = Mock::new();
        for z in -5..=5 {
            w.set([2, 1, z], Cell::Solid);
            w.set([2, 2, z], Cell::Solid);
        }
        let p = find_path(&w, START, Goal::Exact([4, 1, 0]), Options::default()).unwrap();
        assert!(p.complete);
        assert!(p.steps.len() > 4);
        assert!(p.steps.iter().all(|s| s.pos[0] != 2 || s.pos[2].abs() > 5));
    }

    #[test]
    fn climbs_one_block_steps() {
        let mut w = Mock::new();
        w.set([1, 1, 0], Cell::Solid);
        w.set([2, 1, 0], Cell::Solid);
        w.set([2, 2, 0], Cell::Solid);
        let p = find_path(&w, START, Goal::Exact([2, 3, 0]), Options::default()).unwrap();
        assert_eq!(kinds(&p), vec![Kind::Ascend, Kind::Ascend]);
        assert_eq!(p.steps[0].pos, [1, 2, 0]);
    }

    #[test]
    fn will_not_jump_up_under_a_ceiling() {
        let mut w = Mock::new();
        w.set([1, 1, 0], Cell::Solid);
        w.set([0, 3, 0], Cell::Solid); // no room to jump at the start
        let p = find_path(&w, START, Goal::Exact([1, 2, 0]), Options::default()).unwrap();
        // It can still get there, but from a side with headroom.
        assert!(p.complete);
        assert!(p.steps.len() > 1);
        assert_ne!(p.steps[0].pos, [1, 2, 0]);
    }

    #[test]
    fn drops_a_few_blocks_but_not_too_many() {
        let mut w = Mock::new();
        for y in 1..=4 {
            w.set([0, y, 0], Cell::Solid); // a pillar
        }
        let top = [0, 5, 0];
        // Four blocks down is past the default limit of three.
        assert!(find_path(&w, top, Goal::Exact([1, 1, 0]), Options::default()).is_none());
        let mut o = Options::default();
        o.max_fall = 4;
        let p = find_path(&w, top, Goal::Exact([1, 1, 0]), o).unwrap();
        assert!(p.complete);
        assert_eq!(kinds(&p), vec![Kind::Descend]);
        // Three blocks down is fine by default.
        let p = find_path(&w, [0, 4, 0], Goal::Exact([1, 1, 0]), Options::default()).unwrap();
        assert!(p.complete);
    }

    #[test]
    fn never_steps_on_or_into_lava() {
        let mut w = Mock::new();
        for z in -12..=12 {
            w.set([2, 0, z], Cell::Lava); // a lava river in the floor
        }
        let p = find_path(&w, START, Goal::Exact([4, 1, 0]), Options::default());
        assert!(p.map_or(true, |p| !p.complete));
    }

    fn pit_world() -> Mock {
        let mut w = Mock::new();
        for z in -12..=12 {
            for y in -3..=0 {
                w.set([2, y, z], Cell::Air); // a pit four deep
            }
        }
        w
    }

    #[test]
    fn a_wide_pit_needs_parkour() {
        let w = pit_world();
        let goal = Goal::Exact([4, 1, 0]);
        assert!(find_path(&w, START, goal, Options::default()).map_or(true, |p| !p.complete));
        let mut o = Options::default();
        o.allow_parkour = true;
        let p = find_path(&w, START, goal, o).unwrap();
        assert!(p.complete);
        assert!(kinds(&p).contains(&Kind::Parkour));
        assert!(p.steps.iter().all(|s| s.pos[0] != 2));
    }

    #[test]
    fn parkour_checks_headroom_over_the_gap() {
        let mut w = pit_world();
        for z in -12..=12 {
            w.set([2, 3, z], Cell::Solid); // a ceiling over the whole pit
        }
        let mut o = Options::default();
        o.allow_parkour = true;
        let p = find_path(&w, START, Goal::Exact([4, 1, 0]), o);
        assert!(p.map_or(true, |p| !p.complete));
    }

    fn pool() -> Mock {
        let mut w = Mock::new();
        for x in 1..=3 {
            for z in -12..=12 {
                w.set([x, 0, z], Cell::Water);
                w.set([x, 1, z], Cell::Water);
            }
        }
        w
    }

    #[test]
    fn swims_across_water_when_allowed() {
        let w = pool();
        let goal = Goal::Exact([4, 1, 0]);
        let p = find_path(&w, START, goal, Options::default()).unwrap();
        assert!(p.complete);
        assert!(kinds(&p).contains(&Kind::Swim));
        let mut o = Options::default();
        o.allow_swim = false;
        assert!(find_path(&w, START, goal, o).map_or(true, |p| !p.complete));
    }

    #[test]
    fn does_not_cut_corners() {
        let mut w = Mock::new();
        w.set([1, 1, 0], Cell::Solid);
        w.set([1, 2, 0], Cell::Solid);
        let p = find_path(&w, START, Goal::Exact([1, 1, 1]), Options::default()).unwrap();
        assert_ne!(p.steps[0].kind, Kind::Diagonal);
        assert_eq!(p.steps.len(), 2);
    }

    #[test]
    fn unreachable_goal_settles_on_the_closest_node() {
        let mut w = Mock::new();
        for z in -12..=12 {
            w.set([5, 1, z], Cell::Solid);
            w.set([5, 2, z], Cell::Solid);
        }
        let p = find_path(&w, START, Goal::Exact([8, 1, 0]), Options::default()).unwrap();
        assert!(!p.complete);
        assert_eq!(p.steps.last().unwrap().pos[0], 4);
    }

    #[test]
    fn boxed_in_start_fails() {
        let mut w = Mock::new();
        for (dx, dz) in CARDINALS.iter().chain(DIAGONALS.iter()) {
            w.set([*dx, 1, *dz], Cell::Solid);
            w.set([*dx, 2, *dz], Cell::Solid);
        }
        assert!(find_path(&w, START, Goal::Exact([5, 1, 0]), Options::default()).is_none());
    }

    #[test]
    fn near_and_column_goals() {
        let w = Mock::new();
        let p = find_path(&w, START, Goal::Near([6, 1, 0], 2.0), Options::default()).unwrap();
        assert!(p.complete);
        assert_eq!(p.steps.len(), 4);
        let p = find_path(&w, START, Goal::Column(0, 7), Options::default()).unwrap();
        assert!(p.complete);
        assert_eq!(p.steps.last().unwrap().pos[2], 7);
    }

    #[test]
    fn stepping_in_small_slices_finds_the_same_path() {
        let mut w = Mock::new();
        for z in -5..=5 {
            w.set([2, 1, z], Cell::Solid);
            w.set([2, 2, z], Cell::Solid);
        }
        let goal = Goal::Exact([4, 1, 0]);
        let whole = find_path(&w, START, goal, Options::default()).unwrap();
        let mut s = Search::new(START, goal, Options::default());
        let mut ticks = 0;
        while s.step(&w, 3) == Status::Searching {
            ticks += 1;
        }
        assert!(ticks > 1);
        assert_eq!(s.take_path().unwrap(), whole);
    }

    #[test]
    fn node_limit_gives_a_partial_path() {
        let w = Mock::new();
        let mut o = Options::default();
        o.max_nodes = 5;
        let p = find_path(&w, START, Goal::Exact([10, 1, 0]), o).unwrap();
        assert!(!p.complete);
        assert!(!p.steps.is_empty());
    }

    #[test]
    fn node_at_rounds_like_the_agent() {
        assert_eq!(node_at([0.5, 1.0, -0.2]), [0, 1, -1]);
        assert_eq!(node_at([3.99, 64.999, 7.0]), [3, 65, 7]);
    }

    #[test]
    fn yaw_matches_minecraft() {
        assert!((yaw_to(0.0, 1.0) - 0.0).abs() < 1e-4); // south
        assert!((yaw_to(-1.0, 0.0) - 90.0).abs() < 1e-4); // west
        assert!((yaw_to(1.0, 0.0) + 90.0).abs() < 1e-4); // east
    }

    #[test]
    fn follower_walks_then_arrives() {
        let steps = vec![Step { pos: [1, 1, 0], kind: Kind::Traverse }];
        let mut f = Follower::new(START, steps, false);
        // Turning the camera: face east and walk forward.
        match f.update([0.5, 1.0, 0.5], true, false, None) {
            Follow::Moving(c) => {
                assert!(c.forward && !c.back && !c.left && !c.right && !c.jump && !c.sprint);
                assert!((c.yaw.unwrap() + 90.0).abs() < 1e-3);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(f.update([1.5, 1.0, 0.5], true, false, None), Follow::Arrived);
    }

    #[test]
    fn follower_jumps_for_an_ascend_only_near_it_and_on_the_ground() {
        let steps = vec![Step { pos: [3, 2, 0], kind: Kind::Ascend }];
        let mut f = Follower::new(START, steps, false);
        let jump = |f: &mut Follower, p: [f64; 3], ground: bool| match f.update(p, ground, false, None) {
            Follow::Moving(c) => c.jump,
            other => panic!("{other:?}"),
        };
        assert!(!jump(&mut f, [0.5, 1.0, 0.5], true)); // 3 blocks away
        assert!(jump(&mut f, [2.0, 1.0, 0.5], true)); // within 1.6
        assert!(!jump(&mut f, [2.2, 1.4, 0.5], false)); // already airborne
    }

    #[test]
    fn follower_jumps_a_gap_at_the_edge() {
        let steps = vec![Step { pos: [2, 1, 0], kind: Kind::Parkour }];
        let mut f = Follower::new(START, steps, false);
        let moving = |f: &mut Follower, x: f64| match f.update([x, 1.0, 0.5], true, false, None) {
            Follow::Moving(c) => c,
            other => panic!("{other:?}"),
        };
        assert!(moving(&mut f, 0.5).sprint);
        assert!(!moving(&mut f, 0.5).jump); // still a block and a half out
        assert!(moving(&mut f, 0.95).jump); // at the start block's edge
    }

    #[test]
    fn follower_reports_stuck_after_no_progress() {
        let steps = vec![Step { pos: [5, 1, 0], kind: Kind::Traverse }];
        let mut f = Follower::new(START, steps, false);
        let mut result = Follow::Arrived;
        for _ in 0..70 {
            result = f.update([0.5, 1.0, 0.5], true, false, None);
            if result == Follow::Stuck {
                break;
            }
        }
        assert_eq!(result, Follow::Stuck);
    }

    #[test]
    fn follower_reports_stuck_when_far_off_the_path() {
        let steps = vec![Step { pos: [1, 1, 0], kind: Kind::Traverse }];
        let mut f = Follower::new(START, steps, false);
        assert_eq!(f.update([20.0, 1.0, 20.0], true, false, None), Follow::Stuck);
    }

    #[test]
    fn wasd_picks_keys_relative_to_the_camera() {
        // Facing south (yaw 0): +z is ahead, -z behind, +x on the left, -x on the right.
        assert_eq!(wasd(0.0, 0.0, 5.0), (true, false, false, false));
        assert_eq!(wasd(0.0, 0.0, -5.0), (false, true, false, false));
        assert_eq!(wasd(0.0, 5.0, 0.0), (false, false, true, false));
        assert_eq!(wasd(0.0, -5.0, 0.0), (false, false, false, true));
        assert_eq!(wasd(0.0, 3.0, 3.0), (true, false, true, false)); // ahead and left
        assert_eq!(wasd(0.0, -3.0, -3.0), (false, true, false, true));
        // Facing east (yaw -90): +x is ahead.
        assert_eq!(wasd(-90.0, 5.0, 0.0), (true, false, false, false));
        // Facing west (yaw 90): +x is behind.
        assert_eq!(wasd(90.0, 5.0, 0.0), (false, true, false, false));
        // Nowhere to go: no keys.
        assert_eq!(wasd(40.0, 0.0, 0.0), (false, false, false, false));
    }

    #[test]
    fn wasd_never_presses_opposite_keys() {
        for yaw in (-180..180).step_by(7) {
            for a in 0..360 {
                let r = f64::from(a).to_radians();
                let (f, b, l, rt) = wasd(yaw as f32, r.cos(), r.sin());
                assert!(!(f && b) && !(l && rt));
                assert!(f || b || l || rt, "no key at yaw {yaw}, angle {a}");
            }
        }
    }

    #[test]
    fn wasd_is_within_a_quarter_turn_of_the_target() {
        // The keys move the player in one of eight directions: never more
        // than 22.5 degrees off.
        for yaw in [-170.0f32, -45.0, 0.0, 33.0, 90.0, 151.0] {
            for a in 0..360 {
                let t = f64::from(a).to_radians();
                let (dx, dz) = (t.cos(), t.sin());
                let (f, b, l, r) = wasd(yaw, dx, dz);
                let v = f64::from(yaw).to_radians();
                let fwd = (-v.sin(), v.cos());
                let rgt = (-v.cos(), -v.sin());
                let (a1, a2) = (
                    f64::from(f as i32 - b as i32),
                    f64::from(r as i32 - l as i32),
                );
                let (mx, mz) = (a1 * fwd.0 + a2 * rgt.0, a1 * fwd.1 + a2 * rgt.1);
                let cos = (mx * dx + mz * dz) / (mx * mx + mz * mz).sqrt();
                assert!(cos > 0.92, "yaw {yaw} angle {a}: cos {cos}"); // cos 22.5 = 0.924
            }
        }
    }

    #[test]
    fn follower_steers_with_keys_and_leaves_the_camera_alone() {
        let steps = vec![Step { pos: [3, 1, 0], kind: Kind::Traverse }];
        let mut f = Follower::new(START, steps, true);
        // Looking south, the target is to the east: strafe left, no yaw, no sprint.
        match f.update([0.5, 1.0, 0.5], true, false, Some(0.0)) {
            Follow::Moving(c) => {
                assert!(c.left && !c.forward && !c.right && !c.back);
                assert_eq!(c.yaw, None);
                assert!(!c.sprint, "sprinting needs the forward key");
            }
            other => panic!("{other:?}"),
        }
        // Looking east at the same target: forward, sprinting.
        match f.update([0.5, 1.0, 0.5], true, false, Some(-90.0)) {
            Follow::Moving(c) => {
                assert!(c.forward && !c.left && !c.right && c.sprint);
                assert_eq!(c.yaw, None);
            }
            other => panic!("{other:?}"),
        }
    }

    // A toy player. With `view` it keeps its camera fixed and moves by the
    // keys the controls press; without, it turns toward the target and walks
    // forward. It steps 0.25 a tick and stands on whatever is under its feet.
    fn simulate(
        w: &Mock,
        pather: &mut Pather,
        mut pos: [f64; 3],
        view: Option<f32>,
        max_ticks: usize,
    ) -> (Tick, [f64; 3]) {
        for _ in 0..max_ticks {
            match pather.tick(w, pos, true, false, view) {
                Tick::Controls(c) => {
                    let (mut mx, mut mz) = (0.0f64, 0.0f64);
                    match view {
                        Some(v) => {
                            assert_eq!(c.yaw, None, "the camera must stay put");
                            let r = f64::from(v).to_radians();
                            let a = f64::from(c.forward as i32 - c.back as i32);
                            let b = f64::from(c.right as i32 - c.left as i32);
                            mx = a * -r.sin() + b * -r.cos();
                            mz = a * r.cos() + b * -r.sin();
                        }
                        None => {
                            if let Some(yaw) = c.yaw {
                                let r = f64::from(yaw).to_radians();
                                mx = -r.sin();
                                mz = r.cos();
                            }
                        }
                    }
                    let len = (mx * mx + mz * mz).sqrt();
                    if len > 0.0 {
                        pos[0] += mx / len * 0.25;
                        pos[2] += mz / len * 0.25;
                    }
                    let n = node_at(pos);
                    let mut y = n[1];
                    while w.cell([n[0], y, n[2]]) == Cell::Solid {
                        y += 1;
                    }
                    while w.cell([n[0], y - 1, n[2]]) == Cell::Air {
                        y -= 1;
                    }
                    pos[1] = y as f64;
                }
                Tick::Idle => {}
                done => return (done, pos),
            }
        }
        (Tick::Idle, pos)
    }

    #[test]
    fn pather_reaches_a_goal_around_a_wall_with_stairs() {
        let mut w = Mock::new();
        for z in -5..=5 {
            w.set([3, 1, z], Cell::Solid);
            w.set([3, 2, z], Cell::Solid);
        }
        w.set([1, 1, 0], Cell::Solid); // a step on the way
        let mut p = Pather::new(Goal::Exact([6, 1, 0]), Options::default());
        let (t, end) = simulate(&w, &mut p, [0.5, 1.0, 0.5], None, 2_000);
        assert_eq!(t, Tick::Arrived, "ended at {end:?}");
        assert_eq!(node_at(end), [6, 1, 0]);
    }

    #[test]
    fn pather_arrives_with_a_fixed_camera_in_any_direction() {
        let mut w = Mock::new();
        for z in -5..=5 {
            w.set([3, 1, z], Cell::Solid);
            w.set([3, 2, z], Cell::Solid);
        }
        w.set([1, 1, 0], Cell::Solid);
        for view in [0.0f32, 37.0, 90.0, -120.0, 180.0] {
            let mut p = Pather::new(Goal::Exact([6, 1, 0]), Options::default());
            let (t, end) = simulate(&w, &mut p, [0.5, 1.0, 0.5], Some(view), 3_000);
            assert_eq!(t, Tick::Arrived, "camera {view}: ended at {end:?}");
            assert_eq!(node_at(end), [6, 1, 0], "camera {view}");
        }
    }

    #[test]
    fn pather_plans_again_after_a_partial_path() {
        let w = Mock::new();
        let mut o = Options::default();
        o.max_nodes = 12; // too few to find the whole way at once
        let mut p = Pather::new(Goal::Exact([9, 1, 0]), o);
        let (t, end) = simulate(&w, &mut p, [0.5, 1.0, 0.5], None, 4_000);
        assert_eq!(t, Tick::Arrived, "ended at {end:?}");
        let mut p = Pather::new(Goal::Exact([9, 1, 0]), {
            let mut o = Options::default();
            o.max_nodes = 12;
            o
        });
        let (t, end) = simulate(&w, &mut p, [0.5, 1.0, 0.5], Some(55.0), 4_000);
        assert_eq!(t, Tick::Arrived, "fixed camera: ended at {end:?}");
    }

    #[test]
    fn pather_gives_up_when_boxed_in() {
        let mut w = Mock::new();
        for (dx, dz) in CARDINALS.iter().chain(DIAGONALS.iter()) {
            w.set([*dx, 1, *dz], Cell::Solid);
            w.set([*dx, 2, *dz], Cell::Solid);
        }
        let mut p = Pather::new(Goal::Exact([5, 1, 0]), Options::default());
        let (t, _) = simulate(&w, &mut p, [0.5, 1.0, 0.5], None, 100);
        assert_eq!(t, Tick::Failed);
    }
    #[test]
    fn classifies_blocks() {
        use Liquid::*;
        assert_eq!(classify("minecraft:stone", true, None), Cell::Solid);
        assert_eq!(classify("minecraft:air", false, None), Cell::Air);
        assert_eq!(classify("minecraft:short_grass", false, None), Cell::Air);
        assert_eq!(classify("minecraft:water", false, Water), Cell::Water);
        assert_eq!(classify("minecraft:seagrass", false, Water), Cell::Water);
        assert_eq!(classify("minecraft:lava", false, Lava), Cell::Lava);
        assert_eq!(classify("minecraft:magma_block", true, None), Cell::Hazard);
        assert_eq!(classify("minecraft:fire", false, None), Cell::Hazard);
        assert_eq!(classify("minecraft:cactus", false, None), Cell::Hazard);
        assert_eq!(classify("mymod:magma_block", true, None), Cell::Solid);
    }

    const HERE: [f64; 3] = [10.7, 64.0, -3.2];

    fn parsed(c: &str) -> Result<Cmd, String> {
        parse_command(c, HERE).expect("ours")
    }

    #[test]
    fn other_commands_are_left_alone() {
        assert!(parse_command("gamemode creative", HERE).is_none());
        assert!(parse_command("", HERE).is_none());
        assert!(parse_command("gotomorrow 1 2 3", HERE).is_none());
    }

    #[test]
    fn goto_forms() {
        assert_eq!(parsed("goto 5 70 -9"), Ok(Cmd::Goto(Goal::Exact([5, 70, -9]))));
        assert_eq!(parsed("goto 5 -9"), Ok(Cmd::Goto(Goal::Column(5, -9))));
        assert_eq!(parsed("goto 1.9 2 3.5"), Ok(Cmd::Goto(Goal::Exact([1, 2, 3]))));
        assert_eq!(
            parsed("goto 5 70 -9 3"),
            Ok(Cmd::Goto(Goal::Near([5, 70, -9], 3.0)))
        );
    }

    #[test]
    fn goto_relative_coordinates() {
        assert_eq!(parsed("goto ~ ~ ~"), Ok(Cmd::Goto(Goal::Exact([10, 64, -4]))));
        assert_eq!(parsed("goto ~10 ~-2 ~"), Ok(Cmd::Goto(Goal::Exact([20, 62, -4]))));
        assert_eq!(parsed("goto ~5 ~"), Ok(Cmd::Goto(Goal::Column(15, -4))));
    }

    #[test]
    fn goto_errors_are_readable() {
        assert!(parsed("goto").unwrap_err().starts_with("Usage"));
        assert!(parsed("goto 1 2 3 4 5").unwrap_err().starts_with("Usage"));
        assert!(parsed("goto a b c").unwrap_err().contains("not a coordinate"));
        assert!(parsed("goto 1 2 3 -1").unwrap_err().contains("radius"));
        assert!(parsed("goto ~x 2 3").unwrap_err().contains("not a coordinate"));
    }

    #[test]
    fn path_commands() {
        assert_eq!(parsed("path stop"), Ok(Cmd::Stop));
        assert_eq!(parsed("path status"), Ok(Cmd::Status));
        assert_eq!(parsed("path"), Ok(Cmd::Help));
        assert_eq!(parsed("path parkour on"), Ok(Cmd::Set(Setting::Parkour(true))));
        assert_eq!(parsed("path swim off"), Ok(Cmd::Set(Setting::Swim(false))));
        assert_eq!(parsed("path look on"), Ok(Cmd::Set(Setting::Look(true))));
        assert_eq!(parsed("path fall 5"), Ok(Cmd::Set(Setting::MaxFall(5))));
        assert_eq!(parsed("path nodes 5000"), Ok(Cmd::Set(Setting::MaxNodes(5000))));
        assert!(parsed("path fall 99").is_err());
        assert!(parsed("path nodes 5").is_err());
        assert!(parsed("path parkour maybe").is_err());
        assert!(parsed("path dance").is_err());
    }

    #[test]
    fn settings_change_the_options() {
        let mut o = Options::default();
        let mut sprint = true;
        apply_setting(&mut o, &mut sprint, &Setting::Parkour(true));
        apply_setting(&mut o, &mut sprint, &Setting::MaxFall(5));
        apply_setting(&mut o, &mut sprint, &Setting::Sprint(false));
        assert!(o.allow_parkour && o.max_fall == 5 && !sprint);
    }

    #[test]
    fn describe_follows_the_phases() {
        let w = Mock::new();
        let mut p = Pather::new(Goal::Exact([4, 1, 0]), Options::default());
        assert_eq!(p.describe(), "about to plan");
        p.tick(&w, [0.5, 1.0, 0.5], true, false, None);
        assert!(p.describe().contains("steps to go"), "{}", p.describe());
    }
}