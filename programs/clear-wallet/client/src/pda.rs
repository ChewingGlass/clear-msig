use sha2::{Digest, Sha256};
use solana_address::Address;

pub fn find_wallet_address(name: &str, program_id: &Address) -> (Address, u8) {
    let name_hash = compute_name_hash(name);
    Address::find_program_address(&[b"clear_wallet", &name_hash], program_id)
}

pub fn find_vault_address(wallet: &Address, program_id: &Address) -> (Address, u8) {
    Address::find_program_address(&[b"vault", wallet.as_ref()], program_id)
}

pub fn find_intent_address(wallet: &Address, index: u8, program_id: &Address) -> (Address, u8) {
    Address::find_program_address(&[b"intent", wallet.as_ref(), &[index]], program_id)
}

pub fn find_proposal_address(intent: &Address, index: u64, program_id: &Address) -> (Address, u8) {
    Address::find_program_address(
        &[b"proposal", intent.as_ref(), &index.to_le_bytes()],
        program_id,
    )
}

pub fn compute_name_hash(name: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(name.as_bytes());
    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}
