//! Automatic layout of the original's UI: layout groups that stack their children, layout elements that
//! state sizes, size fitters that size an object to its content, and canvas groups that fade a subtree.

use crate::tree::{NodeId, UiTree};

/// A horizontal or vertical layout group.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutGroup {
    pub enabled: bool,
    pub vertical: bool,
    /// Left, right, top, bottom.
    pub padding: [i32; 4],
    pub spacing: f32,
    /// 0..8: upper left, upper center, upper right, middle left, …
    pub child_alignment: i32,
    /// Per axis (x, y): the group sets the children's sizes.
    pub control_size: [bool; 2],
    pub force_expand: [bool; 2],
    pub use_child_scale: [bool; 2],
    /// Computed: total minimum, preferred and flexible size per axis.
    pub totals: [[f32; 3]; 2],
}

/// Size requests of one object.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutElement {
    pub enabled: bool,
    pub ignore_layout: bool,
    /// Per axis (x, y); negative: not set.
    pub min: [f32; 2],
    pub preferred: [f32; 2],
    pub flexible: [f32; 2],
    pub priority: i32,
}

/// Sizes an object to its content: per axis 0 unconstrained, 1 minimum size, 2 preferred size.
#[derive(Clone, Debug, PartialEq)]
pub struct SizeFitter {
    pub enabled: bool,
    pub fit: [i32; 2],
}

/// Fades everything below it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasGroup {
    pub enabled: bool,
    pub alpha: f32,
    pub ignore_parent_groups: bool,
}

/// Which size a layout property asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Min,
    Preferred,
    Flexible,
}

impl UiTree {
    /// Alpha of a node from the canvas groups above it (itself included): the product of the enabled groups'
    /// alpha up to and including the first group that ignores its parents.
    pub fn inherited_alpha(&self, id: NodeId) -> f32 {
        let mut a = 1.0;
        let mut cur = Some(id);
        while let Some(n) = cur {
            let node = self.node(n);
            if let Some(g) = &node.canvas_group
                && g.enabled
            {
                a *= g.alpha;
                if g.ignore_parent_groups {
                    break;
                }
            }
            cur = node.parent;
        }
        a
    }

    /// Lay out every active layout group and size fitter: sizes are computed children first and applied
    /// parents first, the horizontal axis before the vertical one.
    pub fn rebuild_layout(&mut self) {
        let order = self.active_pre_order();
        for axis in 0..2 {
            for &id in order.iter().rev() {
                self.calc_along_axis(id, axis);
            }
            for &id in &order {
                self.fit_along_axis(id, axis);
                self.set_children_along_axis(id, axis);
            }
        }
    }

    fn active_pre_order(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack = vec![self.root()];
        while let Some(n) = stack.pop() {
            if !self.node(n).active_self {
                continue;
            }
            out.push(n);
            stack.extend(self.node(n).children.iter().rev());
        }
        out
    }

    /// Children a layout group arranges: active, and not all of their layout elements ignoring layout.
    fn rect_children(&self, id: NodeId) -> Vec<NodeId> {
        self.node(id)
            .children
            .iter()
            .copied()
            .filter(|&c| {
                let n = self.node(c);
                n.transform.is_rect
                    && self.active_in_hierarchy(c)
                    && !n.layout_element.as_ref().is_some_and(|e| e.ignore_layout)
            })
            .collect()
    }

    /// A layout property of a node: the value of its highest-priority size provider that states one (≥ 0);
    /// equal priorities keep the larger value. 0 when none does.
    fn layout_property(&self, id: NodeId, axis: usize, kind: Kind) -> f32 {
        let n = self.node(id);
        let mut best: Option<(i32, f32)> = None;
        let mut consider = |priority: i32, v: f32| {
            if v < 0.0 {
                return;
            }
            match best {
                Some((p, _)) if priority < p => {}
                Some((p, cur)) if priority == p => {
                    if v > cur {
                        best = Some((p, v));
                    }
                }
                _ => best = Some((priority, v)),
            }
        };
        if let Some(e) = &n.layout_element
            && e.enabled
        {
            let v = match kind {
                Kind::Min => e.min[axis],
                Kind::Preferred => e.preferred[axis],
                Kind::Flexible => e.flexible[axis],
            };
            consider(e.priority, v);
        }
        if let Some(g) = &n.layout_group
            && g.enabled
        {
            let i = match kind {
                Kind::Min => 0,
                Kind::Preferred => 1,
                Kind::Flexible => 2,
            };
            consider(0, g.totals[axis][i]);
        }
        let others =
            n.image.as_ref().is_some_and(|i| i.enabled) || n.text.as_ref().is_some_and(|t| t.enabled);
        if others && best.is_none_or(|(p, _)| p <= 0) {
            log::warn!("{}: image and text sizes in automatic layouts are not supported yet", n.name);
        }
        best.map_or(0.0, |(_, v)| v)
    }

