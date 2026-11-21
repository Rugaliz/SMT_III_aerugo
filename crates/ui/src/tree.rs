//! The UI object tree: what exists, what is shown, and where.

use crate::color::Color;
use crate::draw::Canvas;
use crate::math::Affine;
use crate::prefab::{Prefab, TextureRef};

/// Handle to a node of a [`UiTree`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(pub(crate) u32);

/// Placement of a node relative to its parent.
///
/// Rect nodes are laid out from anchors: the node's size is `size_delta` plus the span between its two anchor
/// points in the parent's rectangle, and its pivot sits at `anchored_position` from the point between the
/// anchors selected by the pivot. Plain nodes have no rectangle and sit at `local_position`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectTransform {
    pub is_rect: bool,
    pub anchor_min: [f32; 2],
    pub anchor_max: [f32; 2],
    pub anchored_position: [f32; 2],
    pub size_delta: [f32; 2],
    pub pivot: [f32; 2],
    /// Full local position. For rect nodes only `z` is used; x and y follow from the anchors.
    pub local_position: [f32; 3],
    /// Unit quaternion x, y, z, w.
    pub local_rotation: [f32; 4],
    pub local_scale: [f32; 3],
}

impl Default for RectTransform {
    fn default() -> Self {
        Self {
            is_rect: true,
            anchor_min: [0.5; 2],
            anchor_max: [0.5; 2],
            anchored_position: [0.0; 2],
            size_delta: [100.0; 2],
            pivot: [0.5; 2],
            local_position: [0.0; 3],
            local_rotation: [0.0, 0.0, 0.0, 1.0],
            local_scale: [1.0; 3],
        }
    }
}

/// An image region of a texture.
#[derive(Clone, Debug, PartialEq)]
pub struct Sprite {
    pub texture: TextureRef,
    /// Texture size in pixels.
    pub texture_size: [f32; 2],
    /// The sprite's full rectangle (x, y, width, height) in texture pixels, origin bottom-left.
    pub rect: [f32; 4],
    /// The part of the texture actually stored for the sprite.
    pub texture_rect: [f32; 4],
    /// Offset of `texture_rect` inside `rect`.
    pub texture_rect_offset: [f32; 2],
}

impl Sprite {
    /// Texture coordinates of the stored rectangle: u0, v0, u1, v1 (v up).
    pub fn outer_uv(&self) -> [f32; 4] {
        let [x, y, w, h] = self.texture_rect;
        let [tw, th] = self.texture_size;
        [x / tw, y / th, (x + w) / tw, (y + h) / th]
    }

    /// Transparent margins between `rect` and the stored rectangle: left, bottom, right, top.
    pub fn padding(&self) -> [f32; 4] {
        let [ox, oy] = self.texture_rect_offset;
        let (rw, rh, tw, th) = (self.rect[2], self.rect[3], self.texture_rect[2], self.texture_rect[3]);
        [ox, oy, rw - tw - ox, rh - th - oy]
    }
}

/// A drawable image component.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub enabled: bool,
    pub color: Color,
    /// Index into the tree's sprites; `None` draws a plain colored rectangle.
    pub sprite: Option<usize>,
    /// 0 simple, 1 sliced, 2 tiled, 3 filled.
    pub kind: i32,
    pub preserve_aspect: bool,
    pub use_sprite_mesh: bool,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    pub active_self: bool,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub transform: RectTransform,
    pub image: Option<Image>,
    alive: bool,
}

/// A tree of UI nodes under a root canvas.
#[derive(Debug)]
pub struct UiTree {
    nodes: Vec<Node>,
    pub sprites: Vec<Sprite>,
    pub canvas: Canvas,
}

impl UiTree {
    /// A tree holding only the root canvas node, named `root_name`.
    pub fn new(root_name: &str, canvas: Canvas) -> Self {
        let root = Node {
            name: root_name.to_string(),
            active_self: true,
            parent: None,
            children: Vec::new(),
            transform: RectTransform::default(),
            image: None,
            alive: true,
        };
        Self { nodes: vec![root], sprites: Vec::new(), canvas }
    }

