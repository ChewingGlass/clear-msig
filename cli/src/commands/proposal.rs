use crate::config::RuntimeConfig;
use crate::error::*;
use crate::output::print_json;
use crate::{accounts, message, params, resolve, rpc};
use clap::Subcommand;
use solana_pubkey::Pubkey;

#[derive(Subcommand)]
pub enum ProposalAction {
    /// Create a new proposal for a custom intent
    Create {
        #[arg(long)]
        wallet: String,
        /// Intent index to propose against
        #[arg(long)]
        intent_index: u8,
        /// Parameters as key=value pairs
        #[arg(long = "param")]
        params: Vec<String>,
        /// Message expiry (YYYY-MM-DD HH:MM:SS). Defaults to now + configured expiry_seconds.
        #[arg(long)]
        expiry: Option<String>,
    },
    /// Approve an existing proposal
    Approve {
        #[arg(long)]
        wallet: String,
        /// Proposal account address
        #[arg(long)]
        proposal: String,
        /// Message expiry (YYYY-MM-DD HH:MM:SS). Defaults to now + configured expiry_seconds.
        #[arg(long)]
        expiry: Option<String>,
    },
    /// Cancel / reject a proposal
    Cancel {
        #[arg(long)]
        wallet: String,
        #[arg(long)]
        proposal: String,
        /// Message expiry (YYYY-MM-DD HH:MM:SS). Defaults to now + configured expiry_seconds.
        #[arg(long)]
        expiry: Option<String>,
    },
    /// Execute an approved proposal
    Execute {
        #[arg(long)]
        wallet: String,
        #[arg(long)]
        proposal: String,
    },
    /// List proposals for a wallet
    List {
        #[arg(long)]
        wallet: String,
    },
    /// Show details of a specific proposal
    Show {
        /// Proposal account address
        #[arg(long)]
        proposal: String,
    },
    /// Close an executed/cancelled proposal and reclaim rent
    Cleanup {
        #[arg(long)]
        proposal: String,
    },
}

