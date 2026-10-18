//! Bluetooth: the switch and the name others see, the devices paired with
//! this computer (connect, disconnect, rename, connect automatically,
//! forget, battery), and devices nearby (pair, which connects). While the
//! pane shows, the computer is discoverable and looks for devices, as
//! macOS's Bluetooth settings do, and answers pairing: a code to compare,
//! one to type in, or one to show, in a sheet.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use aizome_kit::prelude::*;
use aizome_kit::{column, row};
use aizome_sys::bluetooth::{
    Adapter, Bluetooth as Bluez, BluetoothState, Device, DeviceKind, PairingPrompt, PairingRequest, PairingResponse,
    Responder,
};
use aizome_sys::{Error, Task};

use crate::form::{self, Row};
use crate::pane::{Confirm, Group, Pane, PaneCx, SettingEntry, Tile, ViewCx};
use crate::panes::hw::{signal, spinner};
use crate::system::Cell;
use crate::widgets::{PressStyle, press, tile};

/// BlueZ, live or the sample.
pub enum Bt {
    None,
    Live(Bluez),
    Sample(Cell<BluetoothState>),
}

impl Bt {
    pub fn snapshot(&self) -> Arc<BluetoothState> {
        match self {
            Bt::None => Arc::new(BluetoothState::default()),
            Bt::Live(b) => b.snapshot(),
            Bt::Sample(c) => c.get(),
        }
    }

    fn sample_device(&self, path: &str, f: impl FnOnce(&mut Device)) -> Task<()> {
        match self {
            Bt::Sample(c) => {
                let path = path.to_owned();
                c.update(move |s| {
                    if let Some(d) = s.devices.iter_mut().find(|d| d.path == path) {
                        f(d);
                    }
                })
            }
            _ => Task::ready(Err(Error::Unavailable("BlueZ".into()))),
        }
    }

    fn set_powered(&self, adapter: &str, on: bool) -> Task<()> {
        match self {
            Bt::Live(b) => b.set_powered(adapter, on),
            Bt::Sample(c) => c.update(move |s| {
                for a in &mut s.adapters {
                    a.powered = on;
                    a.power_state = if on { "on".into() } else { "off".into() };
                    a.discovering = on;
                }
                if !on {
                    for d in &mut s.devices {
                        d.connected = false;
                    }
                }
            }),
            Bt::None => Task::ready(Err(Error::Unavailable("BlueZ".into()))),
        }
    }

    fn set_name(&self, adapter: &str, name: &str) -> Task<()> {
        match self {
            Bt::Live(b) => b.set_adapter_name(adapter, name),
            Bt::Sample(c) => {
                let name = name.to_owned();
                c.update(move |s| {
                    for a in &mut s.adapters {
                        a.alias = name.clone();
                    }
                })
            }
            Bt::None => Task::ready(Err(Error::Unavailable("BlueZ".into()))),
        }
    }

    fn connect(&self, device: &str) -> Task<()> {
        match self {
            Bt::Live(b) => b.connect(device),
            _ => self.sample_device(device, |d| d.connected = true),
        }
    }

    fn disconnect(&self, device: &str) -> Task<()> {
        match self {
            Bt::Live(b) => b.disconnect(device),
            _ => self.sample_device(device, |d| d.connected = false),
        }
    }

    fn set_trusted(&self, device: &str, on: bool) -> Task<()> {
        match self {
            Bt::Live(b) => b.set_trusted(device, on),
            _ => self.sample_device(device, move |d| d.trusted = on),
        }
    }

    fn rename(&self, device: &str, name: &str) -> Task<()> {
        match self {
            Bt::Live(b) => b.rename_device(device, name),
            _ => {
                let name = name.to_owned();
                self.sample_device(device, move |d| d.name = name)
            }
        }
    }

    fn remove(&self, device: &str) -> Task<()> {
        match self {
            Bt::Live(b) => b.remove(device),
            Bt::Sample(c) => {
                let device = device.to_owned();
                c.update(move |s| s.devices.retain(|d| d.path != device))
            }
            Bt::None => Task::ready(Err(Error::Unavailable("BlueZ".into()))),
        }
    }
}

