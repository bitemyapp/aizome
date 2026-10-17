//! Menu Bar.

use aizome_kit::prelude::*;

use crate::form;
use crate::pane::{Group, Pane, PaneCx, SettingEntry, Tile, ViewCx};

#[derive(Default)]
pub struct MenuBar;

#[derive(Clone, Debug)]
pub enum Msg {}

impl Pane for MenuBar {
    type Msg = Msg;

    fn id(&self) -> &'static str {
        "menubar"
    }

    fn title(&self) -> &'static str {
        "Menu Bar"
    }

    fn icon(&self) -> Icon {
        Icon::Menu
    }

    fn tile(&self) -> Tile {
        Tile::Kon
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
