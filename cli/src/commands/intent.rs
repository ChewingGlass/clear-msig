use crate::config::RuntimeConfig;
use crate::error::*;
use crate::output::print_json;
use crate::{accounts, message, rpc};
use clap::Subcommand;
use clear_wallet_client::intent_json::IntentTransactionJson;
use solana_pubkey::Pubkey;

#[derive(Subcommand)]
pub enum IntentAction {
    /// Add a new intent to a wallet (proposes via AddIntent)
    Add {
        /// Wallet name
        #[arg(long)]
        wallet: String,
        /// Path to intent JSON file (transaction definition only)
        #[arg(long)]
        file: String,
        /// Comma-separated proposer addresses for this intent
        #[arg(long, value_delimiter = ',')]
        proposers: Vec<String>,
        /// Comma-separated approver addresses for this intent
        #[arg(long, value_delimiter = ',')]
        approvers: Vec<String>,
        /// Approval threshold for this intent
        #[arg(long)]
        threshold: u8,
        /// Cancellation threshold
        #[arg(long, default_value = "1")]
        cancellation_threshold: u8,
        /// Timelock in seconds
        #[arg(long, default_value = "0")]
        timelock: u32,
        /// Message expiry (YYYY-MM-DD HH:MM:SS). Defaults to now + configured expiry_seconds.
        #[arg(long)]
        expiry: Option<String>,
    },
    /// Remove an intent from a wallet (proposes via RemoveIntent)
    Remove {
        #[arg(long)]
        wallet: String,
        /// Intent index to remove
        #[arg(long)]
        index: u8,
        /// Message expiry (YYYY-MM-DD HH:MM:SS). Defaults to now + configured expiry_seconds.
        #[arg(long)]
        expiry: Option<String>,
    },
    /// Update an intent's definition (proposes via UpdateIntent)
    Update {
        #[arg(long)]
        wallet: String,
        /// Intent index to update
        #[arg(long)]
        index: u8,
        /// Path to new intent JSON file
        #[arg(long)]
        file: String,
        #[arg(long, value_delimiter = ',')]
        proposers: Vec<String>,
        #[arg(long, value_delimiter = ',')]
        approvers: Vec<String>,
        #[arg(long)]
        threshold: u8,
        #[arg(long, default_value = "1")]
        cancellation_threshold: u8,
        #[arg(long, default_value = "0")]
        timelock: u32,
        /// Message expiry (YYYY-MM-DD HH:MM:SS). Defaults to now + configured expiry_seconds.
        #[arg(long)]
        expiry: Option<String>,
    },
    /// List all intents on a wallet
    List {
        #[arg(long)]
        wallet: String,
    },
}

