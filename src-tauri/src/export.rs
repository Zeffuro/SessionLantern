use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use pbkdf2::pbkdf2_hmac;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};
use zeroize::{Zeroize, Zeroizing};

const ITERATIONS: u32 = 600_000;
const AAD: &[u8] = b"session-lantern:v1";
const MAX_EXPORT_BYTES: usize = 64 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Credentials {
    pub session_id: String,
    pub target_idc: String,
    pub ms_token: String,
    pub verify_fp: String,
}

impl Drop for Credentials {
    fn drop(&mut self) {
        self.session_id.zeroize();
        self.target_idc.zeroize();
        self.ms_token.zeroize();
        self.verify_fp.zeroize();
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportPayload<'a> {
    service: &'static str,
    exported_at: u64,
    cookies: &'a Credentials,
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    format: String,
    version: u8,
    kdf: Kdf,
    cipher: Cipher,
    ciphertext: String,
}

#[derive(Serialize, Deserialize)]
struct Kdf {
    name: String,
    iterations: u32,
    salt: String,
}

#[derive(Serialize, Deserialize)]
struct Cipher {
    name: String,
    iv: String,
}

pub fn encrypt_export(credentials: &Credentials, password: &str) -> Result<Vec<u8>, String> {
    if password.chars().count() < 12 || password.len() > 1024 {
        return Err(
            "Use a password with at least 12 characters and at most 1024 UTF-8 bytes.".into(),
        );
    }

    let mut salt = [0_u8; 16];
    let mut iv = [0_u8; 12];
    getrandom::fill(&mut salt).map_err(|_| "Secure random generation failed.")?;
    getrandom::fill(&mut iv).map_err(|_| "Secure random generation failed.")?;

    let exported_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .ok_or("The system clock could not be read.")?;
    let plaintext = Zeroizing::new(
        serde_json::to_vec(&ExportPayload {
            service: "tiktok",
            exported_at,
            cookies: credentials,
        })
        .map_err(|_| "The encrypted export could not be prepared.")?,
    );
    if plaintext.len() > MAX_EXPORT_BYTES {
        return Err("The encrypted export exceeds the supported file size.".into());
    }
    let mut key = Zeroizing::new([0_u8; 32]);
    pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, ITERATIONS, &mut *key);
    let cipher = Aes256Gcm::new_from_slice(&*key)
        .map_err(|_| "The encrypted export could not be prepared.")?;
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&iv),
            Payload {
                msg: &plaintext,
                aad: AAD,
            },
        )
        .map_err(|_| "The encrypted export could not be prepared.")?;

    let output = serde_json::to_vec_pretty(&Envelope {
        format: "session-lantern".into(),
        version: 1,
        kdf: Kdf {
            name: "PBKDF2-SHA256".into(),
            iterations: ITERATIONS,
            salt: STANDARD.encode(salt),
        },
        cipher: Cipher {
            name: "AES-256-GCM".into(),
            iv: STANDARD.encode(iv),
        },
        ciphertext: STANDARD.encode(ciphertext),
    })
    .map_err(|_| "The encrypted export could not be prepared.")?;
    if output.len() > MAX_EXPORT_BYTES {
        return Err("The encrypted export exceeds the supported file size.".into());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const PASSWORD: &str = "synthetic passphrase only";

    fn credentials() -> Credentials {
        Credentials {
            session_id: "dummy-session-secret".into(),
            target_idc: "dummy-region-secret".into(),
            ms_token: "dummy-token-secret".into(),
            verify_fp: "dummy-fingerprint-secret".into(),
        }
    }

    fn decrypt(envelope: &Envelope, password: &str, aad: &[u8]) -> Result<Vec<u8>, ()> {
        let salt = STANDARD.decode(&envelope.kdf.salt).unwrap();
        let iv = STANDARD.decode(&envelope.cipher.iv).unwrap();
        let ciphertext = STANDARD.decode(&envelope.ciphertext).unwrap();
        let mut key = Zeroizing::new([0_u8; 32]);
        pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, ITERATIONS, &mut *key);
        Aes256Gcm::new_from_slice(&*key)
            .unwrap()
            .decrypt(
                Nonce::from_slice(&iv),
                Payload {
                    msg: &ciphertext,
                    aad,
                },
            )
            .map_err(|_| ())
    }

    #[test]
    fn envelope_and_payload_have_expected_schema_without_plaintext() {
        let output = encrypt_export(&credentials(), PASSWORD).unwrap();
        let text = std::str::from_utf8(&output).unwrap();
        for secret in [
            "dummy-session-secret",
            "dummy-region-secret",
            "dummy-token-secret",
            "dummy-fingerprint-secret",
            PASSWORD,
            "sessionId",
        ] {
            assert!(!text.contains(secret));
        }
        let envelope: Envelope = serde_json::from_slice(&output).unwrap();
        assert_eq!(envelope.format, "session-lantern");
        assert_eq!(envelope.version, 1);
        assert_eq!(envelope.kdf.name, "PBKDF2-SHA256");
        assert_eq!(envelope.kdf.iterations, 600_000);
        assert_eq!(STANDARD.decode(&envelope.kdf.salt).unwrap().len(), 16);
        assert_eq!(envelope.cipher.name, "AES-256-GCM");
        assert_eq!(STANDARD.decode(&envelope.cipher.iv).unwrap().len(), 12);
        let plaintext = Zeroizing::new(decrypt(&envelope, PASSWORD, AAD).unwrap());
        let payload: Value = serde_json::from_slice(&plaintext).unwrap();
        assert_eq!(payload["service"], "tiktok");
        assert!(payload["exportedAt"].as_u64().unwrap() > 0);
        assert_eq!(payload["cookies"]["sessionId"], "dummy-session-secret");
        assert_eq!(payload["cookies"]["targetIdc"], "dummy-region-secret");
        assert_eq!(payload["cookies"]["msToken"], "dummy-token-secret");
        assert_eq!(payload["cookies"]["verifyFp"], "dummy-fingerprint-secret");
    }

    #[test]
    fn every_export_has_fresh_salt_iv_and_ciphertext() {
        let first: Envelope =
            serde_json::from_slice(&encrypt_export(&credentials(), PASSWORD).unwrap()).unwrap();
        let second: Envelope =
            serde_json::from_slice(&encrypt_export(&credentials(), PASSWORD).unwrap()).unwrap();
        assert_ne!(first.kdf.salt, second.kdf.salt);
        assert_ne!(first.cipher.iv, second.cipher.iv);
        assert_ne!(first.ciphertext, second.ciphertext);
    }

    #[test]
    fn authentication_rejects_wrong_password_tampering_and_wrong_aad() {
        let mut envelope: Envelope =
            serde_json::from_slice(&encrypt_export(&credentials(), PASSWORD).unwrap()).unwrap();
        assert!(decrypt(&envelope, "incorrect synthetic password", AAD).is_err());
        assert!(decrypt(&envelope, PASSWORD, b"session-lantern:v2").is_err());
        let mut ciphertext = STANDARD.decode(&envelope.ciphertext).unwrap();
        ciphertext[0] ^= 1;
        envelope.ciphertext = STANDARD.encode(ciphertext);
        assert!(decrypt(&envelope, PASSWORD, AAD).is_err());
    }

    #[test]
    fn password_limits_count_characters_and_utf8_bytes_without_trimming() {
        assert!(encrypt_export(&credentials(), "elevenchars").is_err());
        assert!(encrypt_export(&credentials(), &"x".repeat(1025)).is_err());
        assert!(encrypt_export(&credentials(), &"é".repeat(513)).is_err());
        let password = "  café 🔑 测试 passphrase  ";
        let envelope: Envelope =
            serde_json::from_slice(&encrypt_export(&credentials(), password).unwrap()).unwrap();
        assert!(decrypt(&envelope, password, AAD).is_ok());
        assert!(decrypt(&envelope, password.trim(), AAD).is_err());
    }

    #[test]
    fn export_rejects_oversized_credentials() {
        for length in [49_000, MAX_EXPORT_BYTES + 1] {
            let mut oversized = credentials();
            oversized.session_id = "x".repeat(length);
            assert!(encrypt_export(&oversized, PASSWORD).is_err());
        }
    }
}
