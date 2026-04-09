use quasar_lang::prelude::*;

#[account(discriminator = 1, set_inner)]
#[seeds(b"clear_wallet", name_hash: Address)]
pub struct ClearWallet<'a> {
    pub bump: u8,
    pub proposal_index: u64,
    pub intent_index: u8,
    pub name: String<'a, 64>,
}
