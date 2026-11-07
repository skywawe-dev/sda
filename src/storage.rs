//! The `maFiles` directory in the original SDA's format: manifest.json plus one
//! <steamid>.maFile per account, optionally encrypted with PBKDF2-HMAC-SHA1
//! (50 000 rounds) into AES-256-CBC, PKCS7-padded and base64-encoded.

use aes::cipher::{BlockModeDecrypt, BlockModeEncrypt, KeyIvInit, block_padding::Pkcs7};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const PBKDF2_ITERATIONS: u32 = 50_000;
const SALT_LEN: usize = 8;
const IV_LEN: usize = 16;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Entry {
    #[serde(default)]
    pub encryption_iv: Option<String>,
    #[serde(default)]
    pub encryption_salt: Option<String>,
    pub filename: String,
    #[serde(default)]
    pub steamid: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Manifest {
    #[serde(default)]
    pub encrypted: bool,
    #[serde(default)]
    pub entries: Vec<Entry>,
    /// Any other setting the original SDA wrote is kept verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// An account is the whole maFile. Keeping the raw JSON means fields this app does
/// not understand survive a round-trip instead of being silently dropped.
#[derive(Clone, Debug)]
pub struct Account {
    pub filename: String,
    pub steamid: u64,
    pub raw: Value,
}

impl Account {
    pub fn field(&self, name: &str) -> &str {
        self.raw.get(name).and_then(Value::as_str).unwrap_or("")
    }
    pub fn session(&self, name: &str) -> &str {
        self.raw.get("Session").and_then(|s| s.get(name)).and_then(Value::as_str).unwrap_or("")
    }
    pub fn name(&self) -> &str {
        let n = self.field("account_name");
        if n.is_empty() { crate::i18n::t("unnamed", "без имени") } else { n }
    }
    pub fn set_tokens(&mut self, access: &str, refresh: Option<&str>) {
        if !self.raw.get("Session").is_some_and(Value::is_object) {
            self.raw["Session"] = Value::Object(Map::new());
        }
        let s = &mut self.raw["Session"];
        s["AccessToken"] = Value::String(access.into());
        if let Some(r) = refresh {
            s["RefreshToken"] = Value::String(r.into());
        }
        if s.get("SteamID").is_none() {
            s["SteamID"] = Value::from(self.steamid);
        }
    }

    fn from_json(text: &str, filename: &str) -> Result<Account, String> {
        let raw: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|e| crate::tf!("{filename}: not a maFile ({e})", "{filename}: это не maFile ({e})"))?;
        if raw.get("shared_secret").and_then(Value::as_str).unwrap_or("").is_empty() {
            return Err(crate::tf!("{filename}: no shared_secret in the file", "{filename}: в файле нет shared_secret"));
        }
        let steamid =
            raw.get("Session").and_then(|s| s.get("SteamID")).and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok())).unwrap_or(0);
        Ok(Account { filename: filename.to_string(), steamid, raw })
    }
}

fn random_b64(len: usize) -> String {
    let mut buf = vec![0u8; len];
    getrandom::fill(&mut buf).expect("the OS refused to provide random bytes");
    STANDARD.encode(buf)
}

fn derive_key(passkey: &str, salt_b64: &str) -> Option<[u8; 32]> {
    let salt = STANDARD.decode(salt_b64).ok()?;
    let mut key = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(passkey.as_bytes(), &salt, PBKDF2_ITERATIONS, &mut key);
    Some(key)
}

fn encrypt(passkey: &str, salt: &str, iv: &str, plaintext: &str) -> Option<String> {
    let key = derive_key(passkey, salt)?;
    let iv: [u8; IV_LEN] = STANDARD.decode(iv).ok()?.try_into().ok()?;
    let ct = cbc::Encryptor::<aes::Aes256>::new(&key.into(), &iv.into()).encrypt_padded_vec::<Pkcs7>(plaintext.as_bytes());
    Some(STANDARD.encode(ct))
}

fn decrypt(passkey: &str, salt: &str, iv: &str, data_b64: &str) -> Option<String> {
    let key = derive_key(passkey, salt)?;
    let iv: [u8; IV_LEN] = STANDARD.decode(iv).ok()?.try_into().ok()?;
    let data = STANDARD.decode(data_b64.trim()).ok()?;
    let pt = cbc::Decryptor::<aes::Aes256>::new(&key.into(), &iv.into()).decrypt_padded_vec::<Pkcs7>(&data).ok()?;
    String::from_utf8(pt).ok()
}

/// Per-account data this app adds. The original SDA has no place for it, so it lives
/// in a separate sda_meta.json next to the maFiles.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Meta {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub alias: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// Manual position in the list. 0 means unset, and the account sorts by name.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub order: i32,
    /// Cached profile data, filled in only when the profile lookup setting is on.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub persona: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub avatar: String,
}

