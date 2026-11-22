# Architecture

## Crates

| Crate | Role |
|---|---|
| `aerugo` | The executable: command line, application loop, developer viewers |
| `aerugo-platform` | SDL3 window and events, keyboard and gamepad input, audio output |
| `aerugo-gfx` | Vulkan: instance, device, swapchain, frames in flight, sprites, textures, distance-field text |
| `aerugo-unity` | Reading the game's Unity asset bundles: bundles, serialized files, object values, textures |
| `aerugo-ui` | The game's 2D interface: object trees loaded from the UI assets, anchor layout on the 1920x1080 canvas, image quads, text materials and layout, message tables, fonts |
| `aerugo-audio` | The game's sound banks: FSB5 reading, Vorbis setup headers from the installation, decoding to PCM |
| `aerugo-media` | The game's movies: ASF demuxing, WMV3 video and WMA v2 audio decoders, background decoding for playback |
| `aerugo-sim` | Game logic: fixed tick, the process scheduler and per-tick kernel, sequences, the fade, input (pad and keyboard checks), the movie manager, the title; more systems as they are implemented |

### Dependency rules

- Format readers do no rendering and no window work.
- Game logic (`aerugo-sim`) never depends on graphics, audio or the window. It is deterministic and
  testable without a GPU.
- Game logic talks to the screen through small traits in its own terms (show the logo, move the title
  graphics). The executable implements them on top of `aerugo-ui`, which is the only place that knows how
  the original's UI assets are laid out.
- `unsafe` code is limited to `aerugo-gfx` and `aerugo-platform`. Every `unsafe` block carries a
  `SAFETY:` comment.

## Screen: the 16:9 canvas

The original lays its 2D screens out on a 1920x1080 canvas and always shows a 16:9 picture. Aerugo does the
same for now: the canvas is scaled to the largest 16:9 area of the window, with bars on wider or narrower
windows. Using the full width of ultrawide displays is planned.

## Text

Text components are laid out the way the original's text component does it (`aerugo-ui::text`): glyph quads from
the font's metrics and the material's padding, word wrapping, then the block anchored in the component's
rectangle and each line justified. The renderer draws them with a GLSL transcription of the game's own
distance-field text shader. The original draws its screens as an overlay canvas; Aerugo evaluates the shader with
matrices that map canvas units to the window, so the shader's anti-aliasing width follows the canvas scale (at
1920x1080 this is exactly the original's setup). Fonts and materials follow the original's per-language rules:
objects marked to switch fonts take the language's font, and keep their material only if it samples that
font's atlas.

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
