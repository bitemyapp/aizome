//! General.

use aizome_kit::prelude::*;

use crate::form;
use crate::pane::{Group, Pane, PaneCx, SettingEntry, Tile, ViewCx};

#[derive(Default)]
pub struct General;

#[derive(Clone, Debug)]
pub enum Msg {}

impl Pane for General {
    type Msg = Msg;

    fn id(&self) -> &'static str {
        "general"
    }

    fn title(&self) -> &'static str {
        "General"
    }

    fn icon(&self) -> Icon {
        Icon::Settings
    }

    fn tile(&self) -> Tile {
        Tile::Slate
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
