//! Notifications.

use aizome_kit::prelude::*;

use crate::form;
use crate::pane::{Group, Pane, PaneCx, SettingEntry, Tile, ViewCx};

#[derive(Default)]
pub struct Notifications;

#[derive(Clone, Debug)]
pub enum Msg {}

impl Pane for Notifications {
    type Msg = Msg;

    fn id(&self) -> &'static str {
        "notifications"
    }

    fn title(&self) -> &'static str {
        "Notifications"
    }

    fn icon(&self) -> Icon {
        Icon::Bell
    }

    fn tile(&self) -> Tile {
        Tile::Red
    }

    fn group(&self) -> Group {
        Group::Alerts
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