/// The glyph for a kind of device.
pub fn glyph(kind: DeviceKind) -> Icon {
    match kind {
        DeviceKind::Headset => Icon::Headset,
        DeviceKind::Headphones => Icon::Headphones,
        DeviceKind::Speaker => Icon::Speaker,
        DeviceKind::Microphone => Icon::Mic,
        DeviceKind::Keyboard => Icon::Keyboard,
        DeviceKind::Mouse => Icon::Mouse,
        DeviceKind::Gamepad => Icon::Gamepad2,
        DeviceKind::Tablet => Icon::Tablet,
        DeviceKind::Phone => Icon::Smartphone,
        DeviceKind::Computer => Icon::Laptop,
        DeviceKind::Watch => Icon::Watch,
        DeviceKind::Printer => Icon::Printer,
        DeviceKind::Camera => Icon::Camera,
        DeviceKind::Tv => Icon::Tv,
        DeviceKind::Network => Icon::Router,
        DeviceKind::Other => Icon::Bluetooth,
    }
}

/// "Headphones", "Mouse".
pub fn kind_words(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Headset => "Headset",
        DeviceKind::Headphones => "Headphones",
        DeviceKind::Speaker => "Speaker",
        DeviceKind::Microphone => "Microphone",
        DeviceKind::Keyboard => "Keyboard",
        DeviceKind::Mouse => "Mouse",
        DeviceKind::Gamepad => "Game controller",
        DeviceKind::Tablet => "Tablet",
        DeviceKind::Phone => "Phone",
        DeviceKind::Computer => "Computer",
        DeviceKind::Watch => "Watch",
        DeviceKind::Printer => "Printer",
        DeviceKind::Camera => "Camera",
        DeviceKind::Tv => "Display",
        DeviceKind::Network => "Network access point",
        DeviceKind::Other => "Device",
    }
}

/// Signal strength from RSSI, in four steps.
pub fn rssi_bars(rssi: i16) -> u8 {
    match rssi {
        -60.. => 4,
        -70..=-61 => 3,
        -80..=-71 => 2,
        _ => 1,
    }
}

fn battery_icon(level: u8) -> Icon {
    match level {
        70.. => Icon::BatteryFull,
        35..=69 => Icon::BatteryMedium,
        _ => Icon::BatteryLow,
    }
}

/// What BlueZ asks, as the pane keeps it (the answer goes back through the
/// responder kept beside it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    pub id: u64,
    pub device: String,
    pub name: String,
    pub request: PairingRequest,
    /// What's typed for a PIN or passkey.
    pub typed: String,
}

#[derive(Clone, Debug)]
pub enum Msg {
    Power(bool),
    Powered(bool, Result<(), Error>),
    NameText(String),
    Rename,
    Connect(String),
    Disconnect(String),
    Pair(String),
    Expand(Option<String>),
    Trusted(String, bool),
    DeviceName(String),
    RenameDevice(String),
    Forget(String),
    ForgetNow(String),
    /// BlueZ asks something while pairing.
    Prompted(Prompt),
    PromptTyped(String),
    Answer(bool),
    Done(String, String, Result<(), Error>),
    /// The pairing agent was registered, or not.
    Agent(Result<(), Error>),
}

pub struct Bluetooth {
    bt: Bt,
    /// The adapter's name, as being typed.
    name: Option<String>,
    /// The device whose options are open.
    expanded: Option<String>,
    device_name: String,
    /// Devices being paired, connected or disconnected.
    busy: HashSet<String>,
    prompt: Option<Prompt>,
    responders: Arc<Mutex<HashMap<u64, Responder>>>,
    next_prompt: u64,
    /// The pane made the adapter discoverable (and undoes it when hidden).
    made_discoverable: bool,
    agent: bool,
    /// The pane shows, and has started looking for devices.
    showing: bool,
    looking: bool,
}

impl Default for Bluetooth {
    fn default() -> Bluetooth {
        Bluetooth {
            bt: Bt::None,
            name: None,
            expanded: None,
            device_name: String::new(),
            busy: HashSet::new(),
            prompt: None,
            responders: Arc::new(Mutex::new(HashMap::new())),
            next_prompt: 1,
            made_discoverable: false,
            agent: false,
            showing: false,
            looking: false,
        }
    }
}

