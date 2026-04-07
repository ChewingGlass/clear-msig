use crate::config::RuntimeConfig;
use crate::error::*;
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    signature::Signature,
    transaction::Transaction,
    instruction::Instruction,
    pubkey::Pubkey,
    signer::Signer,
};

pub fn client(config: &RuntimeConfig) -> RpcClient {
    RpcClient::new_with_commitment(&config.rpc_url, CommitmentConfig::confirmed())
}

pub fn fetch_account(rpc: &RpcClient, address: &Pubkey) -> Result<Vec<u8>> {
    let account = rpc.get_account(address)
        .with_context(|| format!("fetching account {address}"))?;
    Ok(account.data)
}

pub fn fetch_account_optional(rpc: &RpcClient, address: &Pubkey) -> Result<Option<Vec<u8>>> {
    match rpc.get_account(address) {
        Ok(account) => Ok(Some(account.data)),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("AccountNotFound") || msg.contains("could not find account") {
                Ok(None)
            } else {
                Err(e).with_context(|| format!("fetching account {address}"))
            }
        }
    }
}

pub fn send_instruction(
    rpc: &RpcClient,
    config: &RuntimeConfig,
    instruction: Instruction,
) -> Result<Signature> {
    let recent_blockhash = rpc.get_latest_blockhash()?;
    let transaction = Transaction::new_signed_with_payer(
        &[instruction],
        Some(&config.payer.pubkey()),
        &[&config.payer],
        recent_blockhash,
    );
    let signature = rpc.send_and_confirm_transaction(&transaction)
        .with_context(|| "sending transaction")?;
    Ok(signature)
}

#[allow(dead_code)]
pub fn send_instructions(
    rpc: &RpcClient,
    config: &RuntimeConfig,
    instructions: Vec<Instruction>,
) -> Result<Signature> {
    let recent_blockhash = rpc.get_latest_blockhash()?;
    let transaction = Transaction::new_signed_with_payer(
        &instructions,
        Some(&config.payer.pubkey()),
        &[&config.payer],
        recent_blockhash,
    );
    let signature = rpc.send_and_confirm_transaction(&transaction)
        .with_context(|| "sending transaction")?;
    Ok(signature)
}
