# Roadmap

## Milestone 1: foundations (in progress)

- [x] SDL3 window, keyboard and gamepad input, audio output stream
- [x] Vulkan 1.3 renderer: swapchain at any size, uncapped present modes, textured sprites
- [x] Fixed 30 Hz simulation clock with interpolation factor
- [x] Unity asset bundle reader (LZ4 bundles, serialized files, type trees, textures)
- [x] The game's random number generator
- [ ] Automatic detection of the Steam installation
- [x] Process scheduler that runs the game's systems each tick
- [x] Sequence director and boot into the title
- [x] UI object trees from the game's layouts, 16:9 canvas, image drawing, full-screen fade
- [x] Opening: warning screen (Japanese), Atlus logo
- [x] Keyboard and gamepad buttons to the game's pad (skipping logos and movies); mouse to do
- [x] Sound banks decode (all 12,552 samples of the installation)
- [x] Audio output: mixer with the original's four groups, SDL3 stream
- [x] Sound id tables (numbers and names to sounds)
- [ ] The game's sound manager: channels, music fades and cross-fades
- [x] Movies: ASF container (all 37 movies demux)
- [x] Movies: WMA v2 audio decoder of our own
- [x] Movies: WMV3 (VC-1) video decoder of our own (every frame of all 37 movies checked)
- [x] Movie player: background decoding, sound through the movie group, frames converted on the GPU
- [x] Title: opening movies, press-start movie and the loop back to the logos
- [x] Title name over the movies, "press start" animation
- [x] Text: message tables, font assets, distance-field text shader, the text component's layout (plain text,
      word wrapping, alignment, margins)
- [x] UI layout groups (vertical/horizontal, layout elements, content size fitters), canvas group alpha
- [x] Text material tags (`<material="...">`) with materials from the player's resources
- [x] Main menu: entrance animation, entries by mode and installed content, cursor, help text, cancel back to
      "press start", confirm fade
- [ ] Main menu entries' actions (new game, load game, config, ...), pointer input, button guide
- [ ] Sliced images (dialog windows)
- [ ] Text colour tags, fallback fonts, line-breaking rules for Japanese, Chinese and Korean
- [ ] The UI brightness/contrast material the original applies to its screens

## Later milestones

- Menus
- Audio: the game's sound banks, music and voices
- Models, animation and the field
- Events and scripts
- Battle, demons and fusion
- Movies in events
