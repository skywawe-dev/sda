//! Steam Web API services (protobuf over HTTP): sign-in, adding and removing an
//! authenticator. The protocol mirrors the official mobile app; steamguard-cli and
//! SteamKit are the usual references for it.

use crate::i18n::t;
use crate::proto::{Enc, Msg};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE};
use num_bigint::BigUint;
use std::time::Duration;

const API: &str = "https://api.steampowered.com";

/// A service error: the EResult code plus Steam's own message when it sends one.
#[derive(Debug, Clone)]
pub struct ApiError {
    pub eresult: i32,
    pub message: String,
}

impl ApiError {
    fn net(e: impl std::fmt::Display) -> Self {
        ApiError { eresult: 0, message: crate::tf!("Network: {e}", "Сеть: {e}") }
    }

    /// The error as something worth showing a person.
    pub fn human(&self) -> String {
        let known = match self.eresult {
            5 => t("Wrong login or password", "Неверный логин или пароль"),
            29 => t("This account already has an authenticator", "К аккаунту уже привязан аутентификатор"),
            63 | 65 | 88 => t("Wrong Steam Guard code", "Неверный код Steam Guard"),
            84 => t("Too many attempts — wait a while and try again", "Слишком много попыток — подождите и попробуйте позже"),
            89 => t("Wrong confirmation code from the SMS or email", "Неверный код подтверждения из SMS/письма"),
            2 if self.message.is_empty() => t("Steam rejected the request", "Steam отклонил запрос"),
            _ => "",
        };
        match (known.is_empty(), self.message.is_empty()) {
            (false, _) => known.to_string(),
            (true, false) => crate::tf!("{} (code {})", "{} (код {})", self.message, self.eresult),
            (true, true) => crate::tf!("Steam error, code {}", "Ошибка Steam, код {}", self.eresult),
        }
    }
}

type R<T> = Result<T, ApiError>;

fn client() -> R<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder().user_agent("okhttp/4.9.2").timeout(Duration::from_secs(20)).build().map_err(ApiError::net)
}

/// Calls one service method, e.g. `IAuthenticationService/BeginAuthSessionViaCredentials`.
fn call(iface: &str, method: &str, get: bool, body: Vec<u8>, access_token: Option<&str>) -> R<Msg> {
    let url = format!("{API}/{iface}/{method}/v1");
    let c = client()?;
    let req = if get {
        let mut q = vec![("input_protobuf_encoded", URL_SAFE.encode(&body))];
        if let Some(t) = access_token {
            q.push(("access_token", t.to_string()));
        }
        c.get(url).query(&q)
    } else {
        let mut r = c.post(url).form(&[("input_protobuf_encoded", STANDARD.encode(&body))]);
        if let Some(t) = access_token {
            r = r.query(&[("access_token", t)]);
        }
        r
    };
    let resp = req.send().map_err(ApiError::net)?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        let message = t("Session is no longer valid — sign in again", "Сессия недействительна — войдите заново");
        return Err(ApiError { eresult: 0, message: message.into() });
    }
    let eresult = resp.headers().get("x-eresult").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
    let message = resp.headers().get("x-error_message").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let bytes = resp.bytes().map_err(ApiError::net)?;
    if eresult != 1 {
        return Err(ApiError { eresult, message });
    }
    Msg::parse(&bytes).map_err(|e| ApiError { eresult: 0, message: e })
}

// ---------------- RSA (PKCS#1 v1.5) for the password ----------------

fn rsa_encrypt(mod_hex: &str, exp_hex: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    let bad_key = || t("Steam sent a malformed RSA key", "Steam прислал неверный RSA-ключ");
    let n = BigUint::parse_bytes(mod_hex.as_bytes(), 16).ok_or_else(bad_key)?;
    let e = BigUint::parse_bytes(exp_hex.as_bytes(), 16).ok_or_else(bad_key)?;
    let k = n.to_bytes_be().len();
    if data.len() + 11 > k {
        return Err(t("Password is too long", "Пароль слишком длинный").into());
    }
    // 00 02 [non-zero random padding] 00 [data]
    let mut block = vec![0u8, 2u8];
    let mut ps = vec![0u8; k - 3 - data.len()];
    getrandom::fill(&mut ps).map_err(|e| e.to_string())?;
    for b in &mut ps {
        while *b == 0 {
            let mut one = [0u8; 1];
            getrandom::fill(&mut one).map_err(|e| e.to_string())?;
            *b = one[0];
        }
    }
    block.extend(ps);
    block.push(0);
    block.extend_from_slice(data);
    let c = BigUint::from_bytes_be(&block).modpow(&e, &n).to_bytes_be();
    let mut out = vec![0u8; k - c.len()];
    out.extend(c);
    Ok(out)
}

