// webrain-core/src/vault.rs — local encrypted credential vault (AES-256-GCM) + TOTP.
//
// The secret loop: the LLM holds only a reference (service+profile). The value is
// decrypted HERE, in-process, and injected straight into the browser via CDP — it
// never passes through the model, chat, or logs.
//
// Storage (portable, no OS daemon — Windows/macOS/Linux/Docker):
//   %APPDATA%/webrain/vault.json   |  ~/.config/webrain/vault.json
//   %APPDATA%/webrain/vault.key    |  ~/.config/webrain/vault.key   (32 random bytes, 0600)
// vault.json holds an index of {service, profile, username, created_at, enc{nonce,ct}};
// the encrypted payload is the Cred struct. vault.key is the only thing that unlocks it —
// copy BOTH files to move the vault to another machine.
//
// ponytail: AES-GCM + key file beats keyring — no OS daemon means headless Linux/CI works.
// TOTP is RFC 6238 (SHA1, 6 digits, 30s) via the already-locked hmac+sha1 crates.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};

/// AES-GCM ciphertext + nonce (base64 in JSON — the only thing stored at rest).
#[derive(Serialize, Deserialize, Clone)]
pub struct Enc {
    pub nonce: String,
    pub ct: String,
}

/// Decrypted credentials for one profile — exists only inside this process.
#[derive(Serialize, Deserialize, Clone)]
pub struct Cred {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub totp: Option<String>, // base32 otpauth seed, for TOTP auto-injection
}

/// Index row — the ONLY view the LLM ever gets (no secrets).
#[derive(Serialize, Deserialize, Clone)]
pub struct Meta {
    pub service: String,
    pub profile: String,
    pub username: String,
    pub created_at: u64,
}

#[derive(Serialize, Deserialize, Clone)]
struct Entry {
    service: String,
    profile: String,
    username: String,
    created_at: u64,
    enc: Enc,
}

pub fn vault_dir() -> std::path::PathBuf {
    if let Ok(d) = std::env::var("WEBRAIN_VAULT_DIR") {
        return std::path::PathBuf::from(d);
    }
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join("webrain")
}

fn key_path() -> std::path::PathBuf {
    vault_dir().join("vault.key")
}
fn index_path() -> std::path::PathBuf {
    vault_dir().join("vault.json")
}

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Load the key, or create one on first enrollment.
/// Only `NotFound` means "no key yet" — any other read error (EACCES, I/O) is
/// propagated so an unreadable key is never silently regenerated (which would
/// lock out every entry encrypted with the existing key).
fn ensure_key() -> anyhow::Result<[u8; 32]> {
    match std::fs::read(key_path()) {
        Ok(raw) => {
            return raw
                .as_slice()
                .try_into()
                .map_err(|_| anyhow::anyhow!("vault.key must be 32 bytes"));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let mut key = [0u8; 32];
    getrandom::fill(&mut key)?;
    std::fs::create_dir_all(vault_dir())?;
    // Exclusive create: two concurrent first runs must not each write a
    // different key and clobber the other (locking out the other's entries).
    // 0600 at creation — no world-readable window before a chmod would run.
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    match opts.open(key_path()) {
        Ok(mut f) => {
            use std::io::Write;
            f.write_all(&key)?;
            f.sync_all()?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // Lost the race — another process created the key; trust theirs.
            let raw = std::fs::read(key_path())?;
            return raw
                .as_slice()
                .try_into()
                .map_err(|_| anyhow::anyhow!("vault.key must be 32 bytes"));
        }
        Err(e) => return Err(e.into()),
    }
    Ok(key)
}

fn load_key() -> anyhow::Result<[u8; 32]> {
    let raw = std::fs::read(key_path()).map_err(|_| {
        anyhow::anyhow!("vault.key not found — run `webrain vault set <service> <profile>` first")
    })?;
    raw.as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("vault.key must be 32 bytes"))
}

fn encrypt(key: &[u8; 32], plain: &str) -> anyhow::Result<Enc> {
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce)?;
    let ct = Aes256Gcm::new_from_slice(key)
        .map_err(|e| anyhow::anyhow!("cipher init: {e}"))?
        .encrypt(&Nonce::from(nonce), plain.as_bytes())
        .map_err(|e| anyhow::anyhow!("encrypt: {e}"))?;
    Ok(Enc {
        nonce: b64().encode(nonce),
        ct: b64().encode(ct),
    })
}