    pub fn root(&self) -> NodeId {
        NodeId(0)
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0 as usize]
    }

    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id.0 as usize]
    }

    /// Copy a prefab under `parent` (as its last child), keeping the prefab's local placement.
    pub fn instantiate(&mut self, prefab: &Prefab, parent: NodeId) -> NodeId {
        let base = self.nodes.len() as u32;
        let sprite_base = self.sprites.len();
        self.sprites.extend(prefab.sprites.iter().cloned());
        for (i, n) in prefab.nodes.iter().enumerate() {
            let mut image = n.image.clone();
            if let Some(img) = &mut image {
                img.sprite = img.sprite.map(|s| s + sprite_base);
            }
            self.nodes.push(Node {
                name: n.name.clone(),
                active_self: n.active,
                parent: Some(match n.parent {
                    Some(p) => NodeId(base + p as u32),
                    None => parent,
                }),
                children: n.children.iter().map(|&c| NodeId(base + c as u32)).collect(),
                transform: n.transform,
                image,
                alive: true,
            });
            debug_assert_eq!(self.nodes.len() as u32, base + i as u32 + 1);
        }
        let root = NodeId(base);
        self.nodes[parent.0 as usize].children.push(root);
        root
    }

    /// Remove a node and everything below it.
    pub fn destroy(&mut self, id: NodeId) {
        if let Some(p) = self.node(id).parent {
            self.node_mut(p).children.retain(|&c| c != id);
        }
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            let node = self.node_mut(n);
            node.alive = false;
            stack.extend(std::mem::take(&mut node.children));
        }
    }

    pub fn is_alive(&self, id: NodeId) -> bool {
        self.node(id).alive
    }

    /// The first direct child named `name`.
    pub fn child(&self, id: NodeId, name: &str) -> Option<NodeId> {
        self.node(id).children.iter().copied().find(|&c| self.node(c).name == name)
    }

    /// Follow a `/`-separated path of child names from `id`. Hidden nodes are found too.
    pub fn find(&self, id: NodeId, path: &str) -> Option<NodeId> {
        path.split('/').try_fold(id, |n, part| self.child(n, part))
    }

    /// The first node named `name` anywhere below the root (depth first).
    pub fn find_anywhere(&self, name: &str) -> Option<NodeId> {
        let mut stack = vec![self.root()];
        while let Some(n) = stack.pop() {
            if self.node(n).name == name {
                return Some(n);
            }
            stack.extend(self.node(n).children.iter().rev());
        }
        None
    }

    pub fn set_active(&mut self, id: NodeId, active: bool) {
        self.node_mut(id).active_self = active;
    }

    pub fn active_self(&self, id: NodeId) -> bool {
        self.node(id).active_self
    }

    /// Shown: the node and all its ancestors are active.
    pub fn active_in_hierarchy(&self, id: NodeId) -> bool {
        let mut cur = Some(id);
        while let Some(n) = cur {
            if !self.node(n).active_self {
                return false;
            }
            cur = self.node(n).parent;
        }
        true
    }

    /// Set the local position. For rect nodes this moves the anchored position so that the pivot lands
    /// on `p` in the parent's space.
    pub fn set_local_position(&mut self, id: NodeId, p: [f32; 3]) {
        let reference = match self.node(id).parent {
            Some(parent) if self.node(id).transform.is_rect => {
                let pr = self.rect(parent);
                anchor_reference(&self.node(id).transform, pr)
            }
            _ => [0.0, 0.0],
        };
        let t = &mut self.node_mut(id).transform;
        t.local_position = p;
        t.anchored_position = [p[0] - reference[0], p[1] - reference[1]];
    }

    pub fn set_image_color(&mut self, id: NodeId, c: Color) {
        if let Some(img) = &mut self.node_mut(id).image {
            img.color = c;
        }
    }

    /// The node's rectangle in its own space: x, y (bottom-left corner relative to the pivot), width, height.
    pub fn rect(&self, id: NodeId) -> [f32; 4] {
        self.layout(id).0
    }

    /// Rectangle in own space and transform to the parent's space.
    pub(crate) fn layout(&self, id: NodeId) -> ([f32; 4], Affine) {
        let n = self.node(id);
        let t = &n.transform;
        let Some(parent) = n.parent else {
            let [w, h] = [self.canvas.width, self.canvas.height];
            return ([-w * 0.5, -h * 0.5, w, h], Affine::IDENTITY);
        };
        if !t.is_rect {
            return ([0.0; 4], Affine::trs(t.local_position, t.local_rotation, t.local_scale));
        }
        let pr = self.rect(parent);
        let size = [
            pr[2] * (t.anchor_max[0] - t.anchor_min[0]) + t.size_delta[0],
            pr[3] * (t.anchor_max[1] - t.anchor_min[1]) + t.size_delta[1],
        ];
        let r = anchor_reference(t, pr);
        let pos = [r[0] + t.anchored_position[0], r[1] + t.anchored_position[1], t.local_position[2]];
        let rect = [-t.pivot[0] * size[0], -t.pivot[1] * size[1], size[0], size[1]];
        (rect, Affine::trs(pos, t.local_rotation, t.local_scale))
    }
}