impl Bluetooth {
    fn adapter(&self) -> Option<Adapter> {
        self.bt.snapshot().default_adapter().cloned()
    }

    fn act(&mut self, cx: &mut PaneCx<Msg>, what: &str, device: &str, task: Task<()>) {
        let (what, device) = (what.to_owned(), device.to_owned());
        cx.wait(task, move |r| Msg::Done(what, device, r));
    }

    /// Discoverable and looking for devices while the pane shows.
    fn look(&mut self, cx: &mut PaneCx<Msg>, just_on: bool) {
        let Bt::Live(b) = &self.bt else { return };
        let Some(a) = b.snapshot().default_adapter().cloned() else { return };
        // Just turned on: the snapshot may not say so yet.
        if !a.powered && !just_on {
            return;
        }
        if !a.discoverable {
            self.made_discoverable = true;
            let t = b.set_discoverable(&a.path, true);
            cx.wait(t, |_| Msg::Done(String::new(), String::new(), Ok(())));
        }
        let t = b.start_discovery(&a.path);
        cx.wait(t, |_| Msg::Done(String::new(), String::new(), Ok(())));
        self.looking = true;
    }

    fn answer(&mut self, accept: bool, cx: &mut PaneCx<Msg>) {
        let Some(p) = self.prompt.take() else { return };
        cx.close_sheet();
        let responder = self.responders.lock().unwrap_or_else(|e| e.into_inner()).remove(&p.id);
        let response = if !accept {
            PairingResponse::Reject
        } else {
            match &p.request {
                PairingRequest::PinCode => PairingResponse::PinCode(p.typed.trim().to_owned()),
                PairingRequest::Passkey => PairingResponse::Passkey(p.typed.trim().parse().unwrap_or(0)),
                _ => PairingResponse::Accept,
            }
        };
        match responder {
            Some(r) => r.respond(response),
            None if matches!(self.bt, Bt::Sample(_)) => {
                // The sample pairs when the code is accepted.
                if accept {
                    let t = self.bt.sample_device(&p.device, |d| {
                        d.paired = true;
                        d.trusted = true;
                        d.connected = true;
                    });
                    self.act(cx, "Couldn't pair", &p.device, t);
                } else {
                    self.busy.remove(&p.device);
                }
            }
            None => {}
        }
        if !accept {
            self.busy.remove(&p.device);
        }
    }
}

impl Pane for Bluetooth {
    type Msg = Msg;