fn is_zero(v: &i32) -> bool {
    *v == 0
}

pub type MetaMap = BTreeMap<u64, Meta>;

/// sda_meta.json is either `{"data":{...}}` or, when encrypted,
/// `{"salt","iv","data":"<base64>"}`.
#[derive(Serialize, Deserialize, Default)]
struct MetaFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    salt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    iv: Option<String>,
    #[serde(default)]
    data: Value,
}

pub struct Store {
    pub dir: PathBuf,
    pub manifest: Manifest,
    passkey: Option<String>,
}

impl Store {
    /// Opens the maFiles directory, creating it when missing. With no manifest.json
    /// but unencrypted *.maFile lying around, those get adopted into a new manifest.
    pub fn open(dir: PathBuf) -> Result<Store, String> {
        fs::create_dir_all(&dir).map_err(|e| crate::tf!("Could not create {}: {e}", "Не удалось создать {}: {e}", dir.display()))?;
        let path = dir.join("manifest.json");
        let manifest = if path.exists() {
            let text = fs::read_to_string(&path).map_err(|e| format!("manifest.json: {e}"))?;
            serde_json::from_str(text.trim_start_matches('\u{feff}'))
                .map_err(|e| crate::tf!("manifest.json is corrupt: {e}", "manifest.json повреждён: {e}"))?
        } else {
            let mut m = Manifest::default();
            m.extra.insert("first_run".into(), Value::Bool(false));
            for e in fs::read_dir(&dir).into_iter().flatten().flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.to_lowercase().ends_with(".mafile")
                    && let Ok(acc) = fs::read_to_string(e.path()).map_err(|e| e.to_string()).and_then(|t| Account::from_json(&t, &name))
                {
                    m.entries.push(Entry { encryption_iv: None, encryption_salt: None, filename: name, steamid: acc.steamid });
                }
            }
            m
        };
        let store = Store { dir, manifest, passkey: None };
        store.save_manifest()?;
        Ok(store)
    }

    pub fn needs_passkey(&self) -> bool {
        self.manifest.encrypted && !self.manifest.entries.is_empty() && self.passkey.is_none()
    }

    pub fn encrypted(&self) -> bool {
        self.manifest.encrypted
    }

    fn save_manifest(&self) -> Result<(), String> {
        let text = serde_json::to_string_pretty(&self.manifest).map_err(|e| e.to_string())?;
        write_atomic(&self.dir.join("manifest.json"), &text)
    }

    fn read_entry(&self, entry: &Entry, passkey: Option<&str>) -> Result<Account, String> {
        let text = fs::read_to_string(self.dir.join(&entry.filename)).map_err(|e| format!("{}: {e}", entry.filename))?;
        let json = if self.manifest.encrypted {
            let (Some(salt), Some(iv), Some(pass)) = (&entry.encryption_salt, &entry.encryption_iv, passkey) else {
                return Err(crate::tf!("{}: no encryption data", "{}: нет данных шифрования", entry.filename));
            };
            decrypt(pass, salt, iv, &text).ok_or_else(|| crate::i18n::t("Wrong password", "Неверный пароль").to_string())?
        } else {
            text
        };
        let mut acc = Account::from_json(&json, &entry.filename)?;
        if acc.steamid == 0 {
            acc.steamid = entry.steamid;
        }
        Ok(acc)
    }

    /// Verifies the password against the first entry and remembers it on success.
    pub fn unlock(&mut self, passkey: &str) -> Result<(), String> {
        if let Some(first) = self.manifest.entries.first() {
            self.read_entry(first, Some(passkey))?;
        }
        self.passkey = Some(passkey.to_string());
        Ok(())
    }

    /// Loads every account. Files that fail to load are reported instead of aborting.
    pub fn load_all(&self) -> (Vec<Account>, Vec<String>) {
        let mut accs = Vec::new();
        let mut errs = Vec::new();
        for e in &self.manifest.entries {
            match self.read_entry(e, self.passkey.as_deref()) {
                Ok(a) => accs.push(a),
                Err(err) => errs.push(err),
            }
        }
        (accs, errs)
    }

    fn write_account(&self, acc: &Account, entry: &mut Entry) -> Result<(), String> {
        let json = serde_json::to_string(&acc.raw).map_err(|e| e.to_string())?;
        let text = if self.manifest.encrypted {
            let pass = self.passkey.as_deref().ok_or_else(|| {
                crate::i18n::t("Vault is encrypted but no password was entered", "Хранилище зашифровано, но пароль не введён")
            })?;
            let salt = random_b64(SALT_LEN);
            let iv = random_b64(IV_LEN);
            let ct = encrypt(pass, &salt, &iv, &json).ok_or_else(|| crate::i18n::t("Encryption failed", "Ошибка шифрования"))?;
            entry.encryption_salt = Some(salt);
            entry.encryption_iv = Some(iv);
            ct
        } else {
            entry.encryption_salt = None;
            entry.encryption_iv = None;
            json
        };
        write_atomic(&self.dir.join(&entry.filename), &text)
    }

    /// Persists a changed account, e.g. after its tokens were refreshed.
    pub fn save_account(&mut self, acc: &Account) -> Result<(), String> {
        let idx = self
            .manifest
            .entries
            .iter()
            .position(|e| e.filename == acc.filename)
            .ok_or_else(|| crate::i18n::t("Account is not in manifest.json", "Аккаунта нет в manifest.json"))?;
        let mut entry = self.manifest.entries[idx].clone();
        self.write_account(acc, &mut entry)?;
        self.manifest.entries[idx] = entry;
        self.save_manifest()
    }

    /// Imports a plain, unencrypted maFile and returns the account it describes.
    pub fn import(&mut self, path: &Path) -> Result<Account, String> {
        let shown = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let text = fs::read_to_string(path).map_err(|e| format!("{shown}: {e}"))?;
        let acc = Account::from_json(&text, &shown)?;
        if acc.steamid == 0 {
            return Err(crate::tf!(
                "{shown}: no Session.SteamID in the file, so the account cannot be identified",
                "{shown}: в файле нет Session.SteamID — не могу определить аккаунт"
            ));
        }
        self.add(acc)
    }

    /// Stores an account straight from JSON, e.g. an authenticator that was just added.
    pub fn add_raw(&mut self, raw: Value, steamid: u64) -> Result<Account, String> {
        self.add(Account { filename: String::new(), steamid, raw })
    }

    fn add(&mut self, mut acc: Account) -> Result<Account, String> {
        acc.filename = format!("{}.maFile", acc.steamid);
        let mut entry = Entry { encryption_iv: None, encryption_salt: None, filename: acc.filename.clone(), steamid: acc.steamid };
        self.write_account(&acc, &mut entry)?;
        self.manifest.entries.retain(|e| e.steamid != acc.steamid && e.filename != entry.filename);
        self.manifest.entries.push(entry);
        self.save_manifest()?;
        Ok(acc)
    }

    /// Drops the account from SDA: the file and its manifest entry. The authenticator
    /// stays attached to the Steam account.
    pub fn remove(&mut self, acc: &Account) -> Result<(), String> {
        self.manifest.entries.retain(|e| e.filename != acc.filename);
        self.save_manifest()?;
        let _ = fs::remove_file(self.dir.join(&acc.filename));
        Ok(())
    }

    /// Retires an account after its authenticator was removed, keeping a backup copy
    /// that is no longer useful for signing in.
    pub fn archive(&mut self, acc: &Account) -> Result<(), String> {
        let from = self.dir.join(&acc.filename);
        let to = self.dir.join(format!("{}.revoked.bak", acc.filename));
        let _ = fs::rename(&from, &to);
        self.manifest.entries.retain(|e| e.filename != acc.filename);
        self.save_manifest()
    }

    // ---------- aliases and notes ----------

    pub fn load_meta(&self) -> MetaMap {
        let Ok(text) = fs::read_to_string(self.dir.join("sda_meta.json")) else { return MetaMap::new() };
        let Ok(file) = serde_json::from_str::<MetaFile>(&text) else { return MetaMap::new() };
        let data = match (&file.salt, &file.iv, &file.data) {
            (Some(salt), Some(iv), Value::String(ct)) => {
                let Some(pass) = &self.passkey else { return MetaMap::new() };
                match decrypt(pass, salt, iv, ct).and_then(|t| serde_json::from_str(&t).ok()) {
                    Some(v) => v,
                    None => return MetaMap::new(),
                }
            }
            (_, _, v) => v.clone(),
        };
        serde_json::from_value(data).unwrap_or_default()
    }

    pub fn save_meta(&self, meta: &MetaMap) -> Result<(), String> {
        let meta: MetaMap = meta.iter().filter(|(_, m)| **m != Meta::default()).map(|(k, v)| (*k, v.clone())).collect();
        let data = serde_json::to_value(&meta).map_err(|e| e.to_string())?;
        let file = if self.manifest.encrypted {
            let pass = self.passkey.as_deref().ok_or_else(|| {
                crate::i18n::t("Vault is encrypted but no password was entered", "Хранилище зашифровано, но пароль не введён")
            })?;
            let salt = random_b64(SALT_LEN);
            let iv = random_b64(IV_LEN);
            let ct =
                encrypt(pass, &salt, &iv, &data.to_string()).ok_or_else(|| crate::i18n::t("Encryption failed", "Ошибка шифрования"))?;
            MetaFile { salt: Some(salt), iv: Some(iv), data: Value::String(ct) }
        } else {
            MetaFile { salt: None, iv: None, data }
        };
        let text = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
        write_atomic(&self.dir.join("sda_meta.json"), &text)
    }

    /// Sets or changes (`Some`) or removes (`None`) encryption, rewriting every file.
    pub fn set_passkey(&mut self, accounts: &[Account], new: Option<&str>) -> Result<(), String> {
        let meta = self.load_meta();
        let old_encrypted = self.manifest.encrypted;
        let old_pass = self.passkey.clone();
        self.manifest.encrypted = new.is_some();
        self.passkey = new.map(str::to_string);
        let result = (|| {
            for acc in accounts {
                self.save_account(acc)?;
            }
            self.save_meta(&meta)?;
            self.save_manifest()
        })();
        if result.is_err() {
            self.manifest.encrypted = old_encrypted;
            self.passkey = old_pass;
        }
        result
    }
}

