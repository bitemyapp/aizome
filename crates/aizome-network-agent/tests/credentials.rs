use aizome_network_agent::{
    Agent, AgentError, Engine, NM, PATH, Result,
    keyring::Store,
    model::{self, Profile, Prompt, Settings, text_value},
    process::Cancel,
    prompt::Prompter,
    vpn::Plugins,
};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use zbus::zvariant::{OwnedObjectPath, Value};

const UUID: &str = "11111111-2222-3333-4444-555555555555";
#[derive(Default)]
struct Memory {
    values: Mutex<HashMap<(String, String, String), String>>,
    lookups: AtomicUsize,
}
impl Store for Memory {
    fn lookup(&self, uuid: &str, setting: &str, _: bool, _: &Cancel) -> Result<HashMap<String, String>> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        Ok(self
            .values
            .lock()
            .unwrap()
            .iter()
            .filter(|((u, s, _), _)| u == uuid && s == setting)
            .map(|((_, _, k), v)| (k.clone(), v.clone()))
            .collect())
    }
    fn store(&self, uuid: &str, setting: &str, key: &str, value: &str, _: &str, _: &Cancel) -> Result<()> {
        self.values.lock().unwrap().insert((uuid.into(), setting.into(), key.into()), value.into());
        Ok(())
    }
    fn delete(&self, uuid: &str, _: &Cancel) -> Result<()> {
        self.values.lock().unwrap().retain(|(u, _, _), _| u != uuid);
        Ok(())
    }
}
struct Ui {
    asked: Mutex<Vec<Prompt>>,
    answer: Option<String>,
    wait: bool,
}
impl Prompter for Ui {
    fn ask(&self, prompt: Prompt, c: &Cancel) -> Result<HashMap<String, String>> {
        self.asked.lock().unwrap().push(prompt.clone());
        if self.wait {
            loop {
                c.check()?;
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        let value = self.answer.as_ref().ok_or_else(|| AgentError::UserCanceled("Cancelled".into()))?;
        Ok(prompt.fields.iter().map(|f| (f.key.clone(), value.clone())).collect())
    }
}
fn profile(flags: u32) -> Settings {
    HashMap::from([
        (
            "connection".into(),
            HashMap::from([
                ("uuid".into(), text_value(UUID.into())),
                ("id".into(), text_value("Test AP".into())),
                ("type".into(), text_value("802-11-wireless".into())),
            ]),
        ),
        (
            "802-11-wireless-security".into(),
            HashMap::from([("key-mgmt".into(), text_value("wpa-psk".into())), ("psk-flags".into(), flags.into())]),
        ),
    ])
}
fn fixture(answer: Option<&str>, wait: bool) -> (Arc<Memory>, Arc<Ui>, Arc<Engine>) {
    let store = Arc::new(Memory::default());
    let ui = Arc::new(Ui { asked: Mutex::default(), answer: answer.map(str::to_owned), wait });
    let engine = Arc::new(Engine { store: store.clone(), ui: ui.clone(), plugins: Plugins { dirs: vec![] } });
    (store, ui, engine)
}

#[test]
fn storage_policy_and_noninteractive_requests() {
    for flags in [0, 1, 2, 3, 4, 5] {
        let (store, ui, engine) = fixture(Some("entered-secret"), false);
        let result = engine.get(
            Profile::new(profile(flags)).unwrap(),
            "802-11-wireless-security",
            &[],
            model::ALLOW_INTERACTION,
            &Cancel::default(),
        );
        if flags & model::NOT_REQUIRED != 0 {
            assert!(matches!(result, Err(AgentError::NoSecrets(_))));
        } else {
            assert_eq!(model::string(&result.unwrap(), "802-11-wireless-security", "psk"), "entered-secret");
        }
        assert_eq!(store.values.lock().unwrap().len(), usize::from(flags == 1), "flags {flags}");
        assert_eq!(ui.asked.lock().unwrap().len(), usize::from(flags & 4 == 0));
    }
    let (_, ui, engine) = fixture(Some("must-not-prompt"), false);
    assert!(matches!(
        engine.get(Profile::new(profile(2)).unwrap(), "802-11-wireless-security", &[], 0, &Cancel::default()),
        Err(AgentError::NoSecrets(_))
    ));
    assert!(ui.asked.lock().unwrap().is_empty());
}

#[test]
fn existing_keyring_and_request_new_and_system_owned_passwords() {
    let (store, ui, engine) = fixture(Some("replacement"), false);
    store.store(UUID, "802-11-wireless-security", "psk", "existing", "", &Cancel::default()).unwrap();
    let result =
        engine.get(Profile::new(profile(1)).unwrap(), "802-11-wireless-security", &[], 0, &Cancel::default()).unwrap();
    assert_eq!(model::string(&result, "802-11-wireless-security", "psk"), "existing");
    assert!(ui.asked.lock().unwrap().is_empty());
    let result =
        engine.get(Profile::new(profile(1)).unwrap(), "802-11-wireless-security", &[], 3, &Cancel::default()).unwrap();
    assert_eq!(model::string(&result, "802-11-wireless-security", "psk"), "replacement");
    let mut p = profile(0);
    p.get_mut("802-11-wireless-security").unwrap().insert("psk".into(), text_value("system-owned".into()));
    let reads = store.lookups.load(Ordering::SeqCst);
    let result = engine.get(Profile::new(p).unwrap(), "802-11-wireless-security", &[], 0, &Cancel::default()).unwrap();
    assert_eq!(model::string(&result, "802-11-wireless-security", "psk"), "system-owned");
    assert_eq!(store.lookups.load(Ordering::SeqCst), reads);
    assert_eq!(ui.asked.lock().unwrap().len(), 1);
}

#[test]
fn saved_and_deleted_credentials_respect_flags() {
    let (store, _, engine) = fixture(None, false);
    let mut p = profile(1);
    p.get_mut("802-11-wireless-security").unwrap().insert("psk".into(), text_value("saved".into()));
    p.insert(
        "802-1x".into(),
        HashMap::from([("password".into(), text_value("one-time".into())), ("password-flags".into(), 2u32.into())]),
    );
    engine.save(Profile::new(p).unwrap(), &Cancel::default()).unwrap();
    assert_eq!(store.values.lock().unwrap().len(), 1);
    store.delete(UUID, &Cancel::default()).unwrap();
    assert!(store.values.lock().unwrap().is_empty());
}

#[test]
fn user_cancellation_never_saves() {
    let (store, _, engine) = fixture(None, false);
    assert!(matches!(
        engine.get(Profile::new(profile(1)).unwrap(), "802-11-wireless-security", &[], 1, &Cancel::default()),
        Err(AgentError::UserCanceled(_))
    ));
    assert!(store.values.lock().unwrap().is_empty());
}

#[test]
fn enterprise_hints_and_wep_flags() {
    let mut p = profile(0);
    p.insert(
        "802-1x".into(),
        HashMap::from([
            ("eap".into(), Value::from(vec!["tls"]).try_to_owned().unwrap()),
            ("private-key-password-flags".into(), 2u32.into()),
        ]),
    );
    let p = Profile::new(p).unwrap();
    assert_eq!(p.fields("802-1x", &[]).unwrap()[0].key, "private-key-password");
    assert_eq!(p.fields("802-1x", &["pin".into()]).unwrap()[0].key, "pin");
    let mut p = profile(0);
    let s = p.get_mut("802-11-wireless-security").unwrap();
    s.insert("key-mgmt".into(), text_value("none".into()));
    s.insert("wep-tx-keyidx".into(), 2u32.into());
    s.insert("wep-key-flags".into(), 1u32.into());
    let p = Profile::new(p).unwrap();
    assert_eq!(p.fields("802-11-wireless-security", &[]).unwrap()[0].key, "wep-key2");
    assert_eq!(p.flags("802-11-wireless-security", "wep-key2"), 1);
}

#[test]
fn only_networkmanager_can_call_and_cancellation_interrupts_pending_prompt() {
    let mut daemon = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("dbus-daemon is required");
    let mut address = String::new();
    BufReader::new(daemon.stdout.take().unwrap()).read_line(&mut address).unwrap();
    let connect = || zbus::blocking::connection::Builder::address(address.trim()).unwrap().build().unwrap();
    let nm = connect();
    nm.request_name(NM).unwrap();
    let connection = connect();
    let (store, ui, engine) = fixture(Some("secret"), true);
    connection.object_server().at(PATH, Agent::new(connection.inner().clone(), engine)).unwrap();
    let dest = connection.unique_name().unwrap().as_str().to_owned();
    let args = || {
        (
            profile(1),
            OwnedObjectPath::try_from("/test/connection").unwrap(),
            "802-11-wireless-security",
            Vec::<String>::new(),
            1u32,
        )
    };
    let attacker = connect();
    let proxy =
        zbus::blocking::Proxy::new(&attacker, dest.as_str(), PATH, "org.freedesktop.NetworkManager.SecretAgent")
            .unwrap();
    let denied = proxy.call::<_, _, Settings>("GetSecrets", &args()).unwrap_err().to_string();
    assert!(denied.contains("NotAuthorized"), "{denied}");
    assert!(ui.asked.lock().unwrap().is_empty());
    assert_eq!(store.lookups.load(Ordering::SeqCst), 0);
    for method in ["SaveSecrets", "DeleteSecrets"] {
        let denied = proxy
            .call::<_, _, ()>(method, &(profile(1), OwnedObjectPath::try_from("/test/connection").unwrap()))
            .unwrap_err()
            .to_string();
        assert!(denied.contains("NotAuthorized"), "{denied}");
    }
    let denied = proxy
        .call::<_, _, ()>(
            "CancelGetSecrets",
            &(OwnedObjectPath::try_from("/test/connection").unwrap(), "802-11-wireless-security"),
        )
        .unwrap_err()
        .to_string();
    assert!(denied.contains("NotAuthorized"), "{denied}");
    let owned = nm.clone();
    let destination = dest.clone();
    let pending = std::thread::spawn(move || {
        let p = zbus::blocking::Proxy::new(&owned, destination, PATH, "org.freedesktop.NetworkManager.SecretAgent")
            .unwrap();
        p.call::<_, _, Settings>("GetSecrets", &args())
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while ui.asked.lock().unwrap().is_empty() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let proxy =
        zbus::blocking::Proxy::new(&nm, dest.as_str(), PATH, "org.freedesktop.NetworkManager.SecretAgent").unwrap();
    proxy
        .call::<_, _, ()>(
            "CancelGetSecrets",
            &(OwnedObjectPath::try_from("/test/connection").unwrap(), "802-11-wireless-security"),
        )
        .unwrap();
    let error = pending.join().unwrap().unwrap_err().to_string();
    assert!(error.contains("AgentCanceled"), "{error}");
    assert!(store.values.lock().unwrap().is_empty());
    daemon.kill().unwrap();
    daemon.wait().unwrap();
}

struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "aizome-agent-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn script(&self, name: &str, source: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = self.0.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{source}")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn vpn_profile() -> Profile {
    let mut p = profile(0);
    p.insert(
        "vpn".into(),
        HashMap::from([
            ("service-type".into(), text_value("org.example.test-vpn".into())),
            (
                "data".into(),
                Value::from(HashMap::from([("password-flags", "2"), ("username", "fixture-user")]))
                    .try_to_owned()
                    .unwrap(),
            ),
        ]),
    );
    Profile::new(p).unwrap()
}

#[test]
fn vpn_plugins_get_hints_and_data_and_external_dialogs_honor_interaction() {
    let dir = Directory::new();
    let script = dir.script(
        "authenticate",
        r#"
case "$*" in *"--external-ui-mode"*) ;; *) exit 1 ;; esac
case "$*" in *"-t challenge:otp"*) ;; *) exit 1 ;; esac
input=$(cat)
case "$input" in *"DATA_VAL=fixture-user"*) ;; *) exit 1 ;; esac
cat <<'EOF'
[VPN Plugin UI]
Version=2
Title=Authentication
Description=Enter the token\nfrom your device.
[password]
Label=One-time password
IsSecret=true
ShouldAsk=true
Value=
[opaque-state]
Label=State
IsSecret=true
ShouldAsk=false
Value=plugin-state
EOF
"#,
    );
    std::fs::write(dir.0.join("test.name"), format!(
        "[VPN Connection]\nservice=org.example.test-vpn\n[GNOME]\nauth-dialog={}\nsupports-external-ui-mode=true\nsupports-hints=true\n",
        script.display())).unwrap();
    let plugins = Plugins { dirs: vec![dir.0.clone()] };
    let (_, ui, _) = fixture(Some("otp-response"), false);
    assert!(matches!(
        plugins.secrets(&vpn_profile(), &["challenge:otp".into()], 0, HashMap::new(), ui.as_ref(), &Cancel::default()),
        Err(AgentError::NoSecrets(_))
    ));
    assert!(ui.asked.lock().unwrap().is_empty());
    let values = plugins
        .secrets(&vpn_profile(), &["challenge:otp".into()], 3, HashMap::new(), ui.as_ref(), &Cancel::default())
        .unwrap();
    assert_eq!(values["password"], "otp-response");
    assert_eq!(values["opaque-state"], "plugin-state");
    assert_eq!(ui.asked.lock().unwrap()[0].message, "Enter the token\nfrom your device.");
}

