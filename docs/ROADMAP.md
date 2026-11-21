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
- [ ] Mapping keys and buttons to the game's pad (skipping logos)
- [x] Sound banks decode (all 12,552 samples of the installation)
- [x] Audio output: mixer with the original's four groups, SDL3 stream
- [x] Sound id tables (numbers and names to sounds)
- [ ] The game's sound manager: channels, music fades and cross-fades
- [x] Movies: ASF container (all 37 movies demux)
- [x] Movies: WMA v2 audio decoder of our own
- [x] Movies: WMV3 (VC-1) video decoder of our own (every frame of all 37 movies checked)
- [x] Movie player: background decoding, sound through the movie group, frames converted on the GPU
- [x] Title: opening movies, press-start movie and the loop back to the logos
- [ ] Title name over the movies
- [ ] Title screen and main menu
- [ ] The UI brightness/contrast material the original applies to its screens

## Later milestones

- Text and fonts, menus
- Audio: the game's sound banks, music and voices
- Models, animation and the field
- Events and scripts
- Battle, demons and fusion
- Movies in events