    fn id(&self) -> &'static str {
        "bluetooth"
    }

    fn title(&self) -> &'static str {
        "Bluetooth"
    }

    fn icon(&self) -> Icon {
        Icon::Bluetooth
    }

    fn tile(&self) -> Tile {
        Tile::Ai
    }

    fn group(&self) -> Group {
        Group::Connections
    }

    fn keywords(&self) -> &'static [&'static str] {
        &["pair", "headphones", "mouse", "keyboard", "wireless", "devices", "bluez"]
    }

    fn entries(&self) -> Vec<SettingEntry> {
        vec![
            SettingEntry::new("bluetooth", "Bluetooth").keywords(&["turn off", "radio"]),
            SettingEntry::new("name", "Name").keywords(&["discoverable", "rename", "visible as"]),
            SettingEntry::new("devices", "My Devices").keywords(&[
                "paired",
                "connect",
                "disconnect",
                "forget",
                "battery",
            ]),
            SettingEntry::new("nearby", "Nearby Devices").keywords(&["pair", "add", "discover", "scan"]),
        ]
    }

    fn shown(&mut self, cx: &mut PaneCx<Msg>) {
        self.showing = true;
        self.looking = false;
        if matches!(self.bt, Bt::None) {
            let wake = cx.wake();
            self.bt = if cx.sample() {
                Bt::Sample(Cell::new(
                    crate::panes::hw::live_snapshot(
                        || Bluez::new(aizome_sys::Wake::none()),
                        |b| b.available(),
                        |b| b.snapshot(),
                    )
                    .unwrap_or_else(sample),
                    wake,
                ))
            } else {
                Bt::Live(Bluez::new(wake))
            };
        }
        if let Bt::Live(b) = &self.bt
            && !self.agent
        {
            let tx = cx.stream();
            let responders = self.responders.clone();
            let counter = Arc::new(std::sync::atomic::AtomicU64::new(1_000));
            // On the agent's own thread: BlueZ can take seconds to answer,
            // and the window must keep moving and drawing meanwhile.
            let registered = b.register_agent(move |p: PairingPrompt| {
                let id = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                responders.lock().unwrap_or_else(|e| e.into_inner()).insert(id, p.responder);
                tx.send(Msg::Prompted(Prompt {
                    id,
                    device: p.device,
                    name: p.name,
                    request: p.request,
                    typed: String::new(),
                }));
            });
            self.agent = true;
            cx.wait(registered, Msg::Agent);
        }
        self.look(cx, false);
    }

    fn system_changed(&mut self, cx: &mut PaneCx<Msg>) {
        // BlueZ's adapter may only be known a moment after the pane shows.
        if self.showing && !self.looking {
            self.look(cx, false);
        }
    }

    fn hidden(&mut self, cx: &mut PaneCx<Msg>) {
        self.showing = false;
        self.looking = false;
        if let Bt::Live(b) = &self.bt {
            if let Some(a) = b.snapshot().default_adapter().cloned() {
                let t = b.stop_discovery(&a.path);
                cx.wait(t, |_| Msg::Done(String::new(), String::new(), Ok(())));
                if self.made_discoverable {
                    let t = b.set_discoverable(&a.path, false);
                    cx.wait(t, |_| Msg::Done(String::new(), String::new(), Ok(())));
                }
            }
            self.made_discoverable = false;
            // Done after the registration, on the agent's thread.
            let _ = b.unregister_agent();
            self.agent = false;
        }
    }

    fn update(&mut self, msg: Msg, cx: &mut PaneCx<Msg>) {
        match msg {
            Msg::Power(on) => {
                let Some(a) = self.adapter() else { return };
                let t = self.bt.set_powered(&a.path, on);
                cx.wait(t, move |r| Msg::Powered(on, r));
            }
            Msg::Powered(on, result) => match result {
                // On now: look for devices, and be found.
                Ok(()) if on => self.look(cx, true),
                Ok(()) => {}
                Err(e) => {
                    cx.failed(if on { "Couldn't turn Bluetooth on" } else { "Couldn't turn Bluetooth off" }, reason(&e))
                }
            },
            Msg::NameText(t) => self.name = Some(t),
            Msg::Rename => {
                let (Some(a), Some(name)) = (self.adapter(), self.name.take()) else { return };
                let name = name.trim().to_owned();
                if name.is_empty() || name == a.alias {
                    return;
                }
                let t = self.bt.set_name(&a.path, &name);
                self.act(cx, "Couldn't rename this computer", "", t);
            }
            Msg::Connect(d) => {
                self.busy.insert(d.clone());
                let t = self.bt.connect(&d);
                self.act(cx, "Couldn't connect", &d, t);
            }
            Msg::Disconnect(d) => {
                self.busy.insert(d.clone());
                let t = self.bt.disconnect(&d);
                self.act(cx, "Couldn't disconnect", &d, t);
            }
            Msg::Pair(d) => {
                self.busy.insert(d.clone());
                match &self.bt {
                    Bt::Live(b) => {
                        let t = b.pair(&d);
                        self.act(cx, "Couldn't pair", &d, t);
                    }
                    Bt::Sample(c) => {
                        // The sample asks to compare a code, as most devices do.
                        let name = c.get().device(&d).map(|x| x.name.clone()).unwrap_or_default();
                        let id = self.next_prompt;
                        self.next_prompt += 1;
                        cx.send(Msg::Prompted(Prompt {
                            id,
                            device: d,
                            name,
                            request: PairingRequest::Confirm(482_913),
                            typed: String::new(),
                        }));
                    }
                    Bt::None => {}
                }
            }
            Msg::Expand(d) => {
                self.device_name =
                    d.as_deref().and_then(|p| self.bt.snapshot().device(p).map(|x| x.name.clone())).unwrap_or_default();
                self.expanded = if self.expanded == d { None } else { d };
            }
            Msg::Trusted(d, on) => {
                let t = self.bt.set_trusted(&d, on);
                self.act(cx, "Couldn't change it", "", t);
            }
            Msg::DeviceName(n) => self.device_name = n,
            Msg::RenameDevice(d) => {
                let name = self.device_name.trim().to_owned();
                if !name.is_empty() {
                    let t = self.bt.rename(&d, &name);
                    self.act(cx, "Couldn't rename it", "", t);
                }
            }
            Msg::Forget(d) => {
                let name = self.bt.snapshot().device(&d).map(|x| x.name.clone()).unwrap_or_default();
                cx.confirm(
                    Confirm::new(
                        format!("Forget “{name}”?"),
                        "It's unpaired from this computer. To use it again, pair it again.",
                        "Forget Device",
                        Msg::ForgetNow(d),
                    )
                    .destructive(),
                );
            }
            Msg::ForgetNow(d) => {
                self.expanded = None;
                let t = self.bt.remove(&d);
                self.act(cx, "Couldn't forget it", "", t);
            }
            Msg::Prompted(p) => {
                if p.request == PairingRequest::Cancelled {
                    if self.prompt.as_ref().is_some_and(|q| q.device == p.device) {
                        self.prompt = None;
                        cx.close_sheet();
                    }
                    self.busy.remove(&p.device);
                    return;
                }
                self.busy.insert(p.device.clone());
                let typing = matches!(p.request, PairingRequest::PinCode | PairingRequest::Passkey);
                // A display-only update (keys typed so far) keeps the sheet.
                self.prompt = Some(p);
                cx.open_sheet();
                if typing {
                    cx.focus("bluetooth-code");
                }
            }
            Msg::PromptTyped(t) => {
                if let Some(p) = &mut self.prompt {
                    p.typed = t;
                }
            }
            Msg::Answer(accept) => self.answer(accept, cx),
            Msg::Agent(result) => {
                if let Err(e) = result {
                    log::info!("no Bluetooth pairing agent: {e}");
                    self.agent = false;
                }
            }
            Msg::Done(what, device, result) => {
                if !device.is_empty() {
                    self.busy.remove(&device);
                }
                if let Err(e) = result
                    && !what.is_empty()
                {
                    let name = self.bt.snapshot().device(&device).map(|d| d.name.clone());
                    let what = match name {
                        Some(n) => format!("{what} {n}"),
                        None => what,
                    };
                    cx.failed(&what, reason(&e));
                }
            }
        }
    }

    fn view(&self, vcx: &ViewCx) -> Element<Msg> {
        let state = self.bt.snapshot();
        if matches!(self.bt, Bt::Live(_)) && !state.available {
            return form::page(vec![
                form::heading("Bluetooth isn't available", "BlueZ, the Bluetooth service, isn't running."),
                form::note("On NixOS it's turned on with hardware.bluetooth.enable = true."),
            ]);
        }
        let Some(adapter) = state.default_adapter().cloned() else {
            return form::page(vec![form::heading(
                "No Bluetooth",
                "This computer has no Bluetooth adapter, or it's switched off in the firmware.",
            )]);
        };
        let on = adapter.powered;
        let mut sections: Vec<Element<Msg>> = Vec::new();
        let detail = if adapter.blocked() {
            "Off: airplane mode or a radio switch keeps it off".to_owned()
        } else if !on {
            "Off".to_owned()
        } else if adapter.discoverable {
            format!("Discoverable as “{}” while this pane is open", adapter.alias)
        } else {
            format!("On, as “{}”", adapter.alias)
        };
        let name = self.name.clone().unwrap_or_else(|| adapter.alias.clone());
        sections.push(vcx.section(
            "",
            vec![
                    form::row("Bluetooth", toggle(on).on_toggle(Msg::Power))
                        .leading(tile(Icon::Bluetooth, Tile::Ai, 24.0))
                        .detail(detail)
                        .entry("bluetooth"),
                    form::row(
                        "Name",
                        text_field(&name)
                            .on_change(Msg::NameText)
                            .on_submit(Msg::Rename)
                            .on_blur(Msg::Rename)
                            .width(200.0)
                            .id("bluetooth-name"),
                    )
                    .help("What other devices see when they look for this computer")
                    .entry("name"),
                ],
        ));
        if !on {
            return form::page(sections);
        }
        let paired: Vec<&Device> = state.paired().collect();
        if paired.is_empty() {
            sections.push(vcx.section(
                "My Devices",
                vec![form::row("No devices yet", space(0.0)).help("Devices you pair show here.").entry("devices")],
            ));
        } else {
            let mut rows: Vec<Row<Msg>> = Vec::new();
            for (i, d) in paired.iter().enumerate() {
                let mut r = device_row(self, d);
                if i == 0 {
                    r = r.entry("devices");
                }
                rows.push(r);
                if self.expanded.as_deref() == Some(d.path.as_str()) {
                    rows.extend(device_options(self, d));
                }
            }
            sections.push(vcx.section("My Devices", rows));
        }
        let nearby: Vec<&Device> = state.nearby().collect();
        let mut rows: Vec<Row<Msg>> = nearby.iter().map(|d| nearby_row(self, d)).collect();
        if rows.is_empty() {
            rows.push(form::whole(
                container(
                    row![
                        spinner(14.0),
                        text("Looking for devices… Make sure the device is in pairing mode.").role(Role::Text3).wrap()
                    ]
                    .spacing(10.0),
                )
                .padding([10.0, 12.0])
                .width(Fill),
            ));
        }
        if !rows.is_empty() {
            let first = rows.remove(0);
            rows.insert(0, first.entry("nearby"));
        }
        let tail: Element<Msg> = if adapter.discovering { spinner(14.0) } else { space(0.0) };
        sections.push(
            column![
                row![form::caption("Nearby Devices"), spacer(), tail].align(Align::Center).width(Fill).height(16.0),
                vcx.section("", rows)
            ]
            .spacing(6.0)
            .width(Fill)
            .into(),
        );
        form::page(sections)
    }

    fn sheet(&self, _vcx: &ViewCx) -> Option<Element<Msg>> {
        self.prompt.as_ref().map(prompt_sheet)
    }

    fn back(&mut self, cx: &mut PaneCx<Msg>) -> bool {
        if self.prompt.is_some() {
            self.answer(false, cx);
            return true;
        }
        if self.expanded.take().is_some() {
            return true;
        }
        false
    }
}

