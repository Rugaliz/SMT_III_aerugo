# Architecture

## Crates

| Crate | Role |
|---|---|
| `aerugo` | The executable: command line, application loop, developer viewers |
| `aerugo-platform` | SDL3 window and events, keyboard and gamepad input, audio output |
| `aerugo-gfx` | Vulkan: instance, device, swapchain, frames in flight, sprites, textures |
| `aerugo-unity` | Reading the game's Unity asset bundles: bundles, serialized files, object values, textures |
| `aerugo-sim` | Game logic: fixed tick, random numbers; more systems as they are implemented |

### Dependency rules

- Format readers do no rendering and no window work.
- Game logic (`aerugo-sim`) never depends on graphics, audio or the window. It is deterministic and
  testable without a GPU.
- `unsafe` code is limited to `aerugo-gfx` and `aerugo-platform`. Every `unsafe` block carries a
  `SAFETY:` comment.

## Time: fixed tick, interpolated frames

The game's logic counts frames at a fixed rate (30 per second), and its animations advance in 1/30 s steps.
Aerugo runs the simulation at exactly that rate regardless of the display:

1. Real time accumulates; for each whole tick period the simulation advances one tick (at most a few per
   frame, so a slow frame slows the game down instead of skipping logic).
2. Every displayed frame interpolates by the fraction of the next tick that has elapsed. Animation curves
   are sampled at that sub-tick time instead of blending poses.

Rendering is not capped: present mode MAILBOX, then IMMEDIATE, or FIFO with `--vsync`.

## Display

The swapchain follows the window size at any aspect ratio. Screens built from the game's own layouts keep
their design area and extend to the sides on wider displays.

## Game data

Everything is read from the user's installation at run time: the asset bundles under
`smt3hd_Data/StreamingAssets/PC`. Block-compressed textures go to the GPU unchanged.
