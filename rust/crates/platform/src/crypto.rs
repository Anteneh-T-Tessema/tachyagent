use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use rand::{RngCore, thread_rng};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("Encryption failed: {0}")]
    Encryption(String),
    #[error("Decryption failed: {0}")]
    Decryption(String),
    #[error("Invalid key size")]
    InvalidKey,
}

/// A sovereign encrypted payload.
#[derive(Debug, Serialize, Deserialize)]
pub struct EncryptedPayload {
    /// AES-GCM nonce (96-bit)
    pub nonce: Vec<u8>,
    /// Ciphertext (encrypted data + auth tag)
    pub ciphertext: Vec<u8>,
}

pub struct SovereignCrypto;

impl SovereignCrypto {
    /// Encrypt a plaintext buffer using AES-256-GCM.
    pub fn encrypt(plaintext: &[u8], key: &[u8; 32]) -> Result<EncryptedPayload, CryptoError> {
        let cipher = Aes256Gcm::new_from_slice(key)
            .map_err(|_| CryptoError::InvalidKey)?;
        
        let mut nonce_bytes = [0u8; 12];
        thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| CryptoError::Encryption(e.to_string()))?;

        Ok(EncryptedPayload {
            nonce: nonce_bytes.to_vec(),
            ciphertext,
        })
    }

    /// Decrypt an encrypted payload using AES-256-GCM.
    pub fn decrypt(payload: &EncryptedPayload, key: &[u8; 32]) -> Result<Vec<u8>, CryptoError> {
        let cipher = Aes256Gcm::new_from_slice(key)
            .map_err(|_| CryptoError::InvalidKey)?;
        
        let nonce = Nonce::from_slice(&payload.nonce);

        cipher
            .decrypt(nonce, payload.ciphertext.as_slice())
            .map_err(|e| CryptoError::Decryption(e.to_string()))
    }
}

#[derive(Clone, Debug)]
pub struct AgentIdentity {
    pub signing_key: ed25519_dalek::SigningKey,
}

impl AgentIdentity {
    #[must_use]
    pub fn generate() -> Self {
        let mut csprng = thread_rng();
        let signing_key = ed25519_dalek::SigningKey::generate(&mut csprng);
        Self { signing_key }
    }

    #[must_use]
    pub fn public_key(&self) -> ed25519_dalek::VerifyingKey {
        self.signing_key.verifying_key()
    }

    pub fn sign(&self, message: &[u8]) -> ed25519_dalek::Signature {
        use ed25519_dalek::Signer;
        self.signing_key.sign(message)
    }

    pub fn verify(message: &[u8], signature: &ed25519_dalek::Signature, public_key: &ed25519_dalek::VerifyingKey) -> bool {
        use ed25519_dalek::Verifier;
        public_key.verify(message, signature).is_ok()
    }

    #[must_use]
    pub fn to_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        let signing_key = ed25519_dalek::SigningKey::from_bytes(bytes);
        Self { signing_key }
    }
}
impl AgentIdentity {
    pub fn asymmetric_sign(&self, message: &[u8]) -> Result<String, String> {
        let signature = self.sign(message);
        Ok(signature.to_bytes().iter().map(|b| format!("{b:02x}")).collect())
    }

    pub fn public_key_hex(&self) -> String {
        self.public_key().to_bytes().iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn verify_static(signature_hex: &str, public_key_hex: &str, message: &[u8]) -> Result<(), String> {
        let sig_bytes = hex::decode(signature_hex).map_err(|e| e.to_string())?;
        let pk_bytes = hex::decode(public_key_hex).map_err(|e| e.to_string())?;
        
        let signature = ed25519_dalek::Signature::from_slice(&sig_bytes).map_err(|e| e.to_string())?;
        let public_key = ed25519_dalek::VerifyingKey::from_bytes(&pk_bytes.try_into().map_err(|_| "invalid pk size")?).map_err(|e| e.to_string())?;
        
        if Self::verify(message, &signature, &public_key) {
            Ok(())
        } else {
            Err("signature verification failed".to_string())
        }
    }
}

impl audit::AsymmetricSigner for AgentIdentity {
    fn sign_payload(&self, message: &[u8]) -> (String, String) {
        let signature = self.sign(message);
        let sig_hex = signature.to_bytes().iter().map(|b| format!("{b:02x}")).collect();
        let pk_hex = self.public_key_hex();
        (sig_hex, pk_hex)
    }
}

pub struct FinancialVault {
    identity: AgentIdentity,
    policy: super::finance::InvestmentPolicy,
}

impl FinancialVault {
    pub fn new(identity: AgentIdentity, policy: super::finance::InvestmentPolicy) -> Self {
        Self { identity, policy }
    }

    pub fn authorize_transaction(&self, protocol: &str, amount: f64) -> Result<String, String> {
        if self.policy.restricted_protocols.contains(&protocol.to_string()) {
            return Err(format!("SECURITY: Protocol '{}' is restricted by investment policy.", protocol));
        }

        if amount > self.policy.max_exposure_per_protocol {
            return Err(format!("EXPOSURE: Transaction amount ${} exceeds protocol limit of ${}.", amount, self.policy.max_exposure_per_protocol));
        }

        // Sign the authorization token
        let auth_msg = format!("AUTH_TX:{}:{}", protocol, amount);
        self.identity.asymmetric_sign(auth_msg.as_bytes())
    }
}
