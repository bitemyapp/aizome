//! Session-scoped NetworkManager SecretAgent. The daemon owns registration,
//! cancellation and keyring access; short-lived native dialogs own only UI.
pub mod keyring;
pub mod model;
pub mod process;
pub mod prompt;
pub mod vpn;

use keyring::Store;
use model::{AGENT_OWNED, ALLOW_INTERACTION, NOT_REQUIRED, NOT_SAVED, Profile, Prompt, REQUEST_NEW, Settings};
use process::Cancel;
use prompt::Prompter;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zbus::{message::Header, zvariant::OwnedObjectPath};

pub const NM: &str = "org.freedesktop.NetworkManager";
pub const PATH: &str = "/org/freedesktop/NetworkManager/SecretAgent";
pub const IDENTIFIER: &str = "org.aizome.NetworkAgent";
pub type Result<T> = std::result::Result<T, AgentError>;

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.NetworkManager.SecretAgent")]
pub enum AgentError {
    Failed(String),
    InvalidConnection(String),
    UserCanceled(String),
    AgentCanceled(String),
    NoSecrets(String),
    NotAuthorized(String),
}

pub struct Engine {
    pub store: Arc<dyn Store>,
    pub ui: Arc<dyn Prompter>,
    pub plugins: vpn::Plugins,
}
impl Engine {
    pub fn get(&self, p: Profile, setting: &str, hints: &[String], flags: u32, cancel: &Cancel) -> Result<Settings> {
        cancel.check()?;
        let interactive = flags & ALLOW_INTERACTION != 0;
        let mut values = p.supplied(setting);
        let fields = if setting == "vpn" { Vec::new() } else { p.fields(setting, hints)? };
        let needs_keyring = if setting == "vpn" {
            model::dict(&p.settings, "vpn", "data")
                .iter()
                .any(|(key, value)| key.ends_with("-flags") && value.parse::<u32>().ok() == Some(AGENT_OWNED))
        } else {
            fields.iter().any(|f| p.flags(setting, &f.key) == AGENT_OWNED)
        };
        let saved = if !needs_keyring || flags & REQUEST_NEW != 0 {
            HashMap::new()
        } else {
            match self.store.lookup(&p.uuid, setting, interactive, cancel) {
                Ok(values) => values,
                Err(_) if interactive => {
                    cancel.check()?;
                    HashMap::new()
                }
                Err(_) => return Err(AgentError::NoSecrets("Stored credentials are unavailable".into())),
            }
        };
        for (key, value) in saved {
            if p.flags(setting, &key) & (AGENT_OWNED | NOT_SAVED | NOT_REQUIRED) == AGENT_OWNED {
                values.entry(key).or_insert(value);
            }
        }
        let response = if setting == "vpn" {
            self.plugins.secrets(&p, hints, flags, values, self.ui.as_ref(), cancel)?
        } else {
            let mut fields = fields;
            let mut answer = HashMap::new();
            fields.retain(|f| {
                if flags & REQUEST_NEW == 0
                    && let Some(value) = values.get(&f.key).filter(|v| !v.is_empty())
                {
                    answer.insert(f.key.clone(), value.clone());
                    false
                } else {
                    true
                }
            });
            if !fields.is_empty() {
                if !interactive {
                    return Err(AgentError::NoSecrets("Network authentication needs user interaction".into()));
                }
                let requested: Vec<_> = fields.iter().map(|f| f.key.clone()).collect();
                let entered = self.ui.ask(
                    Prompt {
                        title: "Network Authentication".into(),
                        message: format!("Enter credentials for “{}”.", p.name),
                        fields,
                    },
                    cancel,
                )?;
                for key in requested {
                    let value = entered
                        .get(&key)
                        .filter(|v| !v.is_empty())
                        .ok_or_else(|| AgentError::NoSecrets("Required credential was not supplied".into()))?;
                    answer.insert(key, value.clone());
                }
            }
            answer
        };
        cancel.check()?;
        // Preserve each existing profile's storage policy. System-owned
        // values go back to NM; one-time/ask-every-time values are never saved.
        for (key, value) in &response {
            if interactive
                && p.flags(setting, key) == AGENT_OWNED
                && self.store.store(&p.uuid, setting, key, value, &format!("{} — {setting}", p.name), cancel).is_err()
            {
                cancel.check()?;
                // Failure to remember a password must not prevent using it.
                log::warn!("The keyring could not save network credentials");
            }
        }
        Ok(p.reply(setting, response))
    }
    pub fn save(&self, p: Profile, cancel: &Cancel) -> Result<()> {
        // SaveSecrets replaces a profile's saved secrets, including when
        // an editor changes a password to "ask every time".
        self.store.delete(&p.uuid, cancel)?;
        for setting in p.settings.keys() {
            let values = p.supplied(setting);
            for (key, value) in values {
                if p.flags(setting, &key) & (AGENT_OWNED | NOT_SAVED | NOT_REQUIRED) == AGENT_OWNED {
                    self.store.store(&p.uuid, setting, &key, &value, &format!("{} — {setting}", p.name), cancel)?;
                }
            }
        }
        Ok(())
    }
}

