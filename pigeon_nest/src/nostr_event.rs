use once_cell::sync::Lazy;
use regex::Regex;
use secp256k1::hashes::{Hash, sha256};
use secp256k1::{Secp256k1, XOnlyPublicKey};
use serde::{Deserialize, Serialize};
use validator::{Validate, ValidationError};

static HEX_32_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[0-9a-fA-F]{64}$").unwrap());
static HEX_64_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[0-9a-fA-F]{128}$").unwrap());

fn validate_hex_32(value: &str) -> Result<(), ValidationError> {
    if !HEX_32_PATTERN.is_match(value) {
        return Err(ValidationError::new("invalid_hex_32"));
    }
    Ok(())
}

fn validate_hex_64(value: &str) -> Result<(), ValidationError> {
    if !HEX_64_PATTERN.is_match(value) {
        return Err(ValidationError::new("invalid_hex_64"));
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, Validate, PartialEq)]
pub struct NostrEvent {
    #[validate(custom(function = "validate_hex_32"))]
    id: String,

    #[validate(custom(function = "validate_hex_32"))]
    pubkey: String,

    created_at: u64,

    #[validate(range(min = 0, max = 65535))]
    kind: u16,

    tags: Vec<Vec<String>>,

    content: String,

    #[validate(custom(function = "validate_hex_64"))]
    sig: String,
}

impl NostrEvent {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn pubkey(&self) -> &str {
        &self.pubkey
    }

    pub fn kind(&self) -> u16 {
        self.kind
    }

    pub fn tags(&self) -> &Vec<Vec<String>> {
        &self.tags
    }

    pub fn created_at(&self) -> u64 {
        self.created_at
    }

    pub fn from_value(value: &serde_json::Value) -> Result<Self, anyhow::Error> {
        let event: Self = serde_json::from_value(value.clone())?;
        event.validate()?;
        Ok(event)
    }

    pub fn verify(&self) -> bool {
        // serialize and hash
        let serialized = serde_json::json!([
            0,
            self.pubkey,
            self.created_at,
            self.kind,
            self.tags,
            self.content
        ]);
        let serialized_str = serde_json::to_string(&serialized).unwrap_or_default();
        let digest = sha256::Hash::hash(serialized_str.as_bytes());

        let calculated_id = hex::encode(digest);

        // verify id
        if calculated_id != self.id {
            return false;
        }

        // set up verification context
        let secp = Secp256k1::verification_only();

        // format pubkey and sig
        let pubkey_bytes = match hex::decode(&self.pubkey) {
            Ok(bytes) => bytes,
            Err(_) => return false,
        };
        let pubkey = match XOnlyPublicKey::from_slice(&pubkey_bytes) {
            Ok(key) => key,
            Err(_) => return false,
        };

        let sig_bytes = match hex::decode(&self.sig) {
            Ok(bytes) => bytes,
            Err(_) => return false,
        };
        let sig = match secp256k1::schnorr::Signature::from_slice(&sig_bytes) {
            Ok(s) => s,
            Err(_) => return false,
        };

        // verify signature
        secp.verify_schnorr(&sig, &digest.to_byte_array(), &pubkey)
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_hex_32() {
        assert!(
            validate_hex_32("8c29b25bdd80d390edb85fa2f214f080fa189996e02e8cd4cf24aeddc86402e2")
                .is_ok()
        );
        assert!(
            validate_hex_32("8c29b25bdd80d390edb85fa2f214f080fa189996e02e8cd4cf24aeddc86402e")
                .is_err()
        );
        assert!(
            validate_hex_32("8c29b25bdd80d390edb85fa2f214f080fa189996e02e8cd4cf24aeddc86402eg")
                .is_err()
        );
    }

    #[test]
    fn test_validate_hex_64() {
        assert!(
            validate_hex_64("8b483ca1a645b0648bf432807a1dfdc0906bce148a1a92a3f35820b103f5e891b606d53687660ff0bf08738ada3060a1d736b0183cef5cf8ad3ea198773c3d0e")
                .is_ok()
        );
        assert!(
            validate_hex_64("8b483ca1a645b0648bf432807a1dfdc0906bce148a1a92a3f35820b103f5e891b606d53687660ff0bf08738ada3060a1d736b0183cef5cf8ad3ea198773c3d0")
                .is_err()
        );
        assert!(
            validate_hex_64("8b483ca1a645b0648bf432807a1dfdc0906bce148a1a92a3f35820b103f5e891b606d53687660ff0bf08738ada3060a1d736b0183cef5cf8ad3ea198773c3d0g")
                .is_err()
        );
    }

    #[test]
    fn test_nostr_event_from_value() {
        let json_str = r#"
        ["EVENT", {
            "id": "8396b726fcfcb6d2253a6a148f9c652242c4e3270ebd20015005ad817841fc27",
            "pubkey": "5e086d4f1b4270bf896da87fa9df8ab5cca1f646c1beaaf120b34796fcae24f2",
            "created_at": 1742023226,
            "kind": 1,
            "tags": [
                [
                "t",
                "test1",
                "",
                "test2"
                ]
            ],
            "content": "",
            "sig": "f7998a7021e7b06a47a20af366aa5962386c029860259c35aa9e7e7087a031890b557c69a661c58fef23480e55b136f6171bd403920bb3152cf20124f4e7a6f9"
        }]"#;

        let parsed: serde_json::Value = serde_json::from_str(json_str).unwrap();
        if let serde_json::Value::Array(arr) = parsed {
            if arr.len() != 2 {
                panic!("message is not a valid JSON array");
            }

            let event = NostrEvent::from_value(&arr[1]).unwrap();

            assert_eq!(
                    event,
                    NostrEvent {
                        id: "8396b726fcfcb6d2253a6a148f9c652242c4e3270ebd20015005ad817841fc27".to_string(),
                        pubkey: "5e086d4f1b4270bf896da87fa9df8ab5cca1f646c1beaaf120b34796fcae24f2".to_string(),
                        created_at: 1742023226,
                        kind: 1,
                        tags: vec![vec!["t".to_string(), "test1".to_string(), "".to_string(), "test2".to_string()]],
                        content: "".to_string(),
                        sig: "f7998a7021e7b06a47a20af366aa5962386c029860259c35aa9e7e7087a031890b557c69a661c58fef23480e55b136f6171bd403920bb3152cf20124f4e7a6f9".to_string()
                    }
                );
            assert!(event.verify());
        } else {
            panic!("message is not a JSON array");
        }
    }
}
