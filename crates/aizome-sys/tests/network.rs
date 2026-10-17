//! Network against a fake NetworkManager on a private bus.

mod common;

use aizome_sys::network::{Network, RadioPaths, Security};
use aizome_sys::{Error, Wake};
use common::{eventually, path};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zbus::fdo;
use zbus::object_server::{ObjectServer, SignalEmitter};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

type Settings = HashMap<String, HashMap<String, OwnedValue>>;

const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const DEVICE: &str = "/org/freedesktop/NetworkManager/Devices/1";

fn clone_settings(settings: &Settings) -> Settings {
    settings
        .iter()
        .map(|(group, props)| {
            (group.clone(), props.iter().map(|(k, v)| (k.clone(), v.try_clone().unwrap())).collect())
        })
        .collect()
}

fn setting_str(settings: &Settings, group: &str, key: &str) -> Option<String> {
    let value = settings.get(group)?.get(key)?;
    String::try_from(value.try_clone().ok()?).ok()
}

/// What the fake was asked to do.
#[derive(Default)]
struct Log {
    added: Vec<Settings>,
    deleted: Vec<String>,
    updated: Vec<(Settings, u32)>,
    activated: Vec<String>,
    scans: usize,
}

struct Manager {
    log: Arc<Mutex<Log>>,
    wireless_enabled: bool,
    active: Vec<OwnedObjectPath>,
    next: u32,
}

/// Whether the fake accepts a connection: open, or the password "right".
fn accepted(settings: &Settings) -> bool {
    match setting_str(settings, "802-11-wireless-security", "key-mgmt") {
        None => true,
        Some(_) => setting_str(settings, "802-11-wireless-security", "psk").as_deref() == Some("right"),
    }
}

impl Manager {
    /// Activate: a new active connection that is up at once when the
    /// password is right, or gone (with the device saying "no secrets")
    /// when not.
    async fn activate(
        &mut self,
        server: &ObjectServer,
        emitter: &SignalEmitter<'_>,
        connection: &str,
        settings: &Settings,
    ) -> fdo::Result<OwnedObjectPath> {
        self.next += 1;
        let active = path(&format!("{NM_PATH}/ActiveConnection/{}", self.next));
        self.log.lock().unwrap().activated.push(connection.to_owned());
        if accepted(settings) {
            server
                .at(
                    active.as_str(),
                    Active {
                        connection: path(connection),
                        uuid: setting_str(settings, "connection", "uuid").unwrap_or_default(),
                        id: setting_str(settings, "connection", "id").unwrap_or_default(),
                        state: 2,
                    },
                )
                .await?;
            self.active.push(active.clone());
            self.active_connections_changed(emitter).await?;
        } else {
            let device = server.interface::<_, Device>(DEVICE).await?;
            device.get_mut().await.reason = 7;
        }
        Ok(active)
    }
}

