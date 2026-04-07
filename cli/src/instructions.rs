use solana_sdk::{instruction::{AccountMeta, Instruction}, pubkey::Pubkey};

/// The clear-wallet program ID.
pub fn program_id() -> Pubkey {
    // C1earWa11etMSig1111111111111111111111111111
    let addr = clear_wallet_client::ID;
    Pubkey::new_from_array(addr.to_bytes())
}

/// Build create_wallet instruction.
/// Wire: [0] ++ threshold:u8 ++ cancel_threshold:u8 ++ timelock:u32_le ++ num_proposers:u8 ++ name_len:u32_le ++ name_bytes
pub fn create_wallet(
    payer: Pubkey, name_hash: Pubkey, wallet: Pubkey,
    add_intent: Pubkey, remove_intent: Pubkey, update_intent: Pubkey,
    name: &str, threshold: u8, cancel_threshold: u8, timelock: u32,
    proposers: &[Pubkey], approvers: &[Pubkey],
) -> Instruction {
    let mut accounts = vec![
        AccountMeta::new(payer, true),
        AccountMeta::new_readonly(name_hash, false),
        AccountMeta::new(wallet, false),
        AccountMeta::new(add_intent, false),
        AccountMeta::new(remove_intent, false),
        AccountMeta::new(update_intent, false),
        AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
    ];
    for p in proposers { accounts.push(AccountMeta::new_readonly(*p, false)); }
    for a in approvers { accounts.push(AccountMeta::new_readonly(*a, false)); }

    let mut data = vec![0u8]; // discriminator
    data.push(threshold);
    data.push(cancel_threshold);
    data.extend_from_slice(&timelock.to_le_bytes());
    data.push(proposers.len() as u8);
    data.extend_from_slice(&(name.len() as u32).to_le_bytes());
    data.extend_from_slice(name.as_bytes());

    Instruction { program_id: program_id(), accounts, data }
}

/// Build propose instruction.
/// Wire: [1] ++ expiry:i64_le ++ proposer_pubkey:[u8;32] ++ signature:[u8;64] ++ params_data (tail)
pub fn propose(
    payer: Pubkey, wallet: Pubkey, intent: Pubkey, proposal: Pubkey,
    expiry: i64, proposer_pubkey: [u8; 32], signature: [u8; 64],
    params_data: &[u8],
) -> Instruction {
    let accounts = vec![
        AccountMeta::new(payer, true),
        AccountMeta::new(wallet, false),
        AccountMeta::new(intent, false),
        AccountMeta::new(proposal, false),
        AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
    ];

    let mut data = vec![1u8];
    data.extend_from_slice(&expiry.to_le_bytes());
    data.extend_from_slice(&proposer_pubkey);
    data.extend_from_slice(&signature);
    data.extend_from_slice(params_data); // tail bytes

    Instruction { program_id: program_id(), accounts, data }
}

/// Build approve instruction.
/// Wire: [2] ++ expiry:i64_le ++ approver_index:u8 ++ signature:[u8;64]
pub fn approve(
    wallet: Pubkey, intent: Pubkey, proposal: Pubkey,
    expiry: i64, approver_index: u8, signature: [u8; 64],
) -> Instruction {
    let accounts = vec![
        AccountMeta::new_readonly(wallet, false),
        AccountMeta::new(intent, false),
        AccountMeta::new(proposal, false),
    ];

    let mut data = vec![2u8];
    data.extend_from_slice(&expiry.to_le_bytes());
    data.push(approver_index);
    data.extend_from_slice(&signature);

    Instruction { program_id: program_id(), accounts, data }
}

/// Build cancel instruction.
pub fn cancel(
    wallet: Pubkey, intent: Pubkey, proposal: Pubkey,
    expiry: i64, canceller_index: u8, signature: [u8; 64],
) -> Instruction {
    let accounts = vec![
        AccountMeta::new_readonly(wallet, false),
        AccountMeta::new(intent, false),
        AccountMeta::new(proposal, false),
    ];

    let mut data = vec![3u8];
    data.extend_from_slice(&expiry.to_le_bytes());
    data.push(canceller_index);
    data.extend_from_slice(&signature);

    Instruction { program_id: program_id(), accounts, data }
}

/// Build execute instruction.
pub fn execute(
    wallet: Pubkey, vault: Pubkey, intent: Pubkey, proposal: Pubkey,
    remaining_accounts: Vec<AccountMeta>,
) -> Instruction {
    let mut accounts = vec![
        AccountMeta::new(wallet, false),
        AccountMeta::new(vault, false),
        AccountMeta::new(intent, false),
        AccountMeta::new(proposal, false),
        AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
    ];
    accounts.extend(remaining_accounts);

    Instruction { program_id: program_id(), accounts, data: vec![4u8] }
}

/// Build cleanup_proposal instruction.
pub fn cleanup(proposal: Pubkey, rent_refund: Pubkey) -> Instruction {
    let accounts = vec![
        AccountMeta::new(proposal, false),
        AccountMeta::new(rent_refund, false),
    ];
    Instruction { program_id: program_id(), accounts, data: vec![5u8] }
}
