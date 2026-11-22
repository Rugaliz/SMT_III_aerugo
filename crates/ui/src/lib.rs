//! The game's 2D interface.
//!
//! The original screens (logos, title, menus) are authored as trees of UI objects in the game's asset
//! bundles. This crate rebuilds those trees ([`UiTree`]), lays them out on the 1920x1080 design canvas and
//! turns the visible images into textured quads ([`UiTree::draw_list`]). Game logic changes the tree through
//! small operations (show or hide an object, move it, tint an image) and never sees the asset format.

pub mod anim;
mod color;
mod draw;
pub mod font;
pub mod layout;
pub mod localize;
pub mod material;
mod math;
mod prefab;
pub mod text;
mod tree;

pub use color::{Color, Color32};
pub use draw::{Canvas, UiDraw, UiQuad, UiText};
pub use math::Affine;
pub use prefab::{Prefab, TextureRef};
pub use tree::{AssetRef, Image, Node, NodeId, RectTransform, Sprite, Text, UiTree};