    fn min_size(&self, id: NodeId, axis: usize) -> f32 {
        self.layout_property(id, axis, Kind::Min)
    }

    fn preferred_size(&self, id: NodeId, axis: usize) -> f32 {
        self.min_size(id, axis).max(self.layout_property(id, axis, Kind::Preferred))
    }

    fn flexible_size(&self, id: NodeId, axis: usize) -> f32 {
        self.layout_property(id, axis, Kind::Flexible)
    }

    /// A child's (min, preferred, flexible) size along `axis` for group `g`.
    fn child_sizes(&self, g: &LayoutGroup, child: NodeId, axis: usize) -> (f32, f32, f32) {
        let (min, pref, mut flex) = if g.control_size[axis] {
            (self.min_size(child, axis), self.preferred_size(child, axis), self.flexible_size(child, axis))
        } else {
            let s = self.node(child).transform.size_delta[axis];
            (s, s, 0.0)
        };
        if g.force_expand[axis] {
            flex = flex.max(1.0);
        }
        (min, pref, flex)
    }

    fn child_scale(&self, g: &LayoutGroup, child: NodeId, axis: usize) -> f32 {
        if g.use_child_scale[axis] { self.node(child).transform.local_scale[axis] } else { 1.0 }
    }

    fn calc_along_axis(&mut self, id: NodeId, axis: usize) {
        let Some(g) = self.node(id).layout_group.clone().filter(|g| g.enabled) else { return };
        let padding = padding_sum(&g, axis);
        let along = (axis == 1) == g.vertical;
        let (mut total_min, mut total_pref, mut total_flex) = (padding, padding, 0.0f32);
        let children = self.rect_children(id);
        for &c in &children {
            let (mut min, mut pref, mut flex) = self.child_sizes(&g, c, axis);
            let s = self.child_scale(&g, c, axis);
            min *= s;
            pref *= s;
            flex *= s;
            if along {
                total_min += min + g.spacing;
                total_pref += pref + g.spacing;
                total_flex += flex;
            } else {
                total_min = (min + padding).max(total_min);
                total_pref = (pref + padding).max(total_pref);
                total_flex = flex.max(total_flex);
            }
        }
        if along && !children.is_empty() {
            total_min -= g.spacing;
            total_pref -= g.spacing;
        }
        total_pref = total_min.max(total_pref);
        if let Some(g) = &mut self.node_mut(id).layout_group {
            g.totals[axis] = [total_min, total_pref, total_flex];
        }
    }

    fn fit_along_axis(&mut self, id: NodeId, axis: usize) {
        let Some(fit) = self.node(id).size_fitter.as_ref().filter(|f| f.enabled).map(|f| f.fit[axis]) else {
            return;
        };
        let size = match fit {
            1 => self.min_size(id, axis),
            2 => self.preferred_size(id, axis),
            _ => return,
        };
        self.set_size_with_current_anchors(id, axis, size);
    }

    /// Give the node `size` along `axis`, keeping its anchors.
    fn set_size_with_current_anchors(&mut self, id: NodeId, axis: usize, size: f32) {
        let parent_size = match self.node(id).parent {
            Some(p) => self.rect(p)[2 + axis],
            None => return,
        };
        let t = &mut self.node_mut(id).transform;
        t.size_delta[axis] = size - parent_size * (t.anchor_max[axis] - t.anchor_min[axis]);
    }