/// Writes to a temporary file and renames it, so a crash mid-write cannot corrupt a maFile.
fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text).map_err(|e| crate::tf!("Writing {}: {e}", "Запись {}: {e}", path.display()))?;
    fs::rename(&tmp, path).map_err(|e| crate::tf!("Writing {}: {e}", "Запись {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encryption_roundtrip() {
        let salt = random_b64(SALT_LEN);
        let iv = random_b64(IV_LEN);
        let ct = encrypt("pass", &salt, &iv, r#"{"a":1}"#).unwrap();
        assert_eq!(decrypt("pass", &salt, &iv, &ct).as_deref(), Some(r#"{"a":1}"#));
        assert_eq!(decrypt("wrong", &salt, &iv, &ct), None);
    }

    #[test]
    fn meta_order_roundtrips() {
        let dir = std::env::temp_dir().join(format!("sda_ord_{}", random_b64(6).replace(['/', '+', '='], "")));
        fs::create_dir_all(&dir).unwrap();
        let s = Store::open(dir.clone()).unwrap();
        let mut meta = MetaMap::new();
        meta.insert(10, Meta { order: 2, ..Default::default() });
        meta.insert(20, Meta { alias: "bravo".into(), order: 1, ..Default::default() });
        s.save_meta(&meta).unwrap();
        let back = s.load_meta();
        assert_eq!(back.get(&10).unwrap().order, 2);
        assert_eq!(back.get(&20).unwrap().order, 1);
        // The manual order wins over the names.
        let mut ids: Vec<u64> = back.keys().copied().collect();
        ids.sort_by_key(|id| back[id].order);
        assert_eq!(ids, vec![20, 10]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn store_import_encrypt_reload() {
        let dir = std::env::temp_dir().join(format!("sda_test_{}", random_b64(6).replace(['/', '+', '='], "")));
        let mafile = dir.join("in.maFile");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &mafile,
            r#"{"shared_secret":"AAAAAAAAAAAAAAAAAAAAAAAAAAA=","identity_secret":"AAAA","account_name":"bob","device_id":"android:x","extra_field":42,"Session":{"SteamID":76561198000000001}}"#,
        )
        .unwrap();
        let store_dir = dir.join("maFiles");
        let mut s = Store::open(store_dir.clone()).unwrap();
        let acc = s.import(&mafile).unwrap();
        assert_eq!(acc.steamid, 76561198000000001);
        s.set_passkey(std::slice::from_ref(&acc), Some("hunter2")).unwrap();

        let mut s2 = Store::open(store_dir).unwrap();
        assert!(s2.needs_passkey());
        assert!(s2.unlock("wrong").is_err());
        s2.unlock("hunter2").unwrap();
        let (accs, errs) = s2.load_all();
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(accs[0].name(), "bob");
        assert_eq!(accs[0].raw["extra_field"], 42, "unknown fields survive a round-trip");

        let mut meta = MetaMap::new();
        meta.insert(acc.steamid, Meta { alias: "main".into(), note: "a note".into(), ..Default::default() });
        s2.save_meta(&meta).unwrap();
        let raw = fs::read_to_string(s2.dir.join("sda_meta.json")).unwrap();
        assert!(!raw.contains("main"), "metadata is encrypted along with the vault");
        assert_eq!(s2.load_meta(), meta);
        s2.set_passkey(&accs, None).unwrap();
        assert_eq!(s2.load_meta(), meta, "removing the password keeps aliases intact");
        let _ = fs::remove_dir_all(&dir);
    }
}
