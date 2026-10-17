//! Search.

use aizome_kit::prelude::*;

use crate::form;
use crate::pane::{Group, Pane, PaneCx, SettingEntry, Tile, ViewCx};

#[derive(Default)]
pub struct Search;

#[derive(Clone, Debug)]
pub enum Msg {}

impl Pane for Search {
    type Msg = Msg;

    fn id(&self) -> &'static str {
        "search"
    }

    fn title(&self) -> &'static str {
        "Search"
    }

    fn icon(&self) -> Icon {
        Icon::Search
    }

    fn tile(&self) -> Tile {
        Tile::Violet
    }

    fn group(&self) -> Group {
        Group::System
    }

    fn entries(&self) -> Vec<SettingEntry> {
        Vec::new()
    }

    fn update(&mut self, msg: Msg, _cx: &mut PaneCx<Msg>) {
        match msg {}
    }

    fn view(&self, _vcx: &ViewCx) -> Element<Msg> {
        form::page(vec![form::note("This pane is being built.")])
    }
}
