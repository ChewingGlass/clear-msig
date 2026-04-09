use clear_wallet::clear_wallet::cpi;
use quasar_lang::client::{DynBytes, DynVec, TailBytes};
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;

/// The clear-wallet program ID.
pub fn program_id() -> Pubkey {
    let addr = clear_wallet_client::ID;
    Pubkey::new_from_array(addr.to_bytes())
}

/// Convert a quasar-generated v3 Instruction to a v2 Instruction.
/// Both have identical wire formats (program_id + accounts + data),
/// just different Rust types due to the v2/v3 crate split.
fn from_v3(ix: solana_instruction_v3::Instruction) -> Instruction {
    Instruction {
        program_id: Pubkey::new_from_array(ix.program_id.to_bytes()),
        accounts: ix.accounts.into_iter().map(|a| {
            AccountMeta {
                pubkey: Pubkey::new_from_array(a.pubkey.to_bytes()),
                is_signer: a.is_signer,
                is_writable: a.is_writable,
            }
        }).collect(),
        data: ix.data,
    }
}

/// Type alias for the v4 Pubkey used by quasar-generated structs (via solana-instruction v3).
type V3Pubkey = solana_pubkey_v4::Pubkey;

fn to_v3(p: &Pubkey) -> V3Pubkey {
    V3Pubkey::from(p.to_bytes())
}

pub struct CreateWalletArgs<'a> {
    pub payer: Pubkey,
    pub name_hash: Pubkey,
    pub wallet: Pubkey,
    pub add_intent: Pubkey,
    pub remove_intent: Pubkey,
    pub update_intent: Pubkey,
    pub name: &'a str,
    pub threshold: u8,
    pub cancel_threshold: u8,
    pub timelock: u32,
    pub proposers: &'a [Pubkey],
    pub approvers: &'a [Pubkey],
}

pub fn create_wallet(args: CreateWalletArgs<'_>) -> Instruction {
    from_v3(cpi::CreateWalletInstruction {
        payer: to_v3(&args.payer),
        name_hash: to_v3(&args.name_hash),
        wallet: to_v3(&args.wallet),
        add_intent: to_v3(&args.add_intent),
        remove_intent: to_v3(&args.remove_intent),
        update_intent: to_v3(&args.update_intent),
        system_program: to_v3(&solana_sdk_ids::system_program::ID),
        approval_threshold: args.threshold,
        cancellation_threshold: args.cancel_threshold,
        timelock_seconds: args.timelock,
        name: DynBytes::new(args.name.as_bytes().to_vec()),
        proposers: DynVec::new(args.proposers.iter().map(|p| p.to_bytes()).collect()),
        approvers: DynVec::new(args.approvers.iter().map(|a| a.to_bytes()).collect()),
    }.into())
}

pub struct ProposeArgs<'a> {
    pub payer: Pubkey,
    pub wallet: Pubkey,
    pub intent: Pubkey,
    pub proposal: Pubkey,
    pub proposal_index: u64,
    pub expiry: i64,
    pub proposer_pubkey: [u8; 32],
    pub signature: [u8; 64],
    pub params_data: &'a [u8],
}

pub fn propose(args: ProposeArgs<'_>) -> Instruction {
    from_v3(cpi::ProposeInstruction {
        payer: to_v3(&args.payer),
        wallet: to_v3(&args.wallet),
        intent: to_v3(&args.intent),
        proposal: to_v3(&args.proposal),
        system_program: to_v3(&solana_sdk_ids::system_program::ID),
        proposal_index: args.proposal_index,
        expiry: args.expiry,
        proposer_pubkey: args.proposer_pubkey,
        signature: args.signature,
        params_data: TailBytes(args.params_data.to_vec()),
    }.into())
}

pub fn approve(
    wallet: Pubkey, intent: Pubkey, proposal: Pubkey,
    expiry: i64, approver_index: u8, signature: [u8; 64],
) -> Instruction {
    from_v3(cpi::ApproveInstruction {
        wallet: to_v3(&wallet),
        intent: to_v3(&intent),
        proposal: to_v3(&proposal),
        expiry, approver_index, signature,
    }.into())
}

pub fn cancel(
    wallet: Pubkey, intent: Pubkey, proposal: Pubkey,
    expiry: i64, canceller_index: u8, signature: [u8; 64],
) -> Instruction {
    from_v3(cpi::CancelInstruction {
        wallet: to_v3(&wallet),
        intent: to_v3(&intent),
        proposal: to_v3(&proposal),
        expiry, canceller_index, signature,
    }.into())
}

pub fn execute(
    wallet: Pubkey, vault: Pubkey, intent: Pubkey, proposal: Pubkey,
    remaining_accounts: Vec<AccountMeta>,
) -> Instruction {
    // Convert v2 AccountMetas to v3 for the generated struct
    let v3_remaining: Vec<solana_instruction_v3::AccountMeta> = remaining_accounts
        .into_iter()
        .map(|a| solana_instruction_v3::AccountMeta {
            pubkey: V3Pubkey::from(a.pubkey.to_bytes()),
            is_signer: a.is_signer,
            is_writable: a.is_writable,
        })
        .collect();

    from_v3(cpi::ExecuteInstruction {
        wallet: to_v3(&wallet),
        vault: to_v3(&vault),
        intent: to_v3(&intent),
        proposal: to_v3(&proposal),
        system_program: to_v3(&solana_sdk_ids::system_program::ID),
        remaining_accounts: v3_remaining,
    }.into())
}

pub fn cleanup(proposal: Pubkey, rent_refund: Pubkey) -> Instruction {
    from_v3(cpi::CleanupProposalInstruction {
        proposal: to_v3(&proposal),
        rent_refund: to_v3(&rent_refund),
    }.into())
}
