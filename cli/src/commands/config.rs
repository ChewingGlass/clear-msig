use crate::config::{PersistedConfig, config_path};
use crate::error::*;
use clap::Subcommand;

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Set a configuration value
    Set {
        /// RPC URL
        #[arg(long)]
        url: Option<String>,
        /// Path to payer keypair
        #[arg(long)]
        payer: Option<String>,
        /// Path to signer keypair
        #[arg(long)]
        signer: Option<String>,
        /// Use Ledger as signer
        #[arg(long)]
        signer_ledger: bool,
        /// Default message expiry in seconds from now (default: 300 = 5 minutes)
        #[arg(long)]
        expiry_seconds: Option<u64>,
    },
    /// Show current configuration
    Show,
}

pub fn handle(action: ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Set { url, payer, signer, signer_ledger, expiry_seconds } => {
            let mut config = PersistedConfig::load();
            if let Some(url) = url { config.rpc_url = url; }
            if let Some(payer) = payer { config.payer = payer; }
            if let Some(signer) = signer {
                config.signer = signer;
                config.signer_type = crate::config::SignerType::Keypair;
            }
            if signer_ledger {
                config.signer_type = crate::config::SignerType::Ledger;
            }
            if let Some(seconds) = expiry_seconds {
                config.expiry_seconds = seconds;
            }
            config.save()?;
            let json = serde_json::to_string_pretty(&config)?;
            println!("{json}");
        }
        ConfigAction::Show => {
            let config = PersistedConfig::load();
            let mut output = serde_json::to_value(&config)?;
            output["config_path"] = serde_json::Value::String(config_path().to_string_lossy().to_string());
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
    }
    Ok(())
}
