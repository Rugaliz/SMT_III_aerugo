//! The title's screens, built from the game's UI assets.

use std::cell::RefCell;
use std::rc::Rc;

use aerugo_sim::title::{MENU_ITEMS, PRESS_ANIMATOR, TITLE_SPRITES, TitleView};
use aerugo_ui::{Canvas, Color, NodeId, UiTree};

use crate::assets::Assets;

/// Name of the root UI canvas in the original scene.
pub const ROOT_CANVAS: &str = "Canvas_UI";
/// The main menu group and its parts (paths below the menu screen).
const MENU_GROUP: &str = "titlemain";
const MENU_CURSOR: &str = "titlemain/titlemain_cursur";
const MENU_NAME_WINDOW: &str = "titlemain/namechoice";
fn menu_item(n: usize) -> String {
    format!("titlemain/titlemainmenu/stretch/titlemain_item{n:02}")
}
fn menu_item_text(n: usize) -> String {
    format!("{}/slide/Text_TM", menu_item(n))
}
fn menu_note(n: usize) -> String {
    format!("titlemain/titlemain_note/Text_mainnote{n:02}TM")
}
/// Help line objects.
const MENU_NOTES: usize = 5;

/// Handles into the screens once loaded.
#[derive(Debug, Clone, Copy)]
struct Loaded {
    menu: NodeId,
    atlus_root: NodeId,
    rd_root: NodeId,
    title: Option<NodeId>,
    atlus: Option<NodeId>,
    rd: Option<NodeId>,
    warning: Option<NodeId>,
    sprites: [Option<NodeId>; 7],
    press: Option<NodeId>,
    menu_group: Option<NodeId>,
    items: [Option<NodeId>; MENU_ITEMS],
    item_texts: [Option<NodeId>; MENU_ITEMS],
    notes: [Option<NodeId>; MENU_NOTES],
    cursor: Option<NodeId>,
    name_window: Option<NodeId>,
}

#[derive(Debug)]
pub struct TitleScreens {
    pub ui: UiTree,
    assets: Rc<RefCell<Assets>>,
    loaded: Option<Loaded>,
}

impl TitleScreens {
    pub fn new(assets: Rc<RefCell<Assets>>) -> Self {
        Self { ui: UiTree::new(ROOT_CANVAS, Canvas::DESIGN), assets, loaded: None }
    }

    fn instantiate(&mut self, bundle: &str, dir: &str, name: &str) -> Option<NodeId> {
        let prefab = self.assets.borrow_mut().prefab(bundle, dir, name);
        match prefab {
            Ok(p) => Some(self.ui.instantiate(&p, self.ui.root())),
            Err(e) => {
                log::error!("{bundle}/{dir}/{name}: {e:#}");
                None
            }
        }
    }

    fn set_shown(&mut self, node: Option<NodeId>, shown: bool) {
        if let Some(n) = node {
            self.ui.set_active(n, shown);
        }
    }

    fn shown(&self, node: Option<NodeId>) -> bool {
        node.is_some_and(|n| self.ui.active_self(n))
    }

    /// Object `n` (1-based) of one of the menu's numbered lists.
    fn item<const N: usize>(&self, n: i32, list: impl Fn(&Loaded) -> &[Option<NodeId>; N]) -> Option<NodeId> {
        let l = self.loaded.as_ref()?;
        let i = usize::try_from(n).ok()?.checked_sub(1)?;
        list(l).get(i).copied().flatten()
    }
}

impl TitleView for TitleScreens {
    fn load(&mut self) {
        let (Some(menu), Some(atlus_root), Some(rd_root)) = (
            self.instantiate("common", "dds3data/UI", "titleUI"),
            self.instantiate("title", "dds3data/title", "AtlusLogoUI"),
            self.instantiate("title", "dds3data/title", "RdLogoUI"),
        ) else {
            return;
        };
        self.loaded = Some(Loaded {
            menu,
            atlus_root,
            rd_root,
            title: self.ui.find(menu, "title"),
            atlus: self.ui.find(atlus_root, "atluslogo"),
            rd: self.ui.find(rd_root, "rdlogo"),
            warning: self.ui.find(menu, "WarningState"),
            sprites: TITLE_SPRITES.map(|p| self.ui.find(menu, p)),
            press: self.ui.find(menu, PRESS_ANIMATOR),
            menu_group: self.ui.find(menu, MENU_GROUP),
            items: std::array::from_fn(|i| self.ui.find(menu, &menu_item(i + 1))),
            item_texts: std::array::from_fn(|i| self.ui.find(menu, &menu_item_text(i + 1))),
            notes: std::array::from_fn(|i| self.ui.find(menu, &menu_note(i + 1))),
            cursor: self.ui.find(menu, MENU_CURSOR),
            name_window: self.ui.find(menu, MENU_NAME_WINDOW),
        });
        if let Some(l) = self.loaded {
            let missing = [l.menu_group, l.cursor, l.name_window]
                .into_iter()
                .chain(l.items)
                .chain(l.item_texts)
                .chain(l.notes)
                .filter(Option::is_none)
                .count();
            if missing > 0 {
                log::error!("{missing} main menu objects not found");
            }
        }
        for (i, p) in TITLE_SPRITES.iter().enumerate() {
            if self.loaded.is_some_and(|l| l.sprites[i].is_none()) {
                log::error!("title sprite {p} not found");
            }
        }
    }

