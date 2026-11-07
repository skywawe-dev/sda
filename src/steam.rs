//! Everything that talks to Steam: Steam Guard codes, confirmations, tokens and time.
//! The wire protocol follows SteamAuth, the library the original SDA was built on.

use crate::i18n::t;
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::Value;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type HmacSha1 = Hmac<sha1::Sha1>;

const CODE_CHARS: &[u8] = b"23456789BCDFGHJKMNPQRTVWXY";
const COMMUNITY: &str = "https://steamcommunity.com";
const API: &str = "https://api.steampowered.com";
const USER_AGENT: &str = "Dalvik/2.1.0 (Linux; U; Android 9; Valve Steam App Version/3)";

pub fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn hmac_sha1(key_b64: &str, data: &[u8]) -> Option<Vec<u8>> {
    let key = STANDARD.decode(key_b64.trim()).ok()?;
    let mut mac = HmacSha1::new_from_slice(&key).ok()?;
    mac.update(data);
    Some(mac.finalize().into_bytes().to_vec())
}

/// The five-character Steam Guard code for `time` (unix seconds, on Steam's clock).
pub fn generate_code(shared_secret: &str, time: i64) -> Option<String> {
    let mut v = totp_value(shared_secret, time)?;
    let mut code = String::with_capacity(5);
    for _ in 0..5 {
        code.push(CODE_CHARS[(v % 26) as usize] as char);
        v /= 26;
    }
    Some(code)
}

/// HMAC-SHA1 over the 30-second window number, then RFC 6238 dynamic truncation.
fn totp_value(secret_b64: &str, time: i64) -> Option<u32> {
    let h = hmac_sha1(secret_b64, &((time / 30) as u64).to_be_bytes())?;
    let off = (h[19] & 0x0f) as usize;
    Some(u32::from_be_bytes([h[off], h[off + 1], h[off + 2], h[off + 3]]) & 0x7fff_ffff)
}

/// Signs a confirmations request: base64 of HMAC-SHA1(identity_secret, time_be || tag).
fn confirmation_key(identity_secret: &str, time: i64, tag: &str) -> Option<String> {
    let mut data = (time as u64).to_be_bytes().to_vec();
    data.extend(tag.as_bytes().iter().take(32 - 8));
    Some(STANDARD.encode(hmac_sha1(identity_secret, &data)?))
}

/// A JWT's `exp` claim, when it can be read.
pub fn jwt_exp(token: &str) -> Option<i64> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice::<Value>(&bytes).ok()?.get("exp")?.as_i64()
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| crate::tf!("HTTP client: {e}", "HTTP-клиент: {e}"))
}

fn as_i64(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_str()?.parse().ok())
}

fn between(text: &str, start: &str, end: &str) -> String {
    text.split_once(start).and_then(|(_, rest)| rest.split_once(end)).map(|(v, _)| v.trim().to_string()).unwrap_or_default()
}

/// Public profile data without a Web API key: (persona name, avatar URL).
/// The community XML endpoint (`?xml=1`) always exposes both.
pub fn fetch_profile(steamid: u64) -> Result<(String, String), String> {
    let url = format!("{COMMUNITY}/profiles/{steamid}/?xml=1");
    let text = client()?.get(url).send().map_err(|e| e.to_string())?.text().map_err(|e| e.to_string())?;
    let persona = between(&text, "<steamID><![CDATA[", "]]></steamID>");
    let avatar = between(&text, "<avatarFull><![CDATA[", "]]></avatarFull>");
    if persona.is_empty() && avatar.is_empty() {
        return Err(t("Profile is not available", "Профиль недоступен").into());
    }
    Ok((persona, avatar))
}

/// Downloads the avatar image (JPEG).
pub fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    let r = client()?.get(url).send().map_err(|e| e.to_string())?;
    Ok(r.bytes().map_err(|e| e.to_string())?.to_vec())
}

fn as_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Steam's clock minus the local clock, in seconds.
pub fn query_time_offset() -> Result<i64, String> {
    let resp: Value = client()?
        .post(format!("{API}/ITwoFactorService/QueryTime/v0001"))
        .form(&[("steamid", "0")])
        .send()
        .and_then(|r| r.json())
        .map_err(|e| crate::tf!("Time sync: {e}", "Синхронизация времени: {e}"))?;
    let server = resp
        .pointer("/response/server_time")
        .and_then(as_i64)
        .ok_or_else(|| t("Time sync: unexpected reply from Steam", "Синхронизация времени: неожиданный ответ Steam"))?;
    Ok(server - unix_now())
}

