use std::collections::HashSet;

use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use rand::{Rng, distr::Alphanumeric};
use tokio::sync::Mutex;

use crate::error::{ClawError, Result};

#[derive(Debug, Default)]
pub struct NonceStore {
    pending: Mutex<HashSet<String>>,
}

impl NonceStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn issue(&self) -> String {
        let nonce: String = rand::rng()
            .sample_iter(&Alphanumeric)
            .take(32)
            .map(char::from)
            .collect();

        let mut pending = self.pending.lock().await;
        pending.insert(nonce.clone());
        nonce
    }

    pub async fn consume(&self, nonce: &str) -> bool {
        let mut pending = self.pending.lock().await;
        pending.remove(nonce)
    }
}

pub fn random_challenge() -> String {
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(48)
        .map(char::from)
        .collect()
}

pub fn verify_ed25519_signature(
    public_key_b64: &str,
    signature_b64: &str,
    message: &[u8],
) -> Result<()> {
    let pk_bytes = base64::engine::general_purpose::STANDARD
        .decode(public_key_b64)
        .map_err(|e| ClawError::Auth(format!("invalid public key base64: {e}")))?;
    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(signature_b64)
        .map_err(|e| ClawError::Auth(format!("invalid signature base64: {e}")))?;

    let pk_arr: [u8; 32] = pk_bytes
        .try_into()
        .map_err(|_| ClawError::Auth("public key must be 32 bytes".to_string()))?;
    let sig_arr: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| ClawError::Auth("signature must be 64 bytes".to_string()))?;

    let key = VerifyingKey::from_bytes(&pk_arr)
        .map_err(|e| ClawError::Auth(format!("invalid public key: {e}")))?;
    let sig = Signature::from_bytes(&sig_arr);

    key.verify(message, &sig)
        .map_err(|e| ClawError::Auth(format!("signature verify failed: {e}")))
}

pub fn json_depth(raw: &str) -> usize {
    let mut max_depth = 0usize;
    let mut current = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for ch in raw.chars() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' | '[' => {
                current += 1;
                max_depth = max_depth.max(current);
            }
            '}' | ']' => {
                current = current.saturating_sub(1);
            }
            _ => {}
        }
    }

    max_depth
}
