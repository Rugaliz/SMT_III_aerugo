//! Reader for the Unity 2019.4 asset formats of the game installation:
//! UnityFS bundles, SerializedFiles and typetree-driven object values.

pub mod archive;
pub mod bundle;
pub mod reader;
pub mod serialized;
pub mod typetree;

pub use archive::{Archive, ObjRef, Texture2D, TextureFormat};
pub use bundle::{Bundle, BundleNode, is_bundle};
pub use serialized::{ObjectInfo, SerializedFile, looks_serialized};
pub use typetree::{ReadOpts, TypeTree, ValueReader};
