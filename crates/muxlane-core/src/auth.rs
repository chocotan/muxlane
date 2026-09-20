//! HMAC-SHA256 token（hook / 配对后的 RPC）。
//! Hook：`v1:<expiry_unix>:<base64url(mac)>`。
//! 手机配对：`v1:<expiry_unix>:<base64url(subject)>:<base64url(mac)>`。
use crate::Result;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::path::Path;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct AuthSecret(pub Vec<u8>);

impl AuthSecret {
    pub fn generate() -> Self {
        let bytes: [u8; 32] = rand::random();
        Self(bytes.to_vec())
    }

    pub fn load_or_create(path: &Path) -> Result<Self> {
        if path.exists() {
            let bytes = std::fs::read(path)?;
            if bytes.len() >= 32 {
                force_private(path)?;
                return Ok(Self(bytes));
            }
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let s = Self::generate();
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(path)?;
            file.write_all(&s.0)?;
            file.sync_all()?;
        }
        #[cfg(not(unix))]
        std::fs::write(path, &s.0)?;
        force_private(path)?;
        Ok(s)
    }

    pub fn token(&self, subject: &str, ttl_secs: u64) -> String {
        let expiry = crate::model::now_secs().saturating_add(ttl_secs);
        self.token_at(subject, expiry)
    }

    pub fn token_at(&self, subject: &str, expiry: u64) -> String {
        let sig = self.sign(subject, expiry);
        if subject.starts_with("mobile:") {
            let subject_b64 =
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(subject.as_bytes());
            format!("v1:{expiry}:{subject_b64}:{sig}")
        } else {
            format!("v1:{expiry}:{sig}")
        }
    }

    fn sign(&self, subject: &str, expiry: u64) -> String {
        let msg = format!("{subject}\n{expiry}");
        let mut mac = HmacSha256::new_from_slice(&self.0).expect("HMAC accepts any key");
        mac.update(msg.as_bytes());
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    }

    pub fn verify(&self, subject: &str, token: &str) -> bool {
        match parse_token(token) {
            Some(parsed) if parsed.subject.as_deref().unwrap_or(subject) == subject => {
                self.verify_parsed(subject, &parsed)
            }
            _ => false,
        }
    }

    /// Verify a mobile pairing token (`v1:expiry:subject:mac`) and return its subject.
    pub fn verify_mobile(&self, token: &str) -> Option<String> {
        let parsed = parse_token(token)?;
        let subject = parsed.subject.clone()?;
        if !subject.starts_with("mobile:") {
            return None;
        }
        self.verify_parsed(&subject, &parsed).then_some(subject)
    }

    fn verify_parsed(&self, subject: &str, parsed: &ParsedToken) -> bool {
        if parsed.expiry < crate::model::now_secs() {
            return false;
        }
        let Ok(sig) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&parsed.sig) else {
            return false;
        };
        let msg = format!("{subject}\n{}", parsed.expiry);
        let Ok(mut mac) = HmacSha256::new_from_slice(&self.0) else {
            return false;
        };
        mac.update(msg.as_bytes());
        mac.verify_slice(&sig).is_ok()
    }
}

struct ParsedToken {
    expiry: u64,
    subject: Option<String>,
    sig: String,
}

fn parse_token(token: &str) -> Option<ParsedToken> {
    let mut parts = token.split(':');
    if parts.next() != Some("v1") {
        return None;
    }
    let expiry = parts.next().and_then(|v| v.parse::<u64>().ok())?;
    let second = parts.next()?.to_string();
    match parts.next() {
        Some(sig) if parts.next().is_none() => {
            let subject = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(second.as_bytes())
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok());
            Some(ParsedToken {
                expiry,
                subject,
                sig: sig.to_string(),
            })
        }
        None => Some(ParsedToken {
            expiry,
            subject: None,
            sig: second,
        }),
        Some(_) => None,
    }
}

fn force_private(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_roundtrip_and_tamper() {
        let s = AuthSecret(vec![7; 32]);
        let t = s.token("agent_1", 60);
        assert!(s.verify("agent_1", &t));
        assert!(!s.verify("agent_2", &t));
        let tampered = format!("{}x", t);
        assert!(!s.verify("agent_1", &tampered));
    }
    #[test]
    fn mobile_token_embeds_subject() {
        let s = AuthSecret(vec![7; 32]);
        let t = s.token("mobile:phone-a", 60);
        assert_eq!(s.verify_mobile(&t).as_deref(), Some("mobile:phone-a"));
        assert!(s.verify("mobile:phone-a", &t));
        assert!(s.verify_mobile("v1:1:abc").is_none());
    }
    #[test]
    fn expired_rejected() {
        let s = AuthSecret(vec![9; 32]);
        let t = s.token_at("a", crate::model::now_secs().saturating_sub(1));
        assert!(!s.verify("a", &t));
    }
    #[test]
    fn existing_secret_permissions_are_repaired() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("secret");
        std::fs::write(&p, [3u8; 32]).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        AuthSecret::load_or_create(&p).unwrap();
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn secret_file_is_stable() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("secret");
        let a = AuthSecret::load_or_create(&p).unwrap();
        let b = AuthSecret::load_or_create(&p).unwrap();
        assert_eq!(a.0, b.0);
    }
}
