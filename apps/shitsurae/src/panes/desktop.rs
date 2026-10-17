//! Desktop & Deskbar.

use aizome_kit::prelude::*;

use crate::form;
use crate::pane::{Group, Pane, PaneCx, SettingEntry, Tile, ViewCx};

#[derive(Default)]
pub struct Desktop;

#[derive(Clone, Debug)]
pub enum Msg {}

impl Pane for Desktop {
    type Msg = Msg;

    fn id(&self) -> &'static str {
        "desktop"
    }

    fn title(&self) -> &'static str {
        "Desktop & Deskbar"
    }

    fn icon(&self) -> Icon {
        Icon::LayoutDashboard
    }

    fn tile(&self) -> Tile {
        Tile::Ai
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