/// The point in the parent's rectangle `pr` that the anchored position is measured from.
fn anchor_reference(t: &RectTransform, pr: [f32; 4]) -> [f32; 2] {
    let lerp = |a: f32, b: f32, s: f32| a + (b - a) * s;
    [
        pr[0] + pr[2] * lerp(t.anchor_min[0], t.anchor_max[0], t.pivot[0]),
        pr[1] + pr[3] * lerp(t.anchor_min[1], t.anchor_max[1], t.pivot[1]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefab::PrefabNode;

    fn rt(
        anchor_min: [f32; 2],
        anchor_max: [f32; 2],
        pos: [f32; 2],
        size: [f32; 2],
        pivot: [f32; 2],
    ) -> RectTransform {
        RectTransform {
            anchor_min,
            anchor_max,
            anchored_position: pos,
            size_delta: size,
            pivot,
            ..Default::default()
        }
    }

    fn prefab(nodes: Vec<PrefabNode>) -> Prefab {
        Prefab { nodes, sprites: Vec::new() }
    }

    #[test]
    fn anchors_place_and_size_children() {
        let mut ui = UiTree::new("canvas", Canvas::DESIGN);
        let p = prefab(vec![
            PrefabNode::new(
                "stretch",
                None,
                vec![1],
                rt([0.0; 2], [1.0; 2], [0.0; 2], [-20.0, -40.0], [0.5; 2]),
            ),
            PrefabNode::new(
                "corner",
                Some(0),
                vec![],
                rt([1.0; 2], [1.0; 2], [-5.0, -5.0], [10.0, 10.0], [1.0; 2]),
            ),
        ]);
        let root = ui.instantiate(&p, ui.root());
        assert_eq!(ui.rect(root), [-950.0, -520.0, 1900.0, 1040.0]);
        let corner = ui.find(root, "corner").unwrap();
        let (rect, to_parent) = ui.layout(corner);
        assert_eq!(rect, [-10.0, -10.0, 10.0, 10.0]);
        // Pivot at the top-right anchor of a 1900x1040 rect, pulled in by 5.
        assert_eq!(to_parent.apply([0.0; 3]), [945.0, 515.0, 0.0]);
    }

    #[test]
    fn setting_the_local_position_updates_the_anchored_position() {
        let mut ui = UiTree::new("canvas", Canvas::DESIGN);
        let p = prefab(vec![PrefabNode::new(
            "n",
            None,
            vec![],
            rt([0.0, 1.0], [0.0, 1.0], [0.0; 2], [100.0, 100.0], [0.0, 1.0]),
        )]);
        let n = ui.instantiate(&p, ui.root());
        ui.set_local_position(n, [0.0, 2160.0, 0.0]);
        // Anchor at the canvas top-left (-960, 540).
        assert_eq!(ui.node(n).transform.anchored_position, [960.0, 1620.0]);
        assert_eq!(ui.layout(n).1.apply([0.0; 3]), [0.0, 2160.0, 0.0]);
    }

    #[test]
    fn find_follows_paths_and_sees_hidden_nodes() {
        let mut ui = UiTree::new("canvas", Canvas::DESIGN);
        let mut a = PrefabNode::new("a", None, vec![1], RectTransform::default());
        a.active = true;
        let mut b = PrefabNode::new("b", Some(0), vec![], RectTransform::default());
        b.active = false;
        let root = ui.instantiate(&prefab(vec![a, b]), ui.root());
        let b = ui.find(ui.root(), "a/b").unwrap();
        assert!(!ui.active_in_hierarchy(b));
        ui.set_active(b, true);
        assert!(ui.active_in_hierarchy(b));
        ui.set_active(root, false);
        assert!(!ui.active_in_hierarchy(b));
        assert_eq!(ui.find_anywhere("b"), Some(b));
        ui.destroy(root);
        assert_eq!(ui.find(ui.root(), "a"), None);
        assert!(!ui.is_alive(b));
    }
}