    fn unload(&mut self) {
        if let Some(l) = self.loaded.take() {
            for n in [l.menu, l.atlus_root, l.rd_root] {
                self.ui.destroy(n);
            }
        }
    }

    fn menu_loaded(&self) -> bool {
        self.loaded.is_some()
    }

    fn set_title_shown(&mut self, shown: bool) {
        self.set_shown(self.loaded.and_then(|l| l.title), shown);
    }

    fn set_title_position(&mut self, pos: [f32; 3]) {
        if let Some(n) = self.loaded.and_then(|l| l.title) {
            self.ui.set_local_position(n, pos);
        }
    }

    fn atlus_logo_shown(&self) -> bool {
        self.shown(self.loaded.and_then(|l| l.atlus))
    }

    fn set_atlus_logo_shown(&mut self, shown: bool) {
        self.set_shown(self.loaded.and_then(|l| l.atlus), shown);
    }

    fn set_atlus_logo_color(&mut self, [r, g, b, a]: [f32; 4]) {
        if let Some(n) = self.loaded.and_then(|l| l.atlus) {
            self.ui.set_image_color(n, Color::new(r, g, b, a));
        }
    }

    fn rd_logo_shown(&self) -> bool {
        self.shown(self.loaded.and_then(|l| l.rd))
    }

    fn set_rd_logo_shown(&mut self, shown: bool) {
        self.set_shown(self.loaded.and_then(|l| l.rd), shown);
    }

    fn warning_shown(&self) -> bool {
        self.shown(self.loaded.and_then(|l| l.warning))
    }

    fn set_warning_shown(&mut self, shown: bool) {
        self.set_shown(self.loaded.and_then(|l| l.warning), shown);
    }

    fn set_title_sprite_color(&mut self, index: usize, [r, g, b, a]: [f32; 4]) {
        if let Some(n) = self.loaded.and_then(|l| l.sprites[index]) {
            self.ui.set_image_color(n, Color::new(r, g, b, a));
        }
    }

    fn press_anim_enabled(&self) -> bool {
        self.loaded.and_then(|l| l.press).is_some_and(|n| self.ui.animator_enabled(n))
    }

    fn set_press_anim_enabled(&mut self, enabled: bool) {
        if let Some(n) = self.loaded.and_then(|l| l.press) {
            self.ui.set_animator_enabled(n, enabled);
        }
    }

    fn play_press_anim(&mut self, state: &str, t: f32) {
        if let Some(n) = self.loaded.and_then(|l| l.press) {
            self.ui.animator_play(n, state, t);
        }
    }

    fn menu_shown(&self) -> bool {
        self.shown(self.loaded.and_then(|l| l.menu_group))
    }

    fn set_menu_shown(&mut self, shown: bool) {
        self.set_shown(self.loaded.and_then(|l| l.menu_group), shown);
    }

    fn menu_item_shown(&self, n: i32) -> bool {
        self.shown(self.item(n, |l| &l.items))
    }

    fn set_menu_item_shown(&mut self, n: i32, shown: bool) {
        self.set_shown(self.item(n, |l| &l.items), shown);
    }

    fn set_menu_item_text(&mut self, n: i32, text: &str) {
        if let Some(id) = self.item(n, |l| &l.item_texts) {
            self.ui.set_text(id, text);
        }
    }

    fn menu_item_position(&self, n: i32) -> Option<[f32; 3]> {
        self.item(n, |l| &l.items).map(|id| self.ui.position(id))
    }

    fn set_menu_cursor_position(&mut self, pos: [f32; 3]) {
        if let Some(id) = self.loaded.and_then(|l| l.cursor) {
            self.ui.set_position(id, pos);
        }
    }

    fn set_menu_note_text(&mut self, n: i32, text: &str) -> bool {
        match self.item(n, |l| &l.notes) {
            Some(id) => {
                self.ui.set_text(id, text);
                true
            }
            None => false,
        }
    }

    fn name_window_shown(&self) -> bool {
        self.shown(self.loaded.and_then(|l| l.name_window))
    }

    fn set_name_window_shown(&mut self, shown: bool) {
        self.set_shown(self.loaded.and_then(|l| l.name_window), shown);
    }
}
