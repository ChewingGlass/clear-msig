use crate::config::RuntimeConfig;
use crate::error::*;
use crate::output::print_json;
use crate::rpc;
use clap::Subcommand;
use solana_pubkey::Pubkey;

#[derive(Subcommand)]
pub enum WalletAction {
    /// Create a new multisig wallet
    Create {
        /// Wallet name (used to derive PDA)
        #[arg(long)]
        name: String,
        /// Comma-separated proposer addresses
        #[arg(long, value_delimiter = ',')]
        proposers: Vec<String>,
        /// Comma-separated approver addresses
        #[arg(long, value_delimiter = ',')]
        approvers: Vec<String>,
        /// Approval threshold
        #[arg(long)]
        threshold: u8,
        /// Cancellation threshold
        #[arg(long, default_value = "1")]
        cancellation_threshold: u8,
        /// Timelock in seconds
        #[arg(long, default_value = "0")]
        timelock: u32,
    },
    /// Show wallet details
    Show {
        /// Wallet name
        #[arg(long)]
        name: String,
    },
}

pub fn handle(action: WalletAction, config: &RuntimeConfig) -> Result<()> {
    match action {
        WalletAction::Create {
            name,
            proposers,
            approvers,
            threshold,
            cancellation_threshold,
            timelock,
        } => {
            let program_id = crate::instructions::program_id();
            let pid = solana_address::Address::new_from_array(program_id.to_bytes());

            let (wallet_addr, _) = clear_wallet_client::pda::find_wallet_address(&name, &pid);
            let wallet = Pubkey::new_from_array(wallet_addr.to_bytes());

            let (vault_addr, _) = clear_wallet_client::pda::find_vault_address(&wallet_addr, &pid);
            let vault = Pubkey::new_from_array(vault_addr.to_bytes());

            let name_hash = clear_wallet_client::pda::compute_name_hash(&name);
            let name_hash_pubkey = Pubkey::new_from_array(name_hash);

            // Derive PDAs for the 3 default meta-intents
            let (add_intent_addr, _) =
                clear_wallet_client::pda::find_intent_address(&wallet_addr, 0, &pid);
            let (remove_intent_addr, _) =
                clear_wallet_client::pda::find_intent_address(&wallet_addr, 1, &pid);
            let (update_intent_addr, _) =
                clear_wallet_client::pda::find_intent_address(&wallet_addr, 2, &pid);

            let proposer_pubkeys: Vec<Pubkey> = proposers
                .iter()
                .map(|s| s.parse().with_context(|| format!("invalid proposer address: {s}")))
                .collect::<Result<_>>()?;
            let approver_pubkeys: Vec<Pubkey> = approvers
                .iter()
                .map(|s| s.parse().with_context(|| format!("invalid approver address: {s}")))
                .collect::<Result<_>>()?;

            let payer_pubkey = solana_signer::Signer::pubkey(&config.payer);
            let ix = crate::instructions::create_wallet(crate::instructions::CreateWalletArgs {
                payer: payer_pubkey,
                name_hash: name_hash_pubkey,
                wallet,
                add_intent: Pubkey::new_from_array(add_intent_addr.to_bytes()),
                remove_intent: Pubkey::new_from_array(remove_intent_addr.to_bytes()),
                update_intent: Pubkey::new_from_array(update_intent_addr.to_bytes()),
                name: &name,
                threshold,
                cancel_threshold: cancellation_threshold,
                timelock,
                proposers: &proposer_pubkeys,
                approvers: &approver_pubkeys,
            });

            let client = rpc::client(config);
            let sig = rpc::send_instruction(&client, config, ix)?;

            print_json(&serde_json::json!({
                "txid": sig.to_string(),
                "wallet": wallet.to_string(),
                "vault": vault.to_string(),
            }));
        }
        WalletAction::Show { name } => {
            let program_id = crate::instructions::program_id();
            let pid = solana_address::Address::new_from_array(program_id.to_bytes());
            let (wallet_addr, _) = clear_wallet_client::pda::find_wallet_address(&name, &pid);
            let wallet = Pubkey::new_from_array(wallet_addr.to_bytes());

            let client = rpc::client(config);
            let data = rpc::fetch_account(&client, &wallet)?;
            let account = crate::accounts::parse_wallet(&data)?;

            let (vault_addr, _) = clear_wallet_client::pda::find_vault_address(&wallet_addr, &pid);

            print_json(&serde_json::json!({
                "address": wallet.to_string(),
                "vault": Pubkey::new_from_array(vault_addr.to_bytes()).to_string(),
                "name": account.name,
                "proposal_index": account.proposal_index,
                "intent_index": account.intent_index,
            }));
        }
    }
    Ok(())
}