fn decrypt(key: &[u8; 32], enc: &Enc) -> anyhow::Result<Cred> {
    let nonce = b64().decode(&enc.nonce)?;
    let ct = b64().decode(&enc.ct)?;
    let pt = Aes256Gcm::new_from_slice(key)
        .map_err(|e| anyhow::anyhow!("cipher init: {e}"))?
        .decrypt(
            &Nonce::try_from(nonce.as_slice()).map_err(|_| anyhow::anyhow!("bad nonce length"))?,
            ct.as_ref(),
        )
        .map_err(|e| anyhow::anyhow!("decrypt failed (wrong key?): {e}"))?;
    Ok(serde_json::from_slice(&pt)?)
}

/// Serializes read-modify-write cycles (set/set_username/remove) so two
/// concurrent mutations can't both read the same snapshot and have the last
/// save_entries silently drop the other's entry. In-process only — the vault is
/// CLI/one-server driven; cross-process installs are out of scope.
static VAULT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn load_entries() -> anyhow::Result<Vec<Entry>> {
    match std::fs::read_to_string(index_path()) {
        Ok(s) if !s.trim().is_empty() => Ok(serde_json::from_str(&s)?),
        Ok(_) => Ok(Vec::new()),
        // Only a missing index means "empty vault"; an unreadable/truncated
        // index must NOT look empty, or a later set/remove would rewrite the
        // file and permanently discard previously stored entries.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

fn save_entries(entries: &[Entry]) -> anyhow::Result<()> {
    std::fs::create_dir_all(vault_dir())?;
    // Atomic replace: write a temp file then rename over vault.json so a crash
    // mid-write can't truncate/corrupt the live index (which would look empty
    // on the next load and trigger a data-losing rewrite).
    let tmp = index_path().with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(entries)?)?;
    std::fs::rename(&tmp, index_path())?;
    Ok(())
}

/// Enroll or rotate a profile. `totp` is an optional base32 otpauth seed.
pub fn set(
    service: &str,
    profile: &str,
    username: &str,
    password: &str,
    totp: Option<String>,
) -> anyhow::Result<()> {
    let _g = VAULT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let key = ensure_key()?;
    let cred = Cred {
        username: username.to_string(),
        password: password.to_string(),
        totp,
    };
    let enc = encrypt(&key, &serde_json::to_string(&cred)?)?;
    let mut entries = load_entries()?;
    entries.retain(|e| !(e.service == service && e.profile == profile)); // upsert
    entries.push(Entry {
        service: service.to_string(),
        profile: profile.to_string(),
        username: username.to_string(),
        created_at: now(),
        enc,
    });
    save_entries(&entries)
}

/// Update only the username of an existing profile. No secret re-entry needed:
/// decryption uses the vault key, not the account password.
pub fn set_username(service: &str, profile: &str, username: &str) -> anyhow::Result<()> {
    let _g = VAULT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let key = load_key()?;
    let mut entries = load_entries()?;
    let entry = entries
        .iter_mut()
        .find(|e| e.service == service && e.profile == profile)
        .ok_or_else(|| anyhow::anyhow!("no vault entry for {service}/{profile}"))?;
    let mut cred: Cred = decrypt(&key, &entry.enc)?;
    cred.username = username.to_string();
    entry.username = username.to_string();
    entry.enc = encrypt(&key, &serde_json::to_string(&cred)?)?;
    save_entries(&entries)
}

/// Names only — safe to show the LLM. Never includes secrets.
pub fn list() -> anyhow::Result<Vec<Meta>> {
    Ok(load_entries()?
        .into_iter()
        .map(|e| Meta {
            service: e.service,
            profile: e.profile,
            username: e.username,
            created_at: e.created_at,
        })
        .collect())
}

/// Resolve + decrypt a profile. In-process only (webrain_login).
pub fn get(service: &str, profile: &str) -> anyhow::Result<Cred> {
    let key = load_key()?;
    let entries = load_entries()?;
    let entry = entries
        .iter()
        .find(|e| e.service == service && e.profile == profile)
        .ok_or_else(|| anyhow::anyhow!("no vault entry for {service}/{profile}"))?;
    decrypt(&key, &entry.enc)
}

pub fn remove(service: &str, profile: &str) -> anyhow::Result<()> {
    let _g = VAULT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut entries = load_entries()?;
    let before = entries.len();
    entries.retain(|e| !(e.service == service && e.profile == profile));
    if entries.len() == before {
        anyhow::bail!("no vault entry for {service}/{profile}");
    }
    save_entries(&entries)
}

// ── TOTP (RFC 6238, SHA1, 6 digits, 30s) ─────────────────────────────────

type HmacSha1 = Hmac<sha1::Sha1>;

fn totp_at(key: &[u8], counter: u64) -> String {
    let mut mac = <HmacSha1 as KeyInit>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(&counter.to_be_bytes());
    let out = mac.finalize().into_bytes();
    let offset = (out[out.len() - 1] & 0x0f) as usize;
    let code = (u32::from(out[offset]) & 0x7f) << 24
        | u32::from(out[offset + 1]) << 16
        | u32::from(out[offset + 2]) << 8
        | u32::from(out[offset + 3]);
    format!("{:06}", code % 1_000_000)
}

/// Current TOTP code for a base32 otpauth secret.
pub fn totp_code(secret_b32: &str) -> anyhow::Result<String> {
    let key = base32_decode(secret_b32)?;
    let counter = now() / 30;
    Ok(totp_at(&key, counter))
}

fn base32_decode(s: &str) -> anyhow::Result<Vec<u8>> {
    let mut bits = 0u32;
    let mut nbits = 0u32;
    let mut out = Vec::new();
    for c in s.chars() {
        if matches!(c, ' ' | '\n' | '\r' | '-' | '=') {
            continue;
        }
        let cu = c.to_ascii_uppercase();
        let v = match cu {
            'A'..='Z' => cu as u8 - b'A',
            '2'..='7' => cu as u8 - b'2' + 26,
            _ => return Err(anyhow::anyhow!("invalid base32 char {c:?}")),
        };
        bits = (bits << 5) | v as u32;
        nbits += 5;
        if nbits >= 8 {
            nbits -= 8;
            out.push((bits >> nbits) as u8);
        }
    }
    // Leftover bits must be zero — a malformed seed with non-zero trailing bits
    // used to be silently accepted as a (wrong) TOTP key.
    if nbits > 0 && (bits & ((1u32 << nbits) - 1)) != 0 {
        anyhow::bail!("base32 seed has non-zero trailing bits");
    }
    Ok(out)
}

// ── CDP injection helpers (selector-based; reused by webrain_login) ──────

fn jstr(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

const FILL_JS: &str = r#"(() => {
  const el = document.querySelector(SEL);
  if (!el) return {ok:false, reason:'no-sel'};
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  const set = Object.getOwnPropertyDescriptor(proto, 'value').set;
  set.call(el, VAL);
  el.dispatchEvent(new Event('input', {bubbles:true}));
  el.dispatchEvent(new Event('change', {bubbles:true}));
  return {ok:true};
})()"#;