type Pending = Arc<Mutex<HashMap<(String, String), Cancel>>>;
pub struct Agent {
    connection: zbus::Connection,
    engine: Arc<Engine>,
    pending: Pending,
}
impl Agent {
    pub fn new(connection: zbus::Connection, engine: Arc<Engine>) -> Self {
        Self { connection, engine, pending: Arc::default() }
    }
    async fn authorize(&self, header: &Header<'_>) -> Result<()> {
        let proxy = zbus::fdo::DBusProxy::new(&self.connection)
            .await
            .map_err(|_| AgentError::NotAuthorized("NetworkManager is unavailable".into()))?;
        let owner = proxy
            .get_name_owner(NM.try_into().unwrap())
            .await
            .map_err(|_| AgentError::NotAuthorized("NetworkManager is unavailable".into()))?;
        if header.sender().map(|n| n.as_str()) != Some(owner.as_str()) {
            return Err(AgentError::NotAuthorized("Only NetworkManager may request credentials".into()));
        }
        Ok(())
    }
    pub fn cancel_all(&self) {
        for token in self.pending.lock().unwrap().drain().map(|(_, v)| v) {
            token.cancel();
        }
    }
    async fn work<T: Send + 'static>(&self, f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(f());
        });
        rx.recv().await.map_err(|_| AgentError::Failed("Credential worker stopped".into()))?
    }
}
#[zbus::interface(name = "org.freedesktop.NetworkManager.SecretAgent")]
impl Agent {
    async fn get_secrets(
        &self,
        connection: Settings,
        connection_path: OwnedObjectPath,
        setting_name: String,
        hints: Vec<String>,
        flags: u32,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Settings> {
        self.authorize(&header).await?;
        let profile = Profile::new(connection)?;
        let key = (connection_path.to_string(), setting_name.clone());
        let cancel = Cancel::default();
        {
            let mut pending = self.pending.lock().unwrap();
            if pending.len() >= 16 && !pending.contains_key(&key) {
                return Err(AgentError::Failed("Too many outstanding authentication requests".into()));
            }
            if let Some(old) = pending.insert(key.clone(), cancel.clone()) {
                old.cancel();
            }
        }
        let engine = self.engine.clone();
        let token = cancel.clone();
        let result = self.work(move || engine.get(profile, &setting_name, &hints, flags, &token)).await;
        let mut pending = self.pending.lock().unwrap();
        if pending.get(&key).is_some_and(|t| Arc::ptr_eq(&t.0, &cancel.0)) {
            pending.remove(&key);
        }
        result
    }
    async fn cancel_get_secrets(
        &self,
        connection_path: OwnedObjectPath,
        setting_name: String,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<()> {
        self.authorize(&header).await?;
        if let Some(token) = self.pending.lock().unwrap().remove(&(connection_path.to_string(), setting_name)) {
            token.cancel();
        }
        Ok(())
    }
    async fn save_secrets(
        &self,
        connection: Settings,
        _connection_path: OwnedObjectPath,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<()> {
        self.authorize(&header).await?;
        let profile = Profile::new(connection)?;
        let engine = self.engine.clone();
        self.work(move || engine.save(profile, &Cancel::default())).await
    }
    async fn delete_secrets(
        &self,
        connection: Settings,
        _connection_path: OwnedObjectPath,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<()> {
        self.authorize(&header).await?;
        let profile = Profile::new(connection)?;
        let engine = self.engine.clone();
        self.work(move || engine.store.delete(&profile.uuid, &Cancel::default())).await
    }
}

/// Keep the same agent process across shell reloads. Re-register after a
/// NetworkManager restart; cancel requests belonging to its old owner.
pub fn serve() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let _singleton = zbus::blocking::connection::Builder::session()?.name(IDENTIFIER)?.build()?;
    let connection = zbus::blocking::Connection::system()?;
    let engine = Arc::new(Engine {
        store: Arc::new(keyring::Keyring),
        ui: Arc::new(prompt::Native),
        plugins: vpn::Plugins::default(),
    });
    let agent = Agent::new(connection.inner().clone(), engine);
    connection.object_server().at(PATH, agent)?;
    let bus = zbus::blocking::fdo::DBusProxy::new(&connection)?;
    let mut registered = None;
    loop {
        let owner = bus.get_name_owner(NM.try_into().unwrap()).ok().map(|n| n.to_string());
        if owner != registered {
            connection.object_server().interface::<_, Agent>(PATH)?.get().cancel_all();
            registered = None;
            if owner.is_some() {
                let manager = zbus::blocking::Proxy::new(
                    &connection,
                    NM,
                    "/org/freedesktop/NetworkManager/AgentManager",
                    "org.freedesktop.NetworkManager.AgentManager",
                )?;
                // VPN_HINTS: pass plugin challenge hints through unchanged.
                match manager.call::<_, _, ()>("RegisterWithCapabilities", &(IDENTIFIER, 1u32)) {
                    Ok(()) => {
                        registered = owner;
                        log::info!("Network credential agent registered");
                    }
                    Err(_) => log::warn!("Network credential agent registration deferred"),
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