pub fn handle(action: ProposalAction, config: &RuntimeConfig) -> Result<()> {
    match action {
        ProposalAction::Create {
            wallet: wallet_name,
            intent_index,
            params: raw_params,
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

            let (intent_addr, _) = clear_wallet_client::pda::find_intent_address(
                &wallet_addr,
                intent_index,
                &pid,
            );
            let intent_pubkey = Pubkey::new_from_array(intent_addr.to_bytes());
            let intent_data = rpc::fetch_account(&client, &intent_pubkey)?;
            let intent_account = accounts::parse_intent(&intent_data)?;

            if !intent_account.approved {
                return Err(anyhow!("intent {} is not approved", intent_index));
            }

            // Check signer is a proposer
            let signer_pubkey_b58 = bs58::encode(config.signer.pubkey()).into_string();
            if !intent_account.proposers.contains(&signer_pubkey_b58) {
                return Err(anyhow!(
                    "signer {} is not a proposer on intent {}",
                    signer_pubkey_b58,
                    intent_index
                ));
            }

            let params_data = params::encode_params(&intent_account, &raw_params)?;

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
                &intent_addr,
                proposal_index,
                &pid,
            );

            let payer_pubkey = solana_signer::Signer::pubkey(&config.payer);
            let ix = crate::instructions::propose(crate::instructions::ProposeArgs {
                payer: payer_pubkey,
                wallet: wallet_pubkey,
                intent: intent_pubkey,
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
            }));
        }

        ProposalAction::Approve {
            wallet: wallet_name,
            proposal: proposal_addr_str,
            expiry,
        } => {
            approve_or_cancel(config, &wallet_name, &proposal_addr_str, &expiry, true)?;
        }

        ProposalAction::Cancel {
            wallet: wallet_name,
            proposal: proposal_addr_str,
            expiry,
        } => {
            approve_or_cancel(config, &wallet_name, &proposal_addr_str, &expiry, false)?;
        }

        ProposalAction::Execute {
            wallet: wallet_name,
            proposal: proposal_addr_str,
        } => {
            let program_id = crate::instructions::program_id();
            let pid = solana_address::Address::new_from_array(program_id.to_bytes());

            let (wallet_addr, _) =
                clear_wallet_client::pda::find_wallet_address(&wallet_name, &pid);
            let wallet_pubkey = Pubkey::new_from_array(wallet_addr.to_bytes());

            let (vault_addr, _) =
                clear_wallet_client::pda::find_vault_address(&wallet_addr, &pid);
            let vault_pubkey = Pubkey::new_from_array(vault_addr.to_bytes());

            let proposal_pubkey: Pubkey = proposal_addr_str
                .parse()
                .with_context(|| "invalid proposal address")?;

            let client = rpc::client(config);
            let proposal_data = rpc::fetch_account(&client, &proposal_pubkey)?;
            let proposal_account = accounts::parse_proposal(&proposal_data)?;

            if proposal_account.status != "Approved" {
                return Err(anyhow!(
                    "proposal status is '{}', must be 'Approved' to execute",
                    proposal_account.status
                ));
            }

            let intent_pubkey: Pubkey = proposal_account
                .intent
                .parse()
                .with_context(|| "invalid intent address in proposal")?;
            let intent_data = rpc::fetch_account(&client, &intent_pubkey)?;
            let intent_account = accounts::parse_intent(&intent_data)?;

            let payer_pubkey = solana_signer::Signer::pubkey(&config.payer);
            let remaining = resolve::resolve_remaining_accounts(
                &client,
                &intent_account,
                &wallet_pubkey,
                &vault_pubkey,
                &proposal_account.params_data,
                &payer_pubkey,
            )?;

            let ix = crate::instructions::execute(
                wallet_pubkey,
                vault_pubkey,
                intent_pubkey,
                proposal_pubkey,
                remaining,
            );

            let sig = rpc::send_instruction(&client, config, ix)?;

            print_json(&serde_json::json!({
                "txid": sig.to_string(),
                "status": "executed",
            }));
        }

        ProposalAction::List {
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

            // Iterate all intents, then all proposals for each
            let mut proposals = Vec::new();
            for intent_idx in 0..=wallet_account.intent_index {
                let (intent_addr, _) = clear_wallet_client::pda::find_intent_address(
                    &wallet_addr,
                    intent_idx,
                    &pid,
                );

                // Try fetching proposals for this intent
                // We don't know the exact count, so scan from 0 up to wallet.proposal_index
                for prop_idx in 0..wallet_account.proposal_index {
                    let (proposal_addr, _) = clear_wallet_client::pda::find_proposal_address(
                        &intent_addr,
                        prop_idx,
                        &pid,
                    );
                    let proposal_pubkey = Pubkey::new_from_array(proposal_addr.to_bytes());
                    if let Some(data) = rpc::fetch_account_optional(&client, &proposal_pubkey)? {
                        if let Ok(p) = accounts::parse_proposal(&data) {
                            proposals.push(serde_json::json!({
                                "address": proposal_pubkey.to_string(),
                                "intent_index": intent_idx,
                                "proposal_index": p.proposal_index,
                                "proposer": p.proposer,
                                "status": p.status,
                                "proposed_at": p.proposed_at,
                                "approved_at": p.approved_at,
                                "approval_bitmap": p.approval_bitmap,
                                "cancellation_bitmap": p.cancellation_bitmap,
                            }));
                        }
                    }
                }
            }

            print_json(&proposals);
        }

        ProposalAction::Show {
            proposal: proposal_addr_str,
        } => {
            let proposal_pubkey: Pubkey = proposal_addr_str
                .parse()
                .with_context(|| "invalid proposal address")?;

            let client = rpc::client(config);
            let data = rpc::fetch_account(&client, &proposal_pubkey)?;
            let proposal = accounts::parse_proposal(&data)?;

            print_json(&serde_json::json!({
                "address": proposal_pubkey.to_string(),
                "wallet": proposal.wallet,
                "intent": proposal.intent,
                "proposal_index": proposal.proposal_index,
                "proposer": proposal.proposer,
                "status": proposal.status,
                "proposed_at": proposal.proposed_at,
                "approved_at": proposal.approved_at,
                "approval_bitmap": proposal.approval_bitmap,
                "cancellation_bitmap": proposal.cancellation_bitmap,
                "rent_refund": proposal.rent_refund,
                "params_data": bs58::encode(&proposal.params_data).into_string(),
            }));
        }

        ProposalAction::Cleanup {
            proposal: proposal_addr_str,
        } => {
            let proposal_pubkey: Pubkey = proposal_addr_str
                .parse()
                .with_context(|| "invalid proposal address")?;

            let client = rpc::client(config);
            let data = rpc::fetch_account(&client, &proposal_pubkey)?;
            let proposal = accounts::parse_proposal(&data)?;
            let rent_refund: Pubkey = proposal.rent_refund
                .parse()
                .with_context(|| "invalid rent_refund address in proposal")?;

            let ix = crate::instructions::cleanup(proposal_pubkey, rent_refund);
            let sig = rpc::send_instruction(&client, config, ix)?;

            print_json(&serde_json::json!({
                "txid": sig.to_string(),
                "status": "cleaned up",
            }));
        }
    }
    Ok(())
}

