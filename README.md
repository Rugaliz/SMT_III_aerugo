# Aerugo: Shin Megami Tensei III

An open-source, from-scratch engine reimplementation of **Shin Megami Tensei III: Nocturne HD Remaster**,
written in Rust.

Aerugo reads the game data from your own installed copy of the Steam release and runs it with a new engine:
a Vulkan renderer, SDL3 for windowing, input and audio, and a faithful reimplementation of the game logic.

> **Status: early development.** Nothing is playable yet. The current milestone builds the foundations:
> window, Vulkan renderer, fixed-tick clock, reading the game's asset bundles. See [docs/ROADMAP.md](docs/ROADMAP.md).

## Goals

- Faithful game logic, tick for tick.
- The game's art as designed, drawn by a modern renderer.
- Any resolution and aspect ratio, ultrawide up to 32:9.
- No frame-rate cap: the simulation keeps the original 30 ticks per second and every displayed frame
  interpolates between ticks, including animation.
- Linux and Windows.

## You need your own copy of the game

Aerugo contains **no game data**. It needs the Steam release of *Shin Megami Tensei III: Nocturne HD
Remaster* installed on your machine. Point Aerugo at the installation folder (the one containing
`smt3hd_Data`) with `--game <folder>` or the `AERUGO_GAME_DIR` environment variable.
Please do not open issues asking for game files.

## Building

Requirements:

- Rust 1.97 or newer (`rustup` picks the pinned toolchain automatically)
- A Vulkan 1.3 capable GPU and driver
- SDL3 3.2 or newer as a system library
- `glslc` (from shaderc) to compile the shaders

```sh
cargo run --release -- --game "/path/to/smt3hd"
```

Useful while developing:

```sh
cargo run --release -- --game "/path/to/smt3hd" view-texture title t_logo   # show one texture
cargo run --release -- --vsync                                              # cap to the display rate
```

`F11` toggles fullscreen.

## Legal

Aerugo is an independent project, not affiliated with or endorsed by ATLUS or SEGA. *Shin Megami Tensei* is a
trademark of ATLUS. The code is MIT-licensed (see [LICENSE](LICENSE)); the game's data is not part of this
repository and is not covered by that license.
