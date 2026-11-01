# Pathing: a Baritone-style pathfinder for Horizon

A native Horizon mod (`.hnmod`). `/goto x y z` and the player walks there:
A* over the loaded world, then keys and view angle driven tick by tick, the way
the old `horizon-ctl goto` did.

## Commands

    /goto <x> <y> <z>           walk to a block (~ is relative: /goto ~20 ~ ~)
    /goto <x> <z>               walk to a column, at any height
    /goto <x> <y> <z> <radius>  get within a radius of a block
    /path stop | status | help
    /path parkour|swim|diagonal|sprint on|off
    /path fall <blocks>         longest planned drop (default 3, which never hurts)
    /path nodes <count>         search size per plan (default 20000)

## What it does

- Moves: walk, diagonal (no corner cutting), step up one block, drop up to
  `fall` blocks, swim, and (off by default) sprint-jump across gaps of 1-2.
- Avoids lava, fire, cactus, magma, berry bushes, cobweb, powder snow and
  unloaded chunks. Never plans a fall longer than `fall`.
- Plans a few hundred nodes a tick, so the game doesn't stall. If a plan runs
  out of nodes it walks to the closest point and plans again from there.
- Replans up to three times when stuck (no progress for 3 s, or far off path).
- Stops if a screen opens. `/path stop` releases the keys.

## What it does not do (yet)

- No breaking or placing blocks (Baritone's mine/bridge/tower). The search has
  no inventory or tool model; that's the next piece.
- Partial blocks are not modelled: slabs, stairs, fences, walls, doors and
  carpets count as air unless the game calls them solid. Expect trouble on
  stairs and slabs, and walking into fences.
- No ladders, vines, scaffolding, boats or elytra. No hunger, mob or
  damage awareness.
- Parkour jump timing is a first guess and needs tuning in a real game.

## Layout

    mixins/pathing/runtime.rs       the pathfinder: pure Rust, no game types.
                                    Also the Cargo library, so `cargo test`
                                    runs its 34 tests.
    mixins/pathing/glue.rs          the game side: reads blocks, holds keys
    mixins/pathing/{tick,command,commands,list}.rs   the four hooks
    mixins/pathing/horizon-mixin.toml, compatibility.json

## Build and run

Put the `pathing/` folder (this one) in `horizon-mods/native/` and, from that
repo:

    python3 native/tools/native.py check pathing    # applies it, cargo-checks the game
    python3 native/tools/native.py build pathing    # builds a game with it; prints the binary
    python3 native/tools/native.py package pathing  # a .hnmod

It builds on the Horizon commit in `compatibility.json` (a4873dec, a Nightly of
2026-10-31). Don't pass `--horizon latest` without checking: see below.

## Why it is pinned

`glue.rs` reads fields of `Game` that are not hook points (`world`, `res`,
`keys`, `yaw`, `player`, `screen`), the same ones `game/agent.rs` uses. A mod
module inside `crate::game` may do that, but those fields can change in any
commit, so this is not a "builds on any later Horizon" mod. If a newer commit
breaks it, the compile error will name the field.

## Tested

- `cargo test` (edition 2024, Rust 1.99): 34 tests pass. They cover the search,
  every move type, the follower, the replanning loop against a toy player, the
  block classifier and the command parser.
- The mod applied to Horizon a4873dec with horizon-mixins a2af5c0 and the whole
  game compiled with `cargo check -p horizon-client` under `-D warnings`: ok.
- **Not tested: running it.** I couldn't launch the game, so nothing here has
  walked a real player. Expect to tune the follower (jump timing, arrival
  tolerances) and to find block types the classifier gets wrong.
