//! Dragging inside a window: where things can be dropped.
//!
//! Sidestep has no drag source of its own yet, so a drag that starts in
//! Hako is a gesture: the view that took the press keeps the pointer and
//! asks the window's [`Spots`] what's under it as it moves. Every place
//! that takes a drop (each view, each sidebar row, each crumb of the
//! path bar) says where it is, in window points, whenever it's drawn.
//! Drags from other programs come through Sidestep's drag and drop, as
//! `file://` URIs, and are dropped through the same places.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use aizome_kit::{Point, Rect};

/// Where a drop goes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Dest {
    /// Into a folder.
    Folder(PathBuf),
    /// To the Trash.
    Trash,
}

/// What's under the pointer during a drag: where a drop would go, and
/// what shows it (a spot, and a row of it).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Target {
    pub dest: Dest,
    pub spot: u64,
    pub row: Option<usize>,
}

/// Finds the target at a window point, within a spot.
pub type Resolve = Rc<dyn Fn(Point) -> Option<Target>>;

/// A place that takes drops.
#[derive(Clone)]
pub struct Spot {
    pub rect: Rect,
    /// Spots of the sidebar and the path bar come before the views', which
    /// can be scrolled under them.
    pub priority: u8,
    pub resolve: Resolve,
}

/// Spot IDs: views are their column's number; the rest are offset.
pub const SIDEBAR: u64 = 1 << 20;
pub const PATHBAR: u64 = 2 << 20;

/// A window's spots, by ID.
#[derive(Clone, Default)]
pub struct Spots(Rc<RefCell<BTreeMap<u64, Spot>>>);

impl Spots {
    pub fn set(&self, id: u64, spot: Spot) {
        self.0.borrow_mut().insert(id, spot);
    }

    /// Forget spots no longer shown.
    pub fn retain(&self, keep: impl Fn(u64) -> bool) {
        self.0.borrow_mut().retain(|id, _| keep(*id));
    }

    /// The target at `p` (window points).
    pub fn at(&self, p: Point) -> Option<Target> {
        let spots = self.0.borrow();
        let mut found: Vec<&Spot> = spots.values().filter(|s| s.rect.contains(p)).collect();
        found.sort_by_key(|s| std::cmp::Reverse(s.priority));
        found.into_iter().find_map(|s| (s.resolve)(p))
    }

    pub fn len(&self) -> usize {
        self.0.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A drag's course, as a view tells it.
#[derive(Clone, Debug, PartialEq)]
pub enum DragEvent {
    /// It started, from the selection of the view.
    Start,
    /// What's under it now.
    Over(Option<Target>),
    /// Let go: onto a target (if any), copying (⌥ or ⌃ held) or not.
    Drop { target: Option<Target>, copy: bool },
    /// Escape, or the pointer lost.
    Cancel,
}

/// Points the pointer must move with the button down before it drags.
pub const SLOP: f64 = 5.0;