pub fn handle(action: IntentAction, config: &RuntimeConfig) -> Result<()> {
    match action {
        IntentAction::Add {
            wallet: wallet_name,
            file,
            proposers,
            approvers,
            threshold,
            cancellation_threshold,
            timelock,
            expiry,
        } => {
            let expiry_ts = message::resolve_expiry(&expiry, config)?;
            let program_id = crate::instructions::program_id();
            let pid = solana_address::Address::new_from_array(program_id.to_bytes());

            let (wallet_addr, _) =
                clear_wallet_client::pda::find_wallet_address(&wallet_name, &pid);
            let wallet_pubkey = Pubkey::new_from_array(wallet_addr.to_bytes());

            let client = rpc::client(config);
            let wallet_data = rpc::fetch_account(&client, &wallet_pubkey)?;
            let wallet_account = accounts::parse_wallet(&wallet_data)?;

            // Load the transaction JSON and inject governance from flags
            let json_str = std::fs::read_to_string(&file)
                .with_context(|| format!("reading intent file: {file}"))?;
            let tx_json: IntentTransactionJson = serde_json::from_str(&json_str)
                .with_context(|| "parsing intent transaction JSON")?;
            let full_json = tx_json.with_governance(
                proposers,
                approvers,
                threshold,
                cancellation_threshold,
                timelock,
            );
            let built = full_json.to_built().map_err(|e| anyhow!("{e}"))?;

            // The next intent index is wallet.intent_index + 1
            let next_index = wallet_account.intent_index + 1;
            let intent_body = built.serialize_body(
                &wallet_addr,
                0, // bump will be computed on-chain
                next_index,
                3, // Custom intent type
            );

            // params_data for AddIntent = the serialized intent body
            let params_data = intent_body;

            // Fetch the AddIntent (index 0) to build message
            let (add_intent_addr, _) =
                clear_wallet_client::pda::find_intent_address(&wallet_addr, 0, &pid);
            let add_intent_pubkey = Pubkey::new_from_array(add_intent_addr.to_bytes());
            let intent_data = rpc::fetch_account(&client, &add_intent_pubkey)?;
            let intent_account = accounts::parse_intent(&intent_data)?;

            let proposal_index = wallet_account.proposal_index;
            let msg = message::build_message(
                "propose",
                expiry_ts,
                &wallet_account.name,
                proposal_index,
                &intent_account,
                &params_data,
            )?;

            eprintln!("Signing message:\n{}", String::from_utf8_lossy(&msg[20..]));
            let signature = config.signer.sign_message(&msg)?;
            let proposer_pubkey = config.signer.pubkey();

            let (proposal_addr, _) = clear_wallet_client::pda::find_proposal_address(
                &add_intent_addr,
                proposal_index,
                &pid,
            );

            let payer_pubkey = solana_signer::Signer::pubkey(&config.payer);
            let ix = crate::instructions::propose(crate::instructions::ProposeArgs {
                payer: payer_pubkey,
                wallet: wallet_pubkey,
                intent: add_intent_pubkey,
                proposal: Pubkey::new_from_array(proposal_addr.to_bytes()),
                proposal_index,
                expiry: expiry_ts,
                proposer_pubkey,
                signature,
                params_data: &params_data,
            });

            let sig = rpc::send_instruction(&client, config, ix)?;

            print_json(&serde_json::json!({
                "txid": sig.to_string(),
                "proposal": Pubkey::new_from_array(proposal_addr.to_bytes()).to_string(),
                "proposal_index": proposal_index,
                "intent_index": 0,
                "action": "add_intent",
            }));
        }
        IntentAction::Remove {
            wallet: wallet_name,
            index,
            expiry,
        } => {
            let expiry_ts = message::resolve_expiry(&expiry, config)?;
            let program_id = crate::instructions::program_id();
            let pid = solana_address::Address::new_from_array(program_id.to_bytes());

            let (wallet_addr, _) =
                clear_wallet_client::pda::find_wallet_address(&wallet_name, &pid);
            let wallet_pubkey = Pubkey::new_from_array(wallet_addr.to_bytes());

            let client = rpc::client(config);
            let wallet_data = rpc::fetch_account(&client, &wallet_pubkey)?;
            let wallet_account = accounts::parse_wallet(&wallet_data)?;

            // RemoveIntent is at index 1; params_data = [target_index]
            let params_data = vec![index];

            let (remove_intent_addr, _) =
                clear_wallet_client::pda::find_intent_address(&wallet_addr, 1, &pid);
            let remove_intent_pubkey = Pubkey::new_from_array(remove_intent_addr.to_bytes());
            let intent_data = rpc::fetch_account(&client, &remove_intent_pubkey)?;
            let intent_account = accounts::parse_intent(&intent_data)?;

            let proposal_index = wallet_account.proposal_index;
            let msg = message::build_message(
                "propose",
                expiry_ts,
                &wallet_account.name,
                proposal_index,
                &intent_account,
                &params_data,
            )?;

            eprintln!("Signing message:\n{}", String::from_utf8_lossy(&msg[20..]));
            let signature = config.signer.sign_message(&msg)?;
            let proposer_pubkey = config.signer.pubkey();

            let (proposal_addr, _) = clear_wallet_client::pda::find_proposal_address(
                &remove_intent_addr,
                proposal_index,
                &pid,
            );

            let payer_pubkey = solana_signer::Signer::pubkey(&config.payer);
            let ix = crate::instructions::propose(crate::instructions::ProposeArgs {
                payer: payer_pubkey,
                wallet: wallet_pubkey,
                intent: remove_intent_pubkey,
                proposal: Pubkey::new_from_array(proposal_addr.to_bytes()),
                proposal_index,
                expiry: expiry_ts,
                proposer_pubkey,
                signature,
                params_data: &params_data,
            });

            let sig = rpc::send_instruction(&client, config, ix)?;

            print_json(&serde_json::json!({
                "txid": sig.to_string(),
                "proposal": Pubkey::new_from_array(proposal_addr.to_bytes()).to_string(),
                "proposal_index": proposal_index,
                "intent_index": 1,
                "action": "remove_intent",
                "target_index": index,
            }));
        }
        IntentAction::Update {
            wallet: wallet_name,
            index,
            file,
            proposers,
            approvers,
            threshold,
            cancellation_threshold,
            timelock,
            expiry,
        } => {
            let expiry_ts = message::resolve_expiry(&expiry, config)?;
            let program_id = crate::instructions::program_id();
            let pid = solana_address::Address::new_from_array(program_id.to_bytes());

            let (wallet_addr, _) =
                clear_wallet_client::pda::find_wallet_address(&wallet_name, &pid);
            let wallet_pubkey = Pubkey::new_from_array(wallet_addr.to_bytes());

            let client = rpc::client(config);
            let wallet_data = rpc::fetch_account(&client, &wallet_pubkey)?;
            let wallet_account = accounts::parse_wallet(&wallet_data)?;

            let json_str = std::fs::read_to_string(&file)
                .with_context(|| format!("reading intent file: {file}"))?;
            let tx_json: IntentTransactionJson = serde_json::from_str(&json_str)
                .with_context(|| "parsing intent transaction JSON")?;
            let full_json = tx_json.with_governance(
                proposers,
                approvers,
                threshold,
                cancellation_threshold,
                timelock,
            );
            let built = full_json.to_built().map_err(|e| anyhow!("{e}"))?;

            let intent_body = built.serialize_body(&wallet_addr, 0, index, 3);

            // UpdateIntent params_data = [target_index, ...new_intent_body]
            let mut params_data = vec![index];
            params_data.extend_from_slice(&intent_body);

            let (update_intent_addr, _) =
                clear_wallet_client::pda::find_intent_address(&wallet_addr, 2, &pid);
            let update_intent_pubkey = Pubkey::new_from_array(update_intent_addr.to_bytes());
            let intent_data = rpc::fetch_account(&client, &update_intent_pubkey)?;
            let intent_account = accounts::parse_intent(&intent_data)?;

            let proposal_index = wallet_account.proposal_index;
            let msg = message::build_message(
                "propose",
                expiry_ts,
                &wallet_account.name,
                proposal_index,
                &intent_account,
                &params_data,
            )?;

            eprintln!("Signing message:\n{}", String::from_utf8_lossy(&msg[20..]));
            let signature = config.signer.sign_message(&msg)?;
            let proposer_pubkey = config.signer.pubkey();

            let (proposal_addr, _) = clear_wallet_client::pda::find_proposal_address(
                &update_intent_addr,
                proposal_index,
                &pid,
            );

            let payer_pubkey = solana_signer::Signer::pubkey(&config.payer);
            let ix = crate::instructions::propose(crate::instructions::ProposeArgs {
                payer: payer_pubkey,
                wallet: wallet_pubkey,
                intent: update_intent_pubkey,
                proposal: Pubkey::new_from_array(proposal_addr.to_bytes()),
                proposal_index,
                expiry: expiry_ts,
                proposer_pubkey,
                signature,
                params_data: &params_data,
            });

            let sig = rpc::send_instruction(&client, config, ix)?;

            print_json(&serde_json::json!({
                "txid": sig.to_string(),
                "proposal": Pubkey::new_from_array(proposal_addr.to_bytes()).to_string(),
                "proposal_index": proposal_index,
                "intent_index": 2,
                "action": "update_intent",
                "target_index": index,
            }));
        }
        IntentAction::List {
            wallet: wallet_name,
        } => {
            let program_id = crate::instructions::program_id();
            let pid = solana_address::Address::new_from_array(program_id.to_bytes());

            let (wallet_addr, _) =
                clear_wallet_client::pda::find_wallet_address(&wallet_name, &pid);
            let wallet_pubkey = Pubkey::new_from_array(wallet_addr.to_bytes());

            let client = rpc::client(config);
            let wallet_data = rpc::fetch_account(&client, &wallet_pubkey)?;
            let wallet_account = accounts::parse_wallet(&wallet_data)?;

            let mut intents = Vec::new();
            for i in 0..=wallet_account.intent_index {
                let (intent_addr, _) =
                    clear_wallet_client::pda::find_intent_address(&wallet_addr, i, &pid);
                let intent_pubkey = Pubkey::new_from_array(intent_addr.to_bytes());
                match rpc::fetch_account_optional(&client, &intent_pubkey)? {
                    Some(data) => {
                        let intent = accounts::parse_intent(&data)?;
                        intents.push(serde_json::json!({
                            "index": i,
                            "address": intent_pubkey.to_string(),
                            "type": intent.intent_type_name(),
                            "approved": intent.approved,
                            "approval_threshold": intent.approval_threshold,
                            "cancellation_threshold": intent.cancellation_threshold,
                            "timelock_seconds": intent.timelock_seconds,
                            "template": intent.template(),
                            "proposers": intent.proposers,
                            "approvers": intent.approvers,
                            "active_proposals": intent.active_proposal_count,
                        }));
                    }
                    None => {
                        intents.push(serde_json::json!({
                            "index": i,
                            "status": "not found (possibly removed)",
                        }));
                    }
                }
            }

            print_json(&intents);
        }
    }
    Ok(())
}