/// What one account needs in order to work with confirmations.
#[derive(Clone)]
pub struct Auth {
    pub steamid: u64,
    pub device_id: String,
    pub identity_secret: String,
    pub access_token: String,
    pub refresh_token: String,
    pub session_id: String,
}

/// Refreshed tokens, when a refresh happened. The caller must persist them to the maFile.
pub type NewTokens = Option<(String, Option<String>)>;

impl Auth {
    fn cookie(&self) -> String {
        format!(
            "steamLoginSecure={}%7C%7C{}; sessionid={}; mobileClient=android; mobileClientVersion=777777 3.6.4",
            self.steamid, self.access_token, self.session_id
        )
    }

    /// Renews the access token if it has expired, or will within the next minute.
    pub fn ensure_fresh(&mut self) -> Result<NewTokens, String> {
        let expired = jwt_exp(&self.access_token).is_none_or(|exp| exp < unix_now() + 60);
        if !expired {
            return Ok(None);
        }
        if self.refresh_token.is_empty() {
            return Err(t(
                "The maFile has no session data — confirmations need you to sign in",
                "В maFile нет данных сессии — для подтверждений нужен вход в аккаунт",
            )
            .into());
        }
        if jwt_exp(&self.refresh_token).is_some_and(|e| e < unix_now()) {
            return Err(t("Session expired — sign in again", "Сессия истекла: нужен повторный вход в аккаунт").into());
        }
        let resp: Value = client()?
            .post(format!("{API}/IAuthenticationService/GenerateAccessTokenForApp/v1/"))
            .form(&[("refresh_token", self.refresh_token.as_str()), ("steamid", &self.steamid.to_string()), ("renewal_type", "0")])
            .send()
            .and_then(|r| r.json())
            .map_err(|e| crate::tf!("Session refresh: {e}", "Обновление сессии: {e}"))?;
        let access = resp
            .pointer("/response/access_token")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| t("Steam issued no new token — sign in again", "Steam не выдал новый токен: нужен повторный вход"))?
            .to_string();
        let refresh = resp.pointer("/response/refresh_token").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
        self.access_token = access.clone();
        if let Some(r) = &refresh {
            self.refresh_token = r.clone();
        }
        Ok(Some((access, refresh)))
    }

    fn params(&self, time: i64, tag: &str) -> Result<Vec<(&'static str, String)>, String> {
        if self.device_id.is_empty() {
            return Err(t("The maFile has no device_id", "В maFile нет device_id").into());
        }
        let k =
            confirmation_key(&self.identity_secret, time, tag).ok_or_else(|| t("Malformed identity_secret", "Неверный identity_secret"))?;
        Ok(vec![
            ("p", self.device_id.clone()),
            ("a", self.steamid.to_string()),
            ("k", k),
            ("t", time.to_string()),
            ("m", "react".into()),
            ("tag", tag.into()),
        ])
    }
}

#[derive(Clone, Debug)]
pub struct Confirmation {
    pub id: String,
    pub nonce: String,
    pub kind: i64,
    pub type_name: String,
    pub headline: String,
    pub summary: Vec<String>,
    pub created: i64,
}

impl Confirmation {
    pub fn kind_label(&self) -> &str {
        match self.kind {
            1 => t("Test", "Тест"),
            2 => t("Trade", "Обмен"),
            3 => t("Market listing", "Торговая площадка"),
            4 => t("Feature opt-out", "Отказ от функции"),
            5 => t("Phone number change", "Смена телефона"),
            6 => t("Account recovery", "Восстановление"),
            _ if !self.type_name.is_empty() => &self.type_name,
            _ => t("Confirmation", "Подтверждение"),
        }
    }
}

