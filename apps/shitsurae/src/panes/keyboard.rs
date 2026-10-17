//! Keyboard.

use aizome_kit::prelude::*;

use crate::form;
use crate::pane::{Group, Pane, PaneCx, SettingEntry, Tile, ViewCx};

#[derive(Default)]
pub struct Keyboard;

#[derive(Clone, Debug)]
pub enum Msg {}

impl Pane for Keyboard {
    type Msg = Msg;

    fn id(&self) -> &'static str {
        "keyboard"
    }

    fn title(&self) -> &'static str {
        "Keyboard"
    }

    fn icon(&self) -> Icon {
        Icon::Keyboard
    }

    fn tile(&self) -> Tile {
        Tile::Slate
    }

    fn group(&self) -> Group {
        Group::Devices
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
