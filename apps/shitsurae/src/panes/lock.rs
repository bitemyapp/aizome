//! Lock Screen.

use aizome_kit::prelude::*;

use crate::form;
use crate::pane::{Group, Pane, PaneCx, SettingEntry, Tile, ViewCx};

#[derive(Default)]
pub struct Lock;

#[derive(Clone, Debug)]
pub enum Msg {}

impl Pane for Lock {
    type Msg = Msg;

    fn id(&self) -> &'static str {
        "lock"
    }

    fn title(&self) -> &'static str {
        "Lock Screen"
    }

    fn icon(&self) -> Icon {
        Icon::Lock
    }

    fn tile(&self) -> Tile {
        Tile::Kon
    }

    fn group(&self) -> Group {
        Group::Security
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