#[zbus::interface(name = "org.freedesktop.NetworkManager")]
impl Manager {
    #[zbus(property)]
    fn version(&self) -> String {
        "1.58.0".into()
    }
    #[zbus(property)]
    fn state(&self) -> u32 {
        70
    }
    #[zbus(property)]
    fn connectivity(&self) -> u32 {
        4
    }
    #[zbus(property)]
    fn networking_enabled(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn wireless_enabled(&self) -> bool {
        self.wireless_enabled
    }
    #[zbus(property)]
    fn set_wireless_enabled(&mut self, on: bool) {
        self.wireless_enabled = on;
    }
    #[zbus(property)]
    fn wireless_hardware_enabled(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn active_connections(&self) -> Vec<OwnedObjectPath> {
        self.active.clone()
    }

    async fn activate_connection(
        &mut self,
        connection: OwnedObjectPath,
        _device: OwnedObjectPath,
        _specific_object: OwnedObjectPath,
        #[zbus(object_server)] server: &ObjectServer,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<OwnedObjectPath> {
        let saved = server.interface::<_, Connection>(connection.as_str()).await?;
        let settings = clone_settings(&saved.get().await.settings);
        self.activate(server, &emitter, connection.as_str(), &settings).await
    }

    async fn add_and_activate_connection2(
        &mut self,
        mut settings: Settings,
        _device: OwnedObjectPath,
        _specific_object: OwnedObjectPath,
        _options: HashMap<String, OwnedValue>,
        #[zbus(object_server)] server: &ObjectServer,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<(OwnedObjectPath, OwnedObjectPath, HashMap<String, OwnedValue>)> {
        self.log.lock().unwrap().added.push(clone_settings(&settings));
        self.next += 1;
        let uuid = format!("00000000-0000-0000-0000-{:012}", self.next);
        settings
            .entry("connection".into())
            .or_default()
            .insert("uuid".into(), OwnedValue::try_from(Value::from(uuid)).unwrap());
        let connection = format!("{NM_PATH}/Settings/{}", 100 + self.next);
        server
            .at(
                connection.as_str(),
                Connection { log: self.log.clone(), settings: clone_settings(&settings) },
            )
            .await?;
        let active = self.activate(server, &emitter, &connection, &settings).await?;
        Ok((path(&connection), active, HashMap::new()))
    }

    async fn deactivate_connection(
        &mut self,
        active: OwnedObjectPath,
        #[zbus(object_server)] server: &ObjectServer,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        server.remove::<Active, _>(active.as_str()).await?;
        self.active.retain(|a| *a != active);
        self.active_connections_changed(&emitter).await?;
        Ok(())
    }
}

struct SettingsRoot;

#[zbus::interface(name = "org.freedesktop.NetworkManager.Settings")]
impl SettingsRoot {
    #[zbus(property)]
    fn hostname(&self) -> String {
        "testhost".into()
    }

    async fn get_connection_by_uuid(
        &self,
        uuid: String,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> fdo::Result<OwnedObjectPath> {
        for n in [1, 101, 102, 103, 104, 105, 106] {
            let p = format!("{NM_PATH}/Settings/{n}");
            if let Ok(connection) = server.interface::<_, Connection>(p.as_str()).await
                && setting_str(&connection.get().await.settings, "connection", "uuid").as_deref()
                    == Some(uuid.as_str())
            {
                return Ok(path(&p));
            }
        }
        Err(fdo::Error::Failed(format!("no connection {uuid}")))
    }
}

struct Connection {
    log: Arc<Mutex<Log>>,
    settings: Settings,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.Settings.Connection")]
impl Connection {
    #[zbus(property)]
    fn filename(&self) -> String {
        String::new()
    }

    fn get_settings(&self) -> Settings {
        clone_settings(&self.settings)
    }

    async fn delete(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> fdo::Result<()> {
        let me = header.path().map(|p| p.to_string()).unwrap_or_default();
        self.log.lock().unwrap().deleted.push(me.clone());
        server.remove::<Connection, _>(me.as_str()).await?;
        Ok(())
    }

    async fn update2(
        &mut self,
        settings: Settings,
        flags: u32,
        _args: HashMap<String, OwnedValue>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<HashMap<String, OwnedValue>> {
        self.log.lock().unwrap().updated.push((clone_settings(&settings), flags));
        self.settings = settings;
        Connection::updated(&emitter).await?;
        Ok(HashMap::new())
    }

    fn save(&self) {}

    #[zbus(signal)]
    async fn updated(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

struct Device {
    reason: u32,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.Device")]
impl Device {
    #[zbus(property)]
    fn interface(&self) -> String {
        "wlan0".into()
    }
    #[zbus(property)]
    fn device_type(&self) -> u32 {
        2
    }
    #[zbus(property)]
    fn state(&self) -> u32 {
        30
    }
    #[zbus(property)]
    fn state_reason(&self) -> (u32, u32) {
        (30, self.reason)
    }
    #[zbus(property)]
    fn managed(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn real(&self) -> bool {
        true
    }
    fn disconnect(&self) {}
}

struct Wireless {
    log: Arc<Mutex<Log>>,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.Device.Wireless")]
impl Wireless {
    #[zbus(property)]
    fn access_points(&self) -> Vec<OwnedObjectPath> {
        vec![path("/org/freedesktop/NetworkManager/AccessPoint/1"), path("/org/freedesktop/NetworkManager/AccessPoint/2"), path("/org/freedesktop/NetworkManager/AccessPoint/3")]
    }
    fn request_scan(&self, _options: HashMap<String, OwnedValue>) {
        self.log.lock().unwrap().scans += 1;
    }
}

struct AccessPoint {
    ssid: &'static str,
    rsn: u32,
    strength: u8,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.AccessPoint")]
impl AccessPoint {
    #[zbus(property)]
    fn ssid(&self) -> Vec<u8> {
        self.ssid.as_bytes().to_vec()
    }
    #[zbus(property)]
    fn flags(&self) -> u32 {
        u32::from(self.rsn != 0)
    }
    #[zbus(property)]
    fn wpa_flags(&self) -> u32 {
        0
    }
    #[zbus(property)]
    fn rsn_flags(&self) -> u32 {
        self.rsn
    }
    #[zbus(property)]
    fn frequency(&self) -> u32 {
        2437
    }
    #[zbus(property)]
    fn strength(&self) -> u8 {
        self.strength
    }
    #[zbus(property)]
    fn mode(&self) -> u32 {
        2
    }
}

struct Active {
    connection: OwnedObjectPath,
    uuid: String,
    id: String,
    state: u32,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.Connection.Active")]
impl Active {
    #[zbus(property)]
    fn connection(&self) -> OwnedObjectPath {
        self.connection.clone()
    }
    #[zbus(property)]
    fn uuid(&self) -> String {
        self.uuid.clone()
    }
    #[zbus(property)]
    fn id(&self) -> String {
        self.id.clone()
    }
    #[zbus(property, name = "Type")]
    fn kind(&self) -> String {
        "802-11-wireless".into()
    }
    #[zbus(property)]
    fn state(&self) -> u32 {
        self.state
    }
    #[zbus(property)]
    fn devices(&self) -> Vec<OwnedObjectPath> {
        vec![path(DEVICE)]
    }
}

fn value(v: impl Into<Value<'static>>) -> OwnedValue {
    OwnedValue::try_from(v.into()).unwrap()
}

/// Serve the fake: one Wi-Fi device seeing Home (WPA2, saved), Office
/// (WPA2) and Cafe (open).
fn serve(bus: &common::TestBus) -> (zbus::blocking::Connection, Arc<Mutex<Log>>) {
    let log = Arc::new(Mutex::new(Log::default()));
    let home: Settings = HashMap::from([
        (
            "connection".to_owned(),
            HashMap::from([
                ("id".to_owned(), value("Home")),
                ("uuid".to_owned(), value("11111111-1111-1111-1111-111111111111")),
                ("type".to_owned(), value("802-11-wireless")),
                ("timestamp".to_owned(), value(1_700_000_000u64)),
            ]),
        ),
        ("802-11-wireless".to_owned(), HashMap::from([("ssid".to_owned(), value(b"Home".to_vec()))])),
        (
            "802-11-wireless-security".to_owned(),
            HashMap::from([("key-mgmt".to_owned(), value("wpa-psk")), ("psk".to_owned(), value("right"))]),
        ),
    ]);
    let conn = bus
        .builder()
        .name("org.freedesktop.NetworkManager")
        .unwrap()
        .serve_at("/org/freedesktop", zbus::fdo::ObjectManager)
        .unwrap()
        .serve_at(
            NM_PATH,
            Manager { log: log.clone(), wireless_enabled: true, active: Vec::new(), next: 0 },
        )
        .unwrap()
        .serve_at(format!("{NM_PATH}/Settings"), SettingsRoot)
        .unwrap()
        .serve_at(format!("{NM_PATH}/Settings/1"), Connection { log: log.clone(), settings: home })
        .unwrap()
        .serve_at(DEVICE, Device { reason: 0 })
        .unwrap()
        .serve_at(DEVICE, Wireless { log: log.clone() })
        .unwrap()
        .serve_at("/org/freedesktop/NetworkManager/AccessPoint/1", AccessPoint { ssid: "Home", rsn: 0x188, strength: 70 })
        .unwrap()
        .serve_at("/org/freedesktop/NetworkManager/AccessPoint/2", AccessPoint { ssid: "Office", rsn: 0x188, strength: 60 })
        .unwrap()
        .serve_at("/org/freedesktop/NetworkManager/AccessPoint/3", AccessPoint { ssid: "Cafe", rsn: 0, strength: 90 })
        .unwrap()
        .build()
        .unwrap();
    (conn, log)
}

fn no_radios() -> RadioPaths {
    let missing = std::env::temp_dir().join("aizome-sys-no-rfkill");
    RadioPaths { device: missing.clone(), sysfs: missing }
}

#[test]
fn follows_networks_and_switches_wifi() {
    let bus = test_bus!();
    let (_fake, _log) = serve(&bus);
    let network = Network::with_bus(bus.bus(), Wake::none(), no_radios());
    assert!(eventually(|| network.snapshot().networks.len() == 3), "{:?}", network.snapshot());
    let state = network.snapshot();
    assert!(state.available && state.wifi_enabled);
    assert_eq!(state.hostname, "testhost");
    let names: Vec<&str> = state.networks.iter().map(|n| n.ssid.as_str()).collect();
    assert_eq!(names, ["Cafe", "Home", "Office"]);
    let home = state.network("Home").unwrap();
    assert!(home.known && !home.connected);
    assert_eq!(home.security, Security::Wpa2Personal);
    assert_eq!(state.wifi_device().unwrap().interface, "wlan0");

    let seen = network.generation();
    network.set_wifi_enabled(false).wait().unwrap();
    assert!(eventually(|| !network.snapshot().wifi_enabled));
    assert!(network.generation() > seen);
}

#[test]
fn joining_a_new_network_sends_its_password() {
    let bus = test_bus!();
    let (_fake, log) = serve(&bus);
    let network = Network::with_bus(bus.bus(), Wake::none(), no_radios());
    assert!(eventually(|| network.snapshot().network("Office").is_some()));
    let office = network.snapshot().network("Office").unwrap().clone();

    // A network that needs a password isn't joined without one.
    assert!(matches!(network.connect(&office, None).wait(), Err(Error::InvalidArgument(_))));

    network.connect(&office, Some("right")).wait().unwrap();
    {
        let log = log.lock().unwrap();
        let added = &log.added[0];
        assert_eq!(setting_str(added, "802-11-wireless-security", "key-mgmt").as_deref(), Some("wpa-psk"));
        assert_eq!(setting_str(added, "802-11-wireless-security", "psk").as_deref(), Some("right"));
        assert_eq!(setting_str(added, "connection", "id").as_deref(), Some("Office"));
    }
    // The new profile shows up, known and connected.
    assert!(eventually(|| network.snapshot().network("Office").is_some_and(|n| n.known)));
    assert!(eventually(|| network.snapshot().network("Office").is_some_and(|n| n.connected)));
}

#[test]
fn a_refused_password_leaves_nothing_behind() {
    let bus = test_bus!();
    let (_fake, log) = serve(&bus);
    let network = Network::with_bus(bus.bus(), Wake::none(), no_radios());
    assert!(eventually(|| network.snapshot().network("Office").is_some()));
    let office = network.snapshot().network("Office").unwrap().clone();
    let result = network.connect(&office, Some("wrong")).wait();
    assert!(matches!(result, Err(Error::Rejected(_))), "{result:?}");
    assert_eq!(log.lock().unwrap().deleted.len(), 1, "the new profile is removed");
}

#[test]
fn saved_networks_join_and_change() {
    let bus = test_bus!();
    let (_fake, log) = serve(&bus);
    let network = Network::with_bus(bus.bus(), Wake::none(), no_radios());
    assert!(eventually(|| network.snapshot().network("Home").is_some_and(|n| n.known)));
    let home = network.snapshot().network("Home").unwrap().clone();
    let uuid = home.connection.clone().unwrap();

    network.connect(&home, None).wait().unwrap();
    assert_eq!(log.lock().unwrap().activated, [format!("{NM_PATH}/Settings/1")]);
    assert!(eventually(|| network.snapshot().connection(&uuid).is_some_and(|c| c.active)));

    network.set_autoconnect(&uuid, false).wait().unwrap();
    {
        let log = log.lock().unwrap();
        let (settings, flags) = log.updated.last().unwrap();
        assert_eq!(*flags, 1, "saved to disk");
        let autoconnect = settings["connection"]["autoconnect"].try_clone().unwrap();
        assert!(!bool::try_from(autoconnect).unwrap());
        // Everything else is kept.
        assert_eq!(setting_str(settings, "connection", "id").as_deref(), Some("Home"));
    }
    assert!(eventually(|| network.snapshot().connection(&uuid).is_some_and(|c| !c.autoconnect)));

    network.deactivate(&uuid).wait().unwrap();
    assert!(eventually(|| network.snapshot().active.is_empty()));

    network.scan().wait().unwrap();
    assert_eq!(log.lock().unwrap().scans, 1);

    network.forget(&uuid).wait().unwrap();
    assert!(eventually(|| network.snapshot().connection(&uuid).is_none()));
    assert!(!network.snapshot().network("Home").unwrap().known);
}

#[test]
fn without_networkmanager_it_waits_for_it() {
    let bus = test_bus!();
    let network = Network::with_bus(bus.bus(), Wake::none(), no_radios());
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(!network.available());
    assert!(network.snapshot().networks.is_empty());
    let result = network.set_wifi_enabled(true).wait();
    assert!(matches!(result, Err(Error::Unavailable(_))), "{result:?}");

    // NetworkManager starts later: the service notices.
    let (fake, _log) = serve(&bus);
    assert!(eventually(|| network.available() && network.snapshot().networks.len() == 3));
    // And notices it leaving.
    drop(fake);
    assert!(eventually(|| !network.available()));
}

#[test]
fn airplane_mode_blocks_every_radio() {
    let bus = test_bus!();
    let dir = std::env::temp_dir().join(format!("aizome-sys-airplane-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let device = dir.join("rfkill");
    std::fs::write(&device, b"").unwrap();
    let network =
        Network::with_bus(bus.bus(), Wake::none(), RadioPaths { device: device.clone(), sysfs: dir.join("none") });
    network.set_airplane_mode(true).wait().unwrap();
    let written = std::fs::read(&device).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    // struct rfkill_event { idx 0, type ALL, op CHANGE_ALL, soft 1, hard 0 }
    assert_eq!(written, [0, 0, 0, 0, 0, 3, 1, 0]);
}

#[test]
fn addresses_and_dns_are_saved_on_the_profile() {
    use aizome_sys::network::Ipv4Method;
    use std::net::Ipv4Addr;
    let bus = test_bus!();
    let (_fake, log) = serve(&bus);
    let network = Network::with_bus(bus.bus(), Wake::none(), no_radios());
    assert!(eventually(|| network.snapshot().network("Home").is_some_and(|n| n.known)));
    let uuid = network.snapshot().network("Home").unwrap().connection.clone().unwrap();
    network.set_dns(&uuid, &["9.9.9.9".parse().unwrap()], false).wait().unwrap();
    {
        let log = log.lock().unwrap();
        let (settings, _) = log.updated.last().unwrap();
        let dns = Vec::<u32>::try_from(settings["ipv4"]["dns"].try_clone().unwrap()).unwrap();
        assert_eq!(dns, [u32::from_ne_bytes([9, 9, 9, 9])]);
        assert!(bool::try_from(settings["ipv4"]["ignore-auto-dns"].try_clone().unwrap()).unwrap());
        // The password and the rest are kept.
        assert_eq!(setting_str(settings, "802-11-wireless-security", "psk").as_deref(), Some("right"));
    }
    let manual = Ipv4Method::Manual { addresses: vec![(Ipv4Addr::new(10, 0, 0, 5), 24)], gateway: Some(Ipv4Addr::new(10, 0, 0, 1)) };
    network.set_ipv4(&uuid, manual).wait().unwrap();
    assert_eq!(setting_str(&log.lock().unwrap().updated.last().unwrap().0, "ipv4", "method").as_deref(), Some("manual"));
    assert!(network.set_ipv4(&uuid, Ipv4Method::Manual { addresses: vec![], gateway: None }).wait().is_err());
}

#[test]
fn profiles_read_whole_and_change_ipv6_and_the_private_address() {
    let bus = test_bus!();
    let (_fake, log) = serve(&bus);
    let network = Network::with_bus(bus.bus(), Wake::none(), no_radios());
    assert!(eventually(|| network.snapshot().network("Home").is_some_and(|n| n.known)));
    let uuid = network.snapshot().network("Home").unwrap().connection.clone().unwrap();
    let profile = network.profile(&uuid).wait().unwrap();
    assert_eq!(profile.id, "Home");
    assert_eq!(profile.ssid.as_deref(), Some("Home"));
    assert_eq!(profile.key_mgmt.as_deref(), Some("wpa-psk"));
    assert_eq!(profile.ipv4_method, "auto");

    network.set_ipv6_method(&uuid, "disabled").wait().unwrap();
    network.set_private_address(&uuid, "stable").wait().unwrap();
    {
        let log = log.lock().unwrap();
        let (settings, _) = log.updated.last().unwrap();
        assert_eq!(setting_str(settings, "ipv6", "method").as_deref(), Some("disabled"));
        assert_eq!(setting_str(settings, "802-11-wireless", "assigned-mac-address").as_deref(), Some("stable"));
        assert_eq!(setting_str(settings, "802-11-wireless-security", "psk").as_deref(), Some("right"));
    }
    assert!(network.set_ipv6_method(&uuid, "sideways").wait().is_err());
    // Not up: nothing to reapply, and no error.
    assert!(!network.reapply(&uuid).wait().unwrap());
}
