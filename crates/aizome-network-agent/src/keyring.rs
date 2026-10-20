use crate::{
    AgentError, Result,
    process::{self, Cancel},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::process::Command;

#[derive(Serialize, Deserialize)]
pub struct Request {
    pub action: String,
    pub uuid: String,
    #[serde(default)]
    pub setting: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub unlock: bool,
}

pub trait Store: Send + Sync {
    fn lookup(&self, uuid: &str, setting: &str, unlock: bool, cancel: &Cancel) -> Result<HashMap<String, String>>;
    fn store(&self, uuid: &str, setting: &str, key: &str, value: &str, label: &str, cancel: &Cancel) -> Result<()>;
    fn delete(&self, uuid: &str, cancel: &Cancel) -> Result<()>;
}

pub struct Keyring;
impl Keyring {
    fn call(&self, r: Request, cancel: &Cancel) -> Result<HashMap<String, String>> {
        let exe = std::env::current_exe().map_err(|_| AgentError::Failed("Cannot locate keyring worker".into()))?;
        let bytes = process::run(
            Command::new(exe).arg("--keyring"),
            serde_json::to_vec(&r).map_err(|_| AgentError::Failed("Invalid keyring request".into()))?,
            cancel,
            AgentError::Failed("Cannot access the login keyring".into()),
        )?;
        serde_json::from_slice(&bytes).map_err(|_| AgentError::Failed("Cannot access the login keyring".into()))
    }
}
impl Store for Keyring {
    fn lookup(&self, uuid: &str, setting: &str, unlock: bool, c: &Cancel) -> Result<HashMap<String, String>> {
        self.call(
            Request {
                action: "lookup".into(),
                uuid: uuid.into(),
                setting: setting.into(),
                key: String::new(),
                value: String::new(),
                label: String::new(),
                unlock,
            },
            c,
        )
    }
    fn store(&self, uuid: &str, setting: &str, key: &str, value: &str, label: &str, c: &Cancel) -> Result<()> {
        self.call(
            Request {
                action: "store".into(),
                uuid: uuid.into(),
                setting: setting.into(),
                key: key.into(),
                value: value.into(),
                label: label.into(),
                unlock: true,
            },
            c,
        )
        .map(drop)
    }
    fn delete(&self, uuid: &str, c: &Cancel) -> Result<()> {
        self.call(
            Request {
                action: "delete".into(),
                uuid: uuid.into(),
                setting: String::new(),
                key: String::new(),
                value: String::new(),
                label: String::new(),
                unlock: false,
            },
            c,
        )
        .map(drop)
    }
}

unsafe extern "C" {
    fn aizome_keyring(
        action: *const c_char,
        uuid: *const c_char,
        setting: *const c_char,
        key: *const c_char,
        value: *const c_char,
        label: *const c_char,
        unlock: i32,
        emit: extern "C" fn(*mut c_void, *const c_char, *const c_char),
        context: *mut c_void,
    ) -> i32;
}
extern "C" fn collect(context: *mut c_void, key: *const c_char, value: *const c_char) {
    // Pointers are owned by libsecret and valid for the duration of this callback.
    unsafe {
        let map = &mut *context.cast::<HashMap<String, String>>();
        map.insert(CStr::from_ptr(key).to_string_lossy().into(), CStr::from_ptr(value).to_string_lossy().into());
    }
}
pub fn worker(r: Request) -> Result<HashMap<String, String>> {
    if !matches!(r.action.as_str(), "lookup" | "store" | "delete") || r.uuid.is_empty() {
        return Err(AgentError::InvalidConnection("Invalid keyring operation".into()));
    }
    let args: Vec<_> = [&r.action, &r.uuid, &r.setting, &r.key, &r.value, &r.label]
        .into_iter()
        .map(|s| CString::new(s.as_str()))
        .collect::<std::result::Result<_, _>>()
        .map_err(|_| AgentError::InvalidConnection("Invalid credential encoding".into()))?;
    let mut values = HashMap::<String, String>::new();
    let result = unsafe {
        aizome_keyring(
            args[0].as_ptr(),
            args[1].as_ptr(),
            args[2].as_ptr(),
            args[3].as_ptr(),
            args[4].as_ptr(),
            args[5].as_ptr(),
            r.unlock.into(),
            collect,
            (&mut values as *mut HashMap<String, String>).cast(),
        )
    };
    if result != 0 {
        return Err(AgentError::Failed("Cannot access the login keyring".into()));
    }
    Ok(values)
}
