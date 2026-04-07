use crate::error::*;
use ed25519_dalek::Signer;

pub trait MessageSigner {
    fn pubkey(&self) -> [u8; 32];
    fn sign_message(&self, message: &[u8]) -> Result<[u8; 64]>;
}

pub struct KeypairMessageSigner {
    key: ed25519_dalek::SigningKey,
}

impl KeypairMessageSigner {
    pub fn from_file(path: &str) -> Result<Self> {
        let expanded = shellexpand::tilde(path).to_string();
        let data = std::fs::read_to_string(&expanded)
            .with_context(|| format!("reading signer keypair from {expanded}"))?;
        let bytes: Vec<u8> = serde_json::from_str(&data)
            .with_context(|| "parsing signer keypair JSON")?;
        // Solana keypair JSON is 64 bytes: [secret_key(32) ++ public_key(32)]
        if bytes.len() < 32 {
            return Err(anyhow!("keypair too short"));
        }
        let secret: [u8; 32] = bytes[..32].try_into()?;
        let key = ed25519_dalek::SigningKey::from_bytes(&secret);
        Ok(Self { key })
    }
}

impl MessageSigner for KeypairMessageSigner {
    fn pubkey(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }

    fn sign_message(&self, message: &[u8]) -> Result<[u8; 64]> {
        Ok(self.key.sign(message).to_bytes())
    }
}

pub struct LedgerMessageSigner {
    wallet_manager: std::rc::Rc<solana_remote_wallet::remote_wallet::RemoteWalletManager>,
    derivation_path: solana_sdk::derivation_path::DerivationPath,
    cached_pubkey: [u8; 32],
}

impl LedgerMessageSigner {
    pub fn new() -> Result<Self> {
        let wallet_manager = solana_remote_wallet::remote_wallet::initialize_wallet_manager()
            .map_err(|e| anyhow!("failed to initialize wallet manager: {e}"))?;

        wallet_manager.update_devices()
            .map_err(|e| anyhow!("failed to detect Ledger devices: {e}"))?;

        let devices = wallet_manager.list_devices();
        if devices.is_empty() {
            return Err(anyhow!("no Ledger device found — is it connected and unlocked with the Solana app open?"));
        }

        let derivation_path = solana_sdk::derivation_path::DerivationPath::default();

        let locator = solana_remote_wallet::locator::Locator {
            manufacturer: solana_remote_wallet::locator::Manufacturer::Ledger,
            pubkey: None,
        };

        let keypair = solana_remote_wallet::remote_keypair::generate_remote_keypair(
            locator,
            derivation_path.clone(),
            &wallet_manager,
            false,
            "signer",
        ).map_err(|e| anyhow!("failed to connect to Ledger: {e}"))?;

        let cached_pubkey = solana_sdk::signer::Signer::pubkey(&keypair).to_bytes();

        Ok(Self {
            wallet_manager,
            derivation_path,
            cached_pubkey,
        })
    }
}

impl MessageSigner for LedgerMessageSigner {
    fn pubkey(&self) -> [u8; 32] {
        self.cached_pubkey
    }

    fn sign_message(&self, message: &[u8]) -> Result<[u8; 64]> {
        let locator = solana_remote_wallet::locator::Locator {
            manufacturer: solana_remote_wallet::locator::Manufacturer::Ledger,
            pubkey: None,
        };

        let keypair = solana_remote_wallet::remote_keypair::generate_remote_keypair(
            locator,
            self.derivation_path.clone(),
            &self.wallet_manager,
            false,
            "signer",
        ).map_err(|e| anyhow!("failed to connect to Ledger: {e}"))?;

        let signature = solana_sdk::signer::Signer::try_sign_message(&keypair, message)
            .map_err(|e| anyhow!("Ledger signing failed: {e}"))?;

        Ok(signature.into())
    }
}