/// Fetches the pending confirmations.
pub fn fetch_confirmations(auth: &mut Auth, time_offset: i64) -> (Result<Vec<Confirmation>, String>, NewTokens) {
    let tokens = match auth.ensure_fresh() {
        Ok(t) => t,
        Err(e) => return (Err(e), None),
    };
    let result = (|| {
        let params = auth.params(unix_now() + time_offset, "conf")?;
        let resp: Value = client()?
            .get(format!("{COMMUNITY}/mobileconf/getlist"))
            .query(&params)
            .header("Cookie", auth.cookie())
            .send()
            .and_then(|r| r.json())
            .map_err(|e| crate::tf!("Confirmations: {e}", "Подтверждения: {e}"))?;
        if resp.get("needauth").and_then(Value::as_bool) == Some(true) {
            return Err(t(
                "Steam wants a new sign-in — the session is no longer valid",
                "Steam требует повторный вход (сессия недействительна)",
            )
            .to_string());
        }
        if resp.get("success").and_then(Value::as_bool) != Some(true) {
            let msg = resp.get("message").map(as_string).unwrap_or_default();
            return Err(if msg.is_empty() { t("Steam returned an error", "Steam вернул ошибку").into() } else { msg });
        }
        let list = resp.get("conf").and_then(Value::as_array).cloned().unwrap_or_default();
        Ok(list
            .iter()
            .map(|c| Confirmation {
                id: c.get("id").map(as_string).unwrap_or_default(),
                nonce: c.get("nonce").map(as_string).unwrap_or_default(),
                kind: c.get("type").and_then(as_i64).unwrap_or(0),
                type_name: c.get("type_name").map(as_string).unwrap_or_default(),
                headline: c.get("headline").map(as_string).unwrap_or_default(),
                summary: c
                    .get("summary")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().map(as_string).filter(|s| !s.is_empty()).collect())
                    .unwrap_or_default(),
                created: c.get("creation_time").and_then(as_i64).unwrap_or(0),
            })
            .collect())
    })();
    (result, tokens)
}

/// Accepts (`accept = true`) or rejects the given confirmations.
pub fn respond(auth: &mut Auth, time_offset: i64, items: &[(String, String)], accept: bool) -> (Result<(), String>, NewTokens) {
    let tokens = match auth.ensure_fresh() {
        Ok(t) => t,
        Err(e) => return (Err(e), None),
    };
    let result = (|| {
        let tag = if accept { "accept" } else { "reject" };
        let mut form: Vec<(String, String)> = vec![("op".into(), if accept { "allow" } else { "cancel" }.into())];
        for (k, v) in auth.params(unix_now() + time_offset, tag)? {
            form.push((k.into(), v));
        }
        for (id, nonce) in items {
            form.push(("cid[]".into(), id.clone()));
            form.push(("ck[]".into(), nonce.clone()));
        }
        let resp: Value = client()?
            .post(format!("{COMMUNITY}/mobileconf/multiajaxop"))
            .header("Cookie", auth.cookie())
            .header("Origin", COMMUNITY)
            .form(&form)
            .send()
            .and_then(|r| r.json())
            .map_err(|e| crate::tf!("Confirmation reply: {e}", "Ответ на подтверждение: {e}"))?;
        if resp.get("success").and_then(Value::as_bool) == Some(true) {
            Ok(())
        } else {
            Err(t(
                "Steam refused the action — the confirmation may no longer be pending",
                "Steam не принял действие (возможно, подтверждение уже неактуально)",
            )
            .into())
        }
    })();
    (result, tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totp_matches_rfc6238() {
        // The official RFC 6238 vectors (SHA1, 8 digits). Steam uses the same HMAC and truncation.
        let secret = STANDARD.encode(b"12345678901234567890");
        for (t, expected) in [(59, 94287082), (1111111109, 7081804), (1234567890, 89005924), (2000000000, 69279037)] {
            assert_eq!(totp_value(&secret, t).unwrap() % 100_000_000, expected, "t={t}");
        }
    }

    #[test]
    fn code_is_stable() {
        let secret = STANDARD.encode(b"12345678901234567890");
        let a = generate_code(&secret, 1_699_999_980).unwrap(); // start of a window
        assert_eq!(a, generate_code(&secret, 1_700_000_009).unwrap(), "code is stable within a window");
        assert_ne!(a, generate_code(&secret, 1_700_000_010).unwrap(), "next window gives a different code");
        assert_eq!(a.len(), 5);
        assert!(a.bytes().all(|c| CODE_CHARS.contains(&c)));
    }

    #[test]
    fn jwt_exp_parses() {
        let payload = URL_SAFE_NO_PAD.encode(br#"{"sub":"1","exp":1800000000}"#);
        assert_eq!(jwt_exp(&format!("eyJhbGciOiJFZERTQSJ9.{payload}.sig")), Some(1_800_000_000));
        assert_eq!(jwt_exp("garbage"), None);
    }
}

#[cfg(test)]
mod live {
    /// Hits the live Steam API: `cargo test -- --ignored`
    #[test]
    #[ignore]
    fn query_time_works() {
        let off = super::query_time_offset().expect("Steam QueryTime");
        println!("offset = {off}");
        assert!(off.abs() < 3600);
    }
}