/// Shared logic for approve and cancel.
fn approve_or_cancel(
    config: &RuntimeConfig,
    wallet_name: &str,
    proposal_addr_str: &str,
    expiry: &Option<String>,
    is_approve: bool,
) -> Result<()> {
    let expiry_ts = message::resolve_expiry(expiry, config)?;
    let program_id = crate::instructions::program_id();
    let pid = solana_address::Address::new_from_array(program_id.to_bytes());

    let (wallet_addr, _) = clear_wallet_client::pda::find_wallet_address(wallet_name, &pid);
    let wallet_pubkey = Pubkey::new_from_array(wallet_addr.to_bytes());

    let client = rpc::client(config);
    let wallet_data = rpc::fetch_account(&client, &wallet_pubkey)?;
    let wallet_account = accounts::parse_wallet(&wallet_data)?;

    let proposal_pubkey: Pubkey = proposal_addr_str
        .parse()
        .with_context(|| "invalid proposal address")?;
    let proposal_data = rpc::fetch_account(&client, &proposal_pubkey)?;
    let proposal_account = accounts::parse_proposal(&proposal_data)?;

    let intent_pubkey: Pubkey = proposal_account
        .intent
        .parse()
        .with_context(|| "invalid intent address in proposal")?;
    let intent_data = rpc::fetch_account(&client, &intent_pubkey)?;
    let intent_account = accounts::parse_intent(&intent_data)?;

    // Find our index in the approvers list
    let signer_pubkey_b58 = bs58::encode(config.signer.pubkey()).into_string();
    let approver_index = intent_account
        .approvers
        .iter()
        .position(|a| a == &signer_pubkey_b58)
        .ok_or(anyhow!(
            "signer {} is not an approver on this intent",
            signer_pubkey_b58
        ))? as u8;

    let action = if is_approve { "approve" } else { "cancel" };
    let msg = message::build_message(
        action,
        expiry_ts,
        &wallet_account.name,
        proposal_account.proposal_index,
        &intent_account,
        &proposal_account.params_data,
    )?;

    eprintln!("Signing message:\n{}", String::from_utf8_lossy(&msg[20..]));
    let signature = config.signer.sign_message(&msg)?;

    let ix = if is_approve {
        crate::instructions::approve(
            wallet_pubkey,
            intent_pubkey,
            proposal_pubkey,
            expiry_ts,
            approver_index,
            signature,
        )
    } else {
        crate::instructions::cancel(
            wallet_pubkey,
            intent_pubkey,
            proposal_pubkey,
            expiry_ts,
            approver_index,
            signature,
        )
    };

    let sig = rpc::send_instruction(&client, config, ix)?;

    print_json(&serde_json::json!({
        "txid": sig.to_string(),
        "action": action,
        "approver_index": approver_index,
    }));

    Ok(())
}