    fn set_children_along_axis(&mut self, id: NodeId, axis: usize) {
        let Some(g) = self.node(id).layout_group.clone().filter(|g| g.enabled) else { return };
        let size = self.rect(id)[2 + axis];
        let alignment = alignment_on_axis(&g, axis);
        let children = self.rect_children(id);
        let [total_min, total_pref, total_flex] = g.totals[axis];
        if (axis == 1) == g.vertical {
            let mut pos = padding_start(&g, axis);
            let mut flex_multiplier = 0.0;
            let surplus = size - total_pref;
            if surplus > 0.0 {
                if total_flex == 0.0 {
                    pos = self.start_offset(&g, id, axis, total_pref - padding_sum(&g, axis));
                } else if total_flex > 0.0 {
                    flex_multiplier = surplus / total_flex;
                }
            }
            let lerp = if total_min != total_pref {
                ((size - total_min) / (total_pref - total_min)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            for c in children {
                let (min, pref, flex) = self.child_sizes(&g, c, axis);
                let scale = self.child_scale(&g, c, axis);
                let child_size = min + (pref - min) * lerp + flex * flex_multiplier;
                if g.control_size[axis] {
                    self.place_child(c, axis, pos, Some(child_size), scale);
                } else {
                    let offset = (child_size - self.node(c).transform.size_delta[axis]) * alignment;
                    self.place_child(c, axis, pos + offset, None, scale);
                }
                pos += child_size * scale + g.spacing;
            }
        } else {
            let inner = size - padding_sum(&g, axis);
            for c in children {
                let (min, pref, flex) = self.child_sizes(&g, c, axis);
                let scale = self.child_scale(&g, c, axis);
                let required = clamp(inner, min, if flex > 0.0 { size } else { pref });
                let start = self.start_offset(&g, id, axis, required * scale);
                if g.control_size[axis] {
                    self.place_child(c, axis, start, Some(required), scale);
                } else {
                    let offset = (required - self.node(c).transform.size_delta[axis]) * alignment;
                    self.place_child(c, axis, start + offset, None, scale);
                }
            }
        }
    }

    /// Where a block of `required` size starts along `axis` in the group's rectangle.
    fn start_offset(&self, g: &LayoutGroup, id: NodeId, axis: usize, required: f32) -> f32 {
        let required = required + padding_sum(g, axis);
        let available = self.rect(id)[2 + axis];
        padding_start(g, axis) + (available - required) * alignment_on_axis(g, axis)
    }

    /// Anchor a child at its parent's top-left and put it at `pos` along `axis` (measured from the left or
    /// down from the top), optionally sizing it.
    fn place_child(&mut self, c: NodeId, axis: usize, pos: f32, size: Option<f32>, scale: f32) {
        let t = &mut self.node_mut(c).transform;
        t.anchor_min = [0.0, 1.0];
        t.anchor_max = [0.0, 1.0];
        if let Some(s) = size {
            t.size_delta[axis] = s;
        }
        let s = t.size_delta[axis];
        t.anchored_position[axis] =
            if axis == 0 { pos + s * t.pivot[0] * scale } else { -pos - s * (1.0 - t.pivot[1]) * scale };
    }
}

/// Clamp as the original's math library does it: below `min` gives `min`, else above `max` gives `max` (so `max`
/// wins when the bounds cross).
fn clamp(v: f32, min: f32, max: f32) -> f32 {
    if v < min {
        min
    } else if v > max {
        max
    } else {
        v
    }
}

fn padding_sum(g: &LayoutGroup, axis: usize) -> f32 {
    if axis == 0 { (g.padding[0] + g.padding[1]) as f32 } else { (g.padding[2] + g.padding[3]) as f32 }
}

fn padding_start(g: &LayoutGroup, axis: usize) -> f32 {
    if axis == 0 { g.padding[0] as f32 } else { g.padding[2] as f32 }
}

fn alignment_on_axis(g: &LayoutGroup, axis: usize) -> f32 {
    if axis == 0 { (g.child_alignment % 3) as f32 * 0.5 } else { (g.child_alignment / 3) as f32 * 0.5 }
}
