// Cryptographic signing module
// This file handles transaction signing, key management, and cryptographic operations
// required for interacting with the Sui blockchain
//
// Numan Thabit 2025 Nov

use crate::errors::AggrError;
use base64::{engine::general_purpose::STANDARD_NO_PAD as B64, Engine as _};
use blake2::{Blake2b512, Digest};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use hex::FromHex;

const INTENT_SCOPE_TRANSACTION_DATA: u8 = 0x00;
const INTENT_VERSION: u8 = 0x00;
const INTENT_APP_ID_SUI: u8 = 0x00;

/// Construct the Sui "intent message" = 3-byte intent header || BCS TransactionData bytes.
/// Hash to 32 bytes with Blake2b, then sign with Ed25519. Output serialized signature
/// format: `flag || signature || pubkey` where flag=0x00 for Ed25519.
/// See Sui signatures spec.  [oai_citation:0‡Sui Documentation](https://docs.sui.io/concepts/cryptography/transaction-auth/signatures?utm_source=chatgpt.com)
pub fn sign_tx_bcs_ed25519_to_serialized_signature(
    tx_bcs: &[u8],
    secret_hex: &str,
) -> Result<(Vec<u8>, [u8; 32]), AggrError> {
    let sk_bytes = <[u8; 32]>::from_hex(secret_hex)
        .map_err(|e| AggrError::Signing(format!("bad hex key: {e}")))?;
    let signing_key = SigningKey::from_bytes(&sk_bytes);
    let vk: VerifyingKey = signing_key.verifying_key();

    // Compose intent message bytes.
    let mut intent = Vec::with_capacity(3 + tx_bcs.len());
    intent.push(INTENT_SCOPE_TRANSACTION_DATA);
    intent.push(INTENT_VERSION);
    intent.push(INTENT_APP_ID_SUI);
    intent.extend_from_slice(tx_bcs);

    // Blake2b-256 hash of intent message.
    let mut hasher = Blake2b512::new();
    hasher.update(&intent);
    let hash_result = hasher.finalize();
    // Take first 32 bytes for 256-bit hash
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&hash_result[..32]);

    // Sign the digest.
    let sig = signing_key.sign(&digest);
    let sig_bytes: [u8; 64] = sig.to_bytes();

    let pk_bytes: [u8; 32] = vk.to_bytes();

    // Serialized signature per Sui spec: flag || signature || pubkey
    let mut serialized = Vec::with_capacity(1 + 64 + 32);
    serialized.push(0x00); // Ed25519 flag
    serialized.extend_from_slice(&sig_bytes);
    serialized.extend_from_slice(&pk_bytes);

    Ok((serialized, pk_bytes))
}

/// Base64 for JSON-RPC submit.
pub fn serialize_signature_b64(sig_ser: &[u8]) -> String {
    B64.encode(sig_ser)
}

/// Sign a transaction with multiple signers (e.g., user + sponsor)
/// Returns a vector of serialized signatures in order
pub fn sign_tx_bcs_multi_ed25519(
    tx_bcs: &[u8],
    secret_keys_hex: &[&str],
) -> Result<Vec<Vec<u8>>, AggrError> {
    let mut signatures = Vec::new();
    for secret_hex in secret_keys_hex {
        let (sig_bytes, _) = sign_tx_bcs_ed25519_to_serialized_signature(tx_bcs, secret_hex)?;
        signatures.push(sig_bytes);
    }
    Ok(signatures)
}