fn reason(e: &Error) -> String {
    match e {
        Error::Rejected(_) => "the device refused, or the codes didn't match".into(),
        Error::Timeout => "the device didn't answer; is it on and nearby?".into(),
        Error::Unavailable(_) => "Bluetooth isn't running".into(),
        Error::DBus { name, message }
            if name.ends_with("AuthenticationFailed") || name.ends_with("AuthenticationRejected") =>
        {
            if message.is_empty() { "pairing was refused".into() } else { message.clone() }
        }
        Error::DBus { name, .. } if name.ends_with("AuthenticationCanceled") => "pairing was cancelled".into(),
        Error::DBus { name, .. } if name.ends_with("ConnectionAttemptFailed") || name.ends_with("Failed") => {
            "the device didn't accept the connection".into()
        }
        other => other.to_string(),
    }
}

fn status(d: &Device) -> String {
    let mut s = if d.connected { "Connected".to_owned() } else { "Not connected".to_owned() };
    if let Some(b) = d.battery {
        s.push_str(&format!(" · {b}%"));
    }
    s
}

fn device_row(p: &Bluetooth, d: &Device) -> Row<Msg> {
    let busy = p.busy.contains(&d.path);
    let mut line = row![
        tile(glyph(d.kind), if d.connected { Tile::Ai } else { Tile::Slate }, 28.0),
        column![
            text(d.name.clone()).weight(if d.connected { 600.0 } else { 400.0 }),
            text(status(d)).size(11.0).role(Role::Text3)
        ]
        .spacing(2.0)
        .width(Fill),
    ]
    .spacing(10.0)
    .align(Align::Center)
    .width(Fill);
    if let Some(b) = d.battery {
        line = line.push(
            icon(battery_icon(b))
                .size(16.0)
                .role(if b < 20 { Role::Danger } else { Role::Text3 })
                .tooltip(format!("Battery {b}%")),
        );
    }
    if busy {
        line = line.push(spinner(14.0));
    } else if d.connected {
        line = line.push(button("Disconnect").compact().on_press(Msg::Disconnect(d.path.clone())));
    } else {
        line = line.push(button("Connect").compact().on_press(Msg::Connect(d.path.clone())));
    }
    let open = p.expanded.as_deref() == Some(d.path.as_str());
    line = line.push(
        icon_button(if open { Icon::ChevronUp } else { Icon::Info })
            .plain()
            .compact()
            .on_press(Msg::Expand(Some(d.path.clone())))
            .tooltip("Options")
            .label(format!("Options for {}", d.name)),
    );
    form::whole(press(line).style(PressStyle::Plain).padding([8.0, 12.0]).named(d.name.clone()).width(Fill))
}