#[test]
fn legacy_vpn_plugin_can_supply_its_own_ui_and_unknown_plugins_fail_closed() {
    let dir = Directory::new();
    let helper = dir.script("authenticate", "cat >/dev/null\nprintf 'password\\nplugin-password\\n\\n'\n");
    std::fs::write(
        dir.0.join("test.name"),
        format!("[VPN Connection]\nservice=org.example.test-vpn\n[GNOME]\nauth-dialog={}\n", helper.display()),
    )
    .unwrap();
    let plugins = Plugins { dirs: vec![dir.0.clone()] };
    let (_, ui, _) = fixture(None, false);
    let result = plugins.secrets(&vpn_profile(), &[], 1, HashMap::new(), ui.as_ref(), &Cancel::default()).unwrap();
    assert_eq!(result["password"], "plugin-password");
    assert!(ui.asked.lock().unwrap().is_empty());
    let mut p = vpn_profile();
    p.settings.get_mut("vpn").unwrap().insert("service-type".into(), text_value("/bin/sh".into()));
    assert!(matches!(
        plugins.secrets(&p, &[], 1, HashMap::new(), ui.as_ref(), &Cancel::default()),
        Err(AgentError::NoSecrets(_))
    ));
}

#[test]
fn cancellation_terminates_blocked_helpers_and_their_children() {
    let dir = Directory::new();
    let helper = dir.script("blocked", "sleep 60 &\nwait\n");
    let cancel = Cancel::default();
    let token = cancel.clone();
    let worker =
        std::thread::spawn(move || aizome_network_agent::process::run(&mut Command::new(helper), Vec::new(), &token));
    std::thread::sleep(Duration::from_millis(150));
    let start = Instant::now();
    cancel.cancel();
    assert!(matches!(worker.join().unwrap(), Err(AgentError::AgentCanceled(_))));
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn changing_to_ask_every_time_clears_previously_saved_secrets() {
    let (store, _, engine) = fixture(None, false);
    store.store(UUID, "802-11-wireless-security", "psk", "old-password", "", &Cancel::default()).unwrap();
    engine.save(Profile::new(profile(2)).unwrap(), &Cancel::default()).unwrap();
    assert!(store.values.lock().unwrap().is_empty());
}

#[test]
fn wireguard_peer_secrets_use_nm_keys_and_nested_reply() {
    let public = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    let key = format!("peers.{public}.preshared-key");
    let mut p = profile(0);
    p.insert(
        "wireguard".into(),
        HashMap::from([
            ("private-key-flags".into(), 1u32.into()),
            (
                "peers".into(),
                Value::from(vec![HashMap::from([
                    ("public-key", text_value(public.into())),
                    ("preshared-key-flags", 2u32.into()),
                ])])
                .try_to_owned()
                .unwrap(),
            ),
        ]),
    );
    let p = Profile::new(p).unwrap();
    assert_eq!(p.fields("wireguard", &[]).unwrap().len(), 2);
    assert_eq!(p.fields("wireguard", std::slice::from_ref(&key)).unwrap()[0].key, key);
    assert_eq!(p.flags("wireguard", &key), 2);
    let (_, _, engine) = fixture(Some("credential"), false);
    let reply = engine.get(p, "wireguard", &[], 1, &Cancel::default()).unwrap();
    assert_eq!(model::string(&reply, "wireguard", "private-key"), "credential");
    let peers: Vec<HashMap<String, zbus::zvariant::OwnedValue>> =
        reply["wireguard"]["peers"].try_clone().unwrap().try_into().unwrap();
    assert_eq!(<&str>::try_from(&peers[0]["public-key"]).unwrap(), public);
    assert_eq!(<&str>::try_from(&peers[0]["preshared-key"]).unwrap(), "credential");
}

struct UnavailableKeyring;
impl Store for UnavailableKeyring {
    fn lookup(&self, _: &str, _: &str, _: bool, _: &Cancel) -> Result<HashMap<String, String>> {
        Err(AgentError::Failed("Keyring unavailable".into()))
    }
    fn store(&self, _: &str, _: &str, _: &str, _: &str, _: &str, _: &Cancel) -> Result<()> {
        Err(AgentError::Failed("Keyring unavailable".into()))
    }
    fn delete(&self, _: &str, _: &Cancel) -> Result<()> {
        Ok(())
    }
}

#[test]
fn unavailable_keyring_does_not_prevent_interactive_authentication() {
    let (_, ui, _) = fixture(Some("usable-password"), false);
    let engine = Engine { store: Arc::new(UnavailableKeyring), ui: ui.clone(), plugins: Plugins { dirs: vec![] } };
    assert!(matches!(
        engine.get(Profile::new(profile(1)).unwrap(), "802-11-wireless-security", &[], 0, &Cancel::default()),
        Err(AgentError::NoSecrets(_))
    ));
    assert!(ui.asked.lock().unwrap().is_empty());
    let reply =
        engine.get(Profile::new(profile(1)).unwrap(), "802-11-wireless-security", &[], 1, &Cancel::default()).unwrap();
    assert_eq!(model::string(&reply, "802-11-wireless-security", "psk"), "usable-password");
}
