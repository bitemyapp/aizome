//! Appearance.

use aizome_kit::prelude::*;

use crate::form;
use crate::pane::{Group, Pane, PaneCx, SettingEntry, Tile, ViewCx};

#[derive(Default)]
pub struct Appearance;

#[derive(Clone, Debug)]
pub enum Msg {}

impl Pane for Appearance {
    type Msg = Msg;

    fn id(&self) -> &'static str {
        "appearance"
    }

    fn title(&self) -> &'static str {
        "Appearance"
    }

    fn icon(&self) -> Icon {
        Icon::Palette
    }

    fn tile(&self) -> Tile {
        Tile::Focus
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