fn device_options(p: &Bluetooth, d: &Device) -> Vec<Row<Msg>> {
    let path = d.path.clone();
    let path2 = d.path.clone();
    vec![
        form::value_mono("Address", d.address.clone()).detail(kind_words(d.kind)),
        form::row(
            "Name",
            text_field(&p.device_name)
                .on_change(Msg::DeviceName)
                .on_submit(Msg::RenameDevice(path.clone()))
                .width(200.0),
        ),
        form::row("Connect automatically", toggle(d.trusted).on_toggle(move |on| Msg::Trusted(path2.clone(), on)))
            .help("It connects by itself when it's on and nearby"),
        form::row(
            "Unpair it from this computer",
            button("Forget This Device…").destructive().on_press(Msg::Forget(path)),
        ),
    ]
}

fn nearby_row(p: &Bluetooth, d: &Device) -> Row<Msg> {
    let busy = p.busy.contains(&d.path);
    let mut line = row![
        tile(glyph(d.kind), Tile::Slate, 28.0),
        column![text(d.name.clone()), text(kind_words(d.kind)).size(11.0).role(Role::Text3)].spacing(2.0).width(Fill),
    ]
    .spacing(10.0)
    .align(Align::Center)
    .width(Fill);
    if let Some(r) = d.rssi {
        line = line.push(signal(rssi_bars(r), 16.0, false).tooltip(format!("{r} dBm")));
    }
    line = line.push(if busy {
        spinner(14.0)
    } else {
        button("Connect").compact().on_press(Msg::Pair(d.path.clone())).into()
    });
    form::whole(press(line).style(PressStyle::Plain).padding([8.0, 12.0]).named(d.name.clone()).width(Fill))
}

