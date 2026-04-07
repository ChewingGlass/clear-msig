use crate::error::*;
use crate::signing::{KeypairMessageSigner, MessageSigner};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct PersistedConfig {
    #[serde(default = "default_rpc_url")]
    pub rpc_url: String,
    #[serde(default = "default_payer_path")]
    pub payer: String,
    #[serde(default = "default_payer_path")]
    pub signer: String,
    #[serde(default)]
    pub signer_type: SignerType,
    #[serde(default = "default_expiry_seconds")]
    pub expiry_seconds: u64,
}

fn default_expiry_seconds() -> u64 { 300 }

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "snake_case")]
pub enum SignerType {
    #[default]
    Keypair,
    Ledger,
}

fn default_rpc_url() -> String {
    "http://localhost:8899".to_string()
}

fn default_payer_path() -> String {
    let home = dirs::home_dir().unwrap_or_default();
    home.join(".config/solana/id.json").to_string_lossy().to_string()
}

pub fn config_path() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_default();
    home.join(".config/clear-msig/config.json")
}

impl PersistedConfig {
    pub fn load() -> Self {
        let path = config_path();
        if path.exists() {
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }
}

/// Loaded runtime config with resolved keypair and signer.
pub struct RuntimeConfig {
    pub rpc_url: String,
    pub payer: solana_sdk::signer::keypair::Keypair,
    pub signer: Box<dyn MessageSigner>,
    pub expiry_seconds: u64,
}

impl RuntimeConfig {
    /// Compute the default expiry timestamp (now + configured expiry_seconds).
    pub fn default_expiry(&self) -> i64 {
        chrono::Utc::now().timestamp() + self.expiry_seconds as i64
    }
}

pub fn load_config(
    url_override: &Option<String>,
    keypair_override: &Option<String>,
    signer_override: &Option<String>,
    signer_ledger: bool,
) -> RuntimeConfig {
    let persisted = PersistedConfig::load();

    let rpc_url = url_override
        .clone()
        .unwrap_or(persisted.rpc_url);

    let payer_path = keypair_override
        .clone()
        .unwrap_or(persisted.payer);
    let payer = load_keypair(&payer_path)
        .expect(&format!("Failed to load payer keypair from {payer_path}"));

    let use_ledger = signer_ledger || matches!(persisted.signer_type, SignerType::Ledger);
    let signer: Box<dyn MessageSigner> = if use_ledger {
        Box::new(
            crate::signing::LedgerMessageSigner::new()
                .expect("Failed to connect to Ledger"),
        )
    } else {
        let signer_path = signer_override
            .clone()
            .unwrap_or(persisted.signer);
        Box::new(
            KeypairMessageSigner::from_file(&signer_path)
                .expect(&format!("Failed to load signer keypair from {signer_path}")),
        )
    };

    let expiry_seconds = persisted.expiry_seconds;
    RuntimeConfig { rpc_url, payer, signer, expiry_seconds }
}

fn load_keypair(path: &str) -> Result<solana_sdk::signer::keypair::Keypair> {
    let expanded = shellexpand::tilde(path).to_string();
    let data = std::fs::read_to_string(&expanded)
        .with_context(|| format!("reading keypair from {expanded}"))?;
    let bytes: Vec<u8> = serde_json::from_str(&data)
        .with_context(|| "parsing keypair JSON")?;
    solana_sdk::signer::keypair::Keypair::try_from(bytes.as_slice())
        .map_err(|e| anyhow!("invalid keypair: {e}"))
}