// ---------------- Sign-in ----------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Guard {
    None,
    EmailCode,
    DeviceCode,
    DeviceConfirmation,
    EmailConfirmation,
    Other(i64),
}

impl Guard {
    fn from(v: i64) -> Guard {
        match v {
            1 => Guard::None,
            2 => Guard::EmailCode,
            3 => Guard::DeviceCode,
            4 => Guard::DeviceConfirmation,
            5 => Guard::EmailConfirmation,
            o => Guard::Other(o),
        }
    }
    fn code(self) -> i64 {
        match self {
            Guard::None => 1,
            Guard::EmailCode => 2,
            Guard::DeviceCode => 3,
            Guard::DeviceConfirmation => 4,
            Guard::EmailConfirmation => 5,
            Guard::Other(o) => o,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AuthSession {
    pub client_id: u64,
    pub request_id: Vec<u8>,
    pub steamid: u64,
    pub interval: f32,
    /// (confirmation method, hint — the email domain, for instance)
    pub guards: Vec<(Guard, String)>,
}

#[derive(Clone, Debug)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    #[allow(dead_code)]
    pub account_name: String,
}

/// Step 1: login and password in, auth session and the available confirmation methods out.
pub fn begin_login(account: &str, password: &str) -> R<AuthSession> {
    let key = call("IAuthenticationService", "GetPasswordRSAPublicKey", true, Enc::new().str(1, account).finish(), None)?;
    let encrypted = rsa_encrypt(&key.str(1), &key.str(2), password.as_bytes()).map_err(|e| ApiError { eresult: 0, message: e })?;
    let device = Enc::new()
        .str(1, "Galaxy S22 (SDA)")
        .int(2, 3) // MobileApp
        .int(3, -500) // Android
        .uint(4, 528);
    let body = Enc::new()
        .str(1, "Galaxy S22 (SDA)")
        .str(2, account)
        .str(3, &STANDARD.encode(encrypted))
        .uint(4, key.u64(3))
        .bool(5, true)
        .int(6, 3) // MobileApp
        .int(7, 1) // Persistent
        .str(8, "Mobile")
        .msg(9, device)
        .finish();
    let r = call("IAuthenticationService", "BeginAuthSessionViaCredentials", false, body, None)?;
    Ok(AuthSession {
        client_id: r.u64(1),
        request_id: r.bytes(2),
        steamid: r.u64(5),
        interval: r.f32(3).max(1.0),
        guards: r.msgs(4).iter().map(|m| (Guard::from(m.i64(1)), m.str(2))).collect(),
    })
}

/// Submits a code, either from the email or from the authenticator.
pub fn submit_code(s: &AuthSession, code: &str, kind: Guard) -> R<()> {
    let body = Enc::new().uint(1, s.client_id).fixed64(2, s.steamid).str(3, code.trim()).int(4, kind.code()).finish();
    match call("IAuthenticationService", "UpdateAuthSessionWithSteamGuardCode", false, body, None) {
        Ok(_) => Ok(()),
        // 29 means the code was already accepted, e.g. when it gets submitted twice.
        Err(e) if e.eresult == 29 => Ok(()),
        Err(e) => Err(e),
    }
}

/// Polls whether the sign-in finished. `None` means Steam is still waiting for confirmation.
pub fn poll(s: &AuthSession) -> R<Option<Tokens>> {
    let body = Enc::new().uint(1, s.client_id).bytes(2, &s.request_id).finish();
    let r = call("IAuthenticationService", "PollAuthSessionStatus", false, body, None)?;
    let refresh = r.str(3);
    if refresh.is_empty() {
        return Ok(None);
    }
    Ok(Some(Tokens { access_token: r.str(4), refresh_token: refresh, account_name: r.str(6) }))
}

// ---------------- Authenticator ----------------

/// A freshly created authenticator; it is not active until `finalize` succeeds.
#[derive(Clone, Debug)]
pub struct NewAuthenticator {
    pub shared_secret: String,
    pub identity_secret: String,
    pub secret_1: String,
    pub serial_number: u64,
    pub revocation_code: String,
    pub uri: String,
    pub server_time: i64,
    pub account_name: String,
    pub token_gid: String,
    pub status: i64,
    pub phone_hint: String,
    /// 1 sends the code by SMS, 3 by email.
    pub confirm_type: i64,
}

pub fn add_authenticator(access_token: &str, steamid: u64, device_id: &str) -> R<NewAuthenticator> {
    let body = Enc::new().fixed64(1, steamid).uint(4, 1).str(5, device_id).str(6, "1").uint(8, 2).finish();
    let r = call("ITwoFactorService", "AddAuthenticator", false, body, Some(access_token))?;
    Ok(NewAuthenticator {
        shared_secret: STANDARD.encode(r.bytes(1)),
        serial_number: r.u64(2),
        revocation_code: r.str(3),
        uri: r.str(4),
        server_time: r.i64(5),
        account_name: r.str(6),
        token_gid: r.str(7),
        identity_secret: STANDARD.encode(r.bytes(8)),
        secret_1: STANDARD.encode(r.bytes(9)),
        status: r.i64(10),
        phone_hint: r.str(11),
        confirm_type: r.i64(12),
    })
}

/// One activation step. `Ok(None)` means done; `Ok(Some(server_time))` means Steam asked
/// for another code (`want_more`) and the time it wants it generated for.
pub fn finalize(access_token: &str, steamid: u64, code: &str, time: i64, activation_code: &str) -> R<Option<i64>> {
    let body = Enc::new().fixed64(1, steamid).str(2, code).uint(3, time as u64).str(4, activation_code.trim()).bool(6, true).finish();
    let r = call("ITwoFactorService", "FinalizeAddAuthenticator", false, body, Some(access_token))?;
    if r.bool(2) {
        return Ok(Some(r.i64(3)));
    }
    if r.bool(1) {
        return Ok(None);
    }
    let status = r.i64(4) as i32;
    Err(ApiError { eresult: if status > 1 { status } else { 2 }, message: String::new() })
}

/// Removes the authenticator from the account. On failure, reports the attempts left.
pub fn remove_authenticator(access_token: &str, revocation_code: &str) -> R<Result<(), u32>> {
    let body = Enc::new()
        .str(2, revocation_code.trim())
        .uint(5, 1)
        .uint(6, 1) // fall back to email codes
        .finish();
    let r = call("ITwoFactorService", "RemoveAuthenticator", false, body, Some(access_token))?;
    Ok(if r.bool(1) { Ok(()) } else { Err(r.u64(5) as u32) })
}

pub fn new_device_id() -> String {
    let mut b = [0u8; 16];
    let _ = getrandom::fill(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40; // UUID v4
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("android:{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rsa_pkcs1_decrypts_back() {
        // A 512-bit key: the textbook p=61, q=53 is far too short to hold a padded block.
        let p = BigUint::parse_bytes(b"F7E75FDC469067FFDC4E847C51F452DF", 16).unwrap();
        let q = BigUint::parse_bytes(b"E85CED54AF57E53E092113E62F436F4F", 16).unwrap();
        let n = &p * &q;
        let e = BigUint::from(65537u32);
        let phi = (&p - 1u32) * (&q - 1u32);
        // Decrypting with d and checking the padding proves the block was built correctly.
        let d = mod_inverse(&e, &phi);
        let ct = rsa_encrypt(&format!("{n:X}"), "10001", b"secret").unwrap();
        let m = BigUint::from_bytes_be(&ct).modpow(&d, &n).to_bytes_be();
        assert_eq!(m[0], 2, "02 follows the leading 00, which BigUint drops");
        assert!(m.ends_with(b"\0secret"));
    }

    fn mod_inverse(a: &BigUint, m: &BigUint) -> BigUint {
        use num_bigint::BigInt;
        let (mut old_r, mut r) = (BigInt::from(a.clone()), BigInt::from(m.clone()));
        let (mut old_s, mut s) = (BigInt::from(1), BigInt::from(0));
        while r != BigInt::from(0) {
            let q = &old_r / &r;
            (old_r, r) = (r.clone(), &old_r - &q * &r);
            (old_s, s) = (s.clone(), &old_s - &q * &s);
        }
        let m = BigInt::from(m.clone());
        (((old_s % &m) + &m) % &m).to_biguint().unwrap()
    }

    #[test]
    fn device_id_format() {
        let d = new_device_id();
        assert!(d.starts_with("android:") && d.len() == 8 + 36, "{d}");
    }
}