fn code(s: String) -> Element<Msg> {
    container(text(s).mono().size(28.0).weight(600.0).letter_spacing(2.0).tabular())
        .padding([8.0, 16.0])
        .fill(Tone::Sunk)
        .border(Edge::Line)
        .center()
        .width(Fill)
        .into()
}

fn six(n: u32) -> String {
    let s = format!("{n:06}");
    format!("{} {}", &s[..3], &s[3..])
}

fn prompt_sheet(p: &Prompt) -> Element<Msg> {
    let name = &p.name;
    let (title, body, middle, ok, cancel): (String, String, Option<Element<Msg>>, Option<&str>, &str) = match &p.request
    {
        PairingRequest::Confirm(n) => (
            format!("Pair with “{name}”?"),
            format!("Check that “{name}” shows this code, then pair."),
            Some(code(six(*n))),
            Some("Pair"),
            "Cancel",
        ),
        PairingRequest::Passkey => (
            format!("Pair with “{name}”"),
            format!("Type the 6-digit code “{name}” shows."),
            Some(
                text_field(&p.typed)
                    .mono()
                    .placeholder("000000")
                    .on_change(Msg::PromptTyped)
                    .on_submit(Msg::Answer(true))
                    .id("bluetooth-code")
                    .width(Fill)
                    .into(),
            ),
            Some("Pair"),
            "Cancel",
        ),
        PairingRequest::PinCode => (
            format!("Pair with “{name}”"),
            format!("Type the PIN for “{name}” (often 0000 or 1234 for older devices)."),
            Some(
                text_field(&p.typed)
                    .mono()
                    .placeholder("PIN")
                    .on_change(Msg::PromptTyped)
                    .on_submit(Msg::Answer(true))
                    .id("bluetooth-code")
                    .width(Fill)
                    .into(),
            ),
            Some("Pair"),
            "Cancel",
        ),
        PairingRequest::DisplayPasskey { passkey, entered } => (
            format!("Pair with “{name}”"),
            format!(
                "Type this code on “{name}”, then press Return on it.{}",
                if *entered > 0 { format!(" {entered} typed so far.") } else { String::new() }
            ),
            Some(code(six(*passkey))),
            None,
            "Cancel",
        ),
        PairingRequest::DisplayPinCode(pin) => (
            format!("Pair with “{name}”"),
            format!("Type this PIN on “{name}”, then press Return on it."),
            Some(code(pin.clone())),
            None,
            "Cancel",
        ),
        PairingRequest::Authorize => (
            format!("Allow “{name}” to pair?"),
            "It asked to pair with this computer.".into(),
            None,
            Some("Allow"),
            "Don't Allow",
        ),
        PairingRequest::AuthorizeService(uuid) => (
            format!("Allow “{name}” to connect?"),
            format!("It asks to use a service of this computer ({uuid})."),
            None,
            Some("Allow"),
            "Don't Allow",
        ),
        PairingRequest::Cancelled => (String::new(), String::new(), None, None, "Close"),
    };
    let typed_ok = match &p.request {
        PairingRequest::Passkey => p.typed.trim().len() == 6 && p.typed.trim().chars().all(|c| c.is_ascii_digit()),
        PairingRequest::PinCode => !p.typed.trim().is_empty() && p.typed.trim().len() <= 16,
        _ => true,
    };
    let mut col = column![
        row![icon(Icon::Bluetooth).size(24.0).role(Role::Accent), text(title).size(15.0).weight(650.0)].spacing(10.0),
        text(body).role(Role::Text2).wrap(),
    ]
    .spacing(12.0)
    .width(Fill);
    if let Some(m) = middle {
        col = col.push(m);
    }
    let mut buttons = row![spacer(), button(cancel).cancel().on_press(Msg::Answer(false))].spacing(8.0).width(Fill);
    if let Some(ok) = ok {
        buttons = buttons.push(button(ok).primary().default().on_press(Msg::Answer(true)).enabled(typed_ok));
    }
    col = col.push(buttons);
    container(col).padding([20.0, 20.0, 16.0, 20.0]).width(400.0).into()
}