const CLICK_JS: &str = r#"(() => { const el = document.querySelector(SEL); if(!el) return {ok:false}; el.click(); return {ok:true}; })()"#;

const OTP_JS: &str = r#"(() => {
  const el = (SEL && document.querySelector(SEL)) ||
    document.querySelector('input[autocomplete="one-time-code"]') ||
    document.querySelector('input[inputmode="numeric"]');
  if (!el || el.offsetParent === null) return {found:false};
  return {found:true};
})()"#;

pub fn fill_js(sel: &str, val: &str) -> String {
    // Two-phase sentinel replacement: the selector value could contain the
    // literal "VAL" (or the value contain "SEL"), corrupting the other's JSON
    // if replaced sequentially. Swap in control-char sentinels first.
    FILL_JS
        .replace("SEL", "\u{1}")
        .replace("VAL", "\u{2}")
        .replace("\u{1}", &jstr(sel))
        .replace("\u{2}", &jstr(val))
}

pub fn click_js(sel: &str) -> String {
    CLICK_JS.replace("SEL", &jstr(sel))
}

pub fn otp_detect_js(sel: &str) -> String {
    let s = if sel.is_empty() {
        "null".to_string()
    } else {
        jstr(sel)
    };
    OTP_JS.replace("SEL", &s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totp_rfc6238_vector() {
        // RFC 6238 Appendix B — SHA1, T=59 → counter 1, secret "12345678901234567890"
        assert_eq!(totp_at(b"12345678901234567890", 1), "287082");
    }

    #[test]
    fn base32_decode_known() {
        // "Hello!\xde\xad\xbe\xef" is the canonical JBSWY3DPEHPK3PXP example
        assert_eq!(
            base32_decode("JBSWY3DPEHPK3PXP").unwrap(),
            b"Hello!\xde\xad\xbe\xef"
        );
    }

    #[test]
    fn base32_rejects_non_zero_trailing_bits() {
        // 13 chars = 65 bits = 8 bytes + 1 leftover bit. Leftover bit 0 → ok;
        // leftover bit 1 (a malformed seed) must be rejected, not silently
        // accepted as a wrong TOTP key.
        assert!(base32_decode("AAAAAAAAAAAAA").is_ok());
        assert!(base32_decode("AAAAAAAAAAAAB").is_err());
    }

    #[test]
    fn encrypt_roundtrip() {
        let key = [7u8; 32];
        let cred = Cred {
            username: "u".into(),
            password: "p".into(),
            totp: Some("JBSWY3DPEHPK3PXP".into()),
        };
        let enc = encrypt(&key, &serde_json::to_string(&cred).unwrap()).unwrap();
        let back = decrypt(&key, &enc).unwrap();
        assert_eq!(back.password, "p");
        assert_eq!(back.totp.as_deref(), Some("JBSWY3DPEHPK3PXP"));
    }

    #[test]
    fn file_backed_set_get_list_remove() {
        let dir = std::env::temp_dir().join(format!("webrain-vault-test-{}", std::process::id()));
        // SAFETY: single-threaded per process; no other vault test reads this env var.
        unsafe { std::env::set_var("WEBRAIN_VAULT_DIR", &dir) };
        let _ = std::fs::remove_dir_all(&dir); // start clean

        set(
            "instagram",
            "me",
            "markosant45",
            "s3cr3t",
            Some("JBSWY3DPEHPK3PXP".into()),
        )
        .unwrap();
        set("github", "me", "markosant45", "gh-pat", None).unwrap();

        let metas = list().unwrap();
        assert_eq!(metas.len(), 2);
        assert!(
            metas
                .iter()
                .all(|m| !m.profile.is_empty() && !m.service.is_empty())
        );

        let cred = get("instagram", "me").unwrap();
        assert_eq!(cred.username, "markosant45");
        assert_eq!(cred.password, "s3cr3t");
        assert_eq!(cred.totp.as_deref(), Some("JBSWY3DPEHPK3PXP"));

        // upsert overwrites, doesn't duplicate
        set("instagram", "me", "markosant45", "new-pass", None).unwrap();
        assert_eq!(list().unwrap().len(), 2);
        assert_eq!(get("instagram", "me").unwrap().password, "new-pass");

        remove("instagram", "me").unwrap();
        assert_eq!(list().unwrap().len(), 1);
        assert!(get("instagram", "me").is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
