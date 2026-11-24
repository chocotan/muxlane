//! End-to-end encrypted RPC transport framing using P-256 ECDH + HKDF-SHA256 + AES-256-GCM.
//! Authenticated with the pairing token so relays cannot MITM key agreement or inspect frames.
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use hkdf::Hkdf;
use p256::ecdh::EphemeralSecret;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::PublicKey;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

const INFO_C2S: &[u8] = b"muxlane-e2ee-c2s-v1";
const INFO_S2C: &[u8] = b"muxlane-e2ee-s2c-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum E2eeRole {
    Client,
    Server,
}

pub struct E2eeKeyPair {
    secret: EphemeralSecret,
    public_sec1: Vec<u8>,
}

impl E2eeKeyPair {
    pub fn generate() -> Self {
        let secret = EphemeralSecret::random(&mut p256::elliptic_curve::rand_core::OsRng);
        let public_sec1 = secret
            .public_key()
            .to_encoded_point(false)
            .as_bytes()
            .to_vec();
        Self {
            secret,
            public_sec1,
        }
    }

    pub fn public_bytes(&self) -> &[u8] {
        &self.public_sec1
    }
}

pub fn random_nonce16() -> [u8; 16] {
    let mut out = [0u8; 16];
    rand::rng().fill_bytes(&mut out);
    out
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct E2eeEnvelope {
    pub e2ee: u8,
    pub s: u64,
    pub p: String,
}

pub struct E2eeSession {
    tx_cipher: Aes256Gcm,
    rx_cipher: Aes256Gcm,
    tx_dir: u8,
    rx_dir: u8,
    tx_seq: u64,
    rx_seq: u64,
}

impl E2eeSession {
    pub fn derive(
        role: E2eeRole,
        local_keys: E2eeKeyPair,
        peer_pubkey_sec1: &[u8],
        pairing_token: &str,
        client_nonce: &[u8],
        server_nonce: &[u8],
    ) -> crate::Result<Self> {
        let peer_pub = PublicKey::from_sec1_bytes(peer_pubkey_sec1)
            .map_err(|e| crate::Error::InvalidFrame(format!("invalid P-256 peer key: {e}")))?;
        let shared = local_keys.secret.diffie_hellman(&peer_pub);

        let mut salt =
            Vec::with_capacity(pairing_token.len() + client_nonce.len() + server_nonce.len());
        salt.extend_from_slice(pairing_token.as_bytes());
        salt.extend_from_slice(client_nonce);
        salt.extend_from_slice(server_nonce);

        let hk = Hkdf::<Sha256>::new(Some(&salt), shared.raw_secret_bytes().as_slice());
        let mut c2s = [0u8; 32];
        let mut s2c = [0u8; 32];
        hk.expand(INFO_C2S, &mut c2s)
            .map_err(|_| crate::Error::InvalidFrame("hkdf c2s expand failed".into()))?;
        hk.expand(INFO_S2C, &mut s2c)
            .map_err(|_| crate::Error::InvalidFrame("hkdf s2c expand failed".into()))?;

        let (tx_bytes, rx_bytes, tx_dir, rx_dir) = match role {
            E2eeRole::Client => (c2s, s2c, 0x01, 0x02),
            E2eeRole::Server => (s2c, c2s, 0x02, 0x01),
        };
        Ok(Self {
            tx_cipher: Aes256Gcm::new_from_slice(&tx_bytes)
                .map_err(|_| crate::Error::InvalidFrame("invalid aes tx key".into()))?,
            rx_cipher: Aes256Gcm::new_from_slice(&rx_bytes)
                .map_err(|_| crate::Error::InvalidFrame("invalid aes rx key".into()))?,
            tx_dir,
            rx_dir,
            tx_seq: 0,
            rx_seq: 0,
        })
    }

    pub fn seal(&mut self, plaintext: &[u8]) -> crate::Result<E2eeEnvelope> {
        self.tx_seq = self
            .tx_seq
            .checked_add(1)
            .ok_or_else(|| crate::Error::InvalidFrame("e2ee sequence overflow".into()))?;
        let nonce_bytes = build_nonce(self.tx_dir, self.tx_seq);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ct = self
            .tx_cipher
            .encrypt(
                nonce,
                Payload {
                    msg: plaintext,
                    aad: &nonce_bytes,
                },
            )
            .map_err(|_| crate::Error::InvalidFrame("e2ee encrypt failed".into()))?;
        Ok(E2eeEnvelope {
            e2ee: 1,
            s: self.tx_seq,
            p: base64::engine::general_purpose::STANDARD.encode(&ct),
        })
    }

    pub fn seal_json<T: Serialize>(&mut self, value: &T) -> crate::Result<E2eeEnvelope> {
        let bytes = serde_json::to_vec(value)?;
        self.seal(&bytes)
    }

    pub fn open(&mut self, env: &E2eeEnvelope) -> crate::Result<Vec<u8>> {
        if env.e2ee != 1 {
            return Err(crate::Error::InvalidFrame(format!(
                "unsupported e2ee version: {}",
                env.e2ee
            )));
        }
        if env.s <= self.rx_seq {
            return Err(crate::Error::InvalidFrame(
                "replayed or out-of-order e2ee sequence".into(),
            ));
        }
        let ct = base64::engine::general_purpose::STANDARD
            .decode(&env.p)
            .map_err(|e| crate::Error::InvalidFrame(format!("invalid e2ee base64: {e}")))?;
        let nonce_bytes = build_nonce(self.rx_dir, env.s);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let pt = self
            .rx_cipher
            .decrypt(
                nonce,
                Payload {
                    msg: &ct,
                    aad: &nonce_bytes,
                },
            )
            .map_err(|_| crate::Error::InvalidFrame("e2ee authentication failed".into()))?;
        self.rx_seq = env.s;
        Ok(pt)
    }
}

fn build_nonce(dir: u8, seq: u64) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[0] = dir;
    nonce[4..12].copy_from_slice(&seq.to_be_bytes());
    nonce
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn e2ee_roundtrips_bidirectional_and_rejects_replay_mitm_and_tamper() {
        let client_kp = E2eeKeyPair::generate();
        let server_kp = E2eeKeyPair::generate();
        let c_pub = client_kp.public_bytes().to_vec();
        let s_pub = server_kp.public_bytes().to_vec();
        let c_nonce = random_nonce16();
        let s_nonce = random_nonce16();
        let token = "tok_shared_secret";

        let mut client = E2eeSession::derive(
            E2eeRole::Client,
            client_kp,
            &s_pub,
            token,
            &c_nonce,
            &s_nonce,
        )
        .unwrap();
        let mut server = E2eeSession::derive(
            E2eeRole::Server,
            server_kp,
            &c_pub,
            token,
            &c_nonce,
            &s_nonce,
        )
        .unwrap();

        let env1 = client
            .seal(b"{\"id\":1,\"method\":\"state.list\"}")
            .unwrap();
        assert_eq!(env1.s, 1);
        let pt1 = server.open(&env1).unwrap();
        assert_eq!(pt1, b"{\"id\":1,\"method\":\"state.list\"}");

        // Replay must be rejected
        assert!(server.open(&env1).is_err());

        // Tamper must fail GCM tag
        let mut env2 = server.seal(b"{\"id\":1,\"result\":{}}").unwrap();
        let mut raw = base64::engine::general_purpose::STANDARD
            .decode(&env2.p)
            .unwrap();
        raw[0] ^= 0xff;
        env2.p = base64::engine::general_purpose::STANDARD.encode(&raw);
        assert!(client.open(&env2).is_err());

        // Wrong pairing token fails key match
        let evil_kp = E2eeKeyPair::generate();
        let mut evil = E2eeSession::derive(
            E2eeRole::Server,
            evil_kp,
            &c_pub,
            "wrong_token",
            &c_nonce,
            &s_nonce,
        )
        .unwrap();
        let env3 = client.seal(b"secret").unwrap();
        assert!(evil.open(&env3).is_err());
    }
}