/// The sample: this computer's adapter, two devices connected, two paired
/// and asleep, and three nearby.
pub fn sample() -> BluetoothState {
    let adapter = Adapter {
        path: "/org/bluez/hci0".into(),
        address: "A8:7E:EA:4C:19:D1".into(),
        name: "kohaku".into(),
        alias: "kohaku".into(),
        powered: true,
        power_state: "on".into(),
        discoverable: true,
        discoverable_timeout: 0,
        pairable: true,
        discovering: true,
    };
    let dev =
        |n: u8, name: &str, kind: DeviceKind, paired: bool, connected: bool, battery: Option<u8>, rssi: Option<i16>| {
            Device {
                path: format!("/org/bluez/hci0/dev_{n:02X}"),
                adapter: adapter.path.clone(),
                address: format!("F4:4E:FD:2A:{n:02X}:{:02X}", n.wrapping_mul(37)),
                name: name.into(),
                has_name: true,
                icon: String::new(),
                kind,
                paired,
                bonded: paired,
                trusted: paired,
                blocked: false,
                connected,
                battery,
                rssi,
                services_resolved: connected,
                legacy_pairing: false,
                uuids: Vec::new(),
            }
        };
    BluetoothState {
        available: true,
        adapters: vec![adapter.clone()],
        devices: vec![
            dev(1, "WH-1000XM5", DeviceKind::Headphones, true, true, Some(80), Some(-48)),
            dev(2, "MX Master 3S", DeviceKind::Mouse, true, true, Some(14), Some(-52)),
            dev(3, "Keychron K3", DeviceKind::Keyboard, true, false, None, None),
            dev(4, "Aiko's Phone", DeviceKind::Phone, true, false, None, None),
            dev(5, "JBL Flip 6", DeviceKind::Speaker, false, false, None, Some(-58)),
            dev(6, "Pixel Watch 2", DeviceKind::Watch, false, false, None, Some(-74)),
            dev(7, "Xbox Wireless Controller", DeviceKind::Gamepad, false, false, None, Some(-83)),
        ],
    }
}
