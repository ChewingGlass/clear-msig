use quasar_lang::prelude::*;

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProposalStatus {
    Active = 0,
    Approved = 1,
    Executed = 2,
    Cancelled = 3,
}

/// Votes are tracked as a bitmap over the intent's approver list.
/// Each bit position corresponds to an approver index in the intent.
#[account(discriminator = 3, set_inner)]
#[seeds(b"proposal", intent: Address, proposal_index: u64)]
pub struct Proposal<'a> {
    pub wallet: Address,
    pub intent: Address,
    pub proposal_index: u64,
    pub proposer: Address,
    pub status: ProposalStatus,
    pub proposed_at: i64,
    pub approved_at: i64,
    pub bump: u8,
    pub approval_bitmap: u16,
    pub cancellation_bitmap: u16,
    pub rent_refund: Address,
    pub params_data: Vec<'a, u8, 4096>,
}

impl Proposal<'_> {
    pub fn approval_count(&self) -> u8 {
        self.approval_bitmap.get().count_ones() as u8
    }

    pub fn cancellation_count(&self) -> u8 {
        self.cancellation_bitmap.get().count_ones() as u8
    }

    pub fn has_approved_by_index(&self, idx: u8) -> bool {
        self.approval_bitmap.get() & (1 << idx) != 0
    }

    pub fn has_cancelled_by_index(&self, idx: u8) -> bool {
        self.cancellation_bitmap.get() & (1 << idx) != 0
    }

    pub fn set_approval(&mut self, idx: u8) {
        let mask = 1u16 << idx;
        self.cancellation_bitmap = PodU16::from(self.cancellation_bitmap.get() & !mask);
        self.approval_bitmap = PodU16::from(self.approval_bitmap.get() | mask);
    }

    pub fn set_cancellation(&mut self, idx: u8) {
        let mask = 1u16 << idx;
        self.approval_bitmap = PodU16::from(self.approval_bitmap.get() & !mask);
        self.cancellation_bitmap = PodU16::from(self.cancellation_bitmap.get() | mask);
    }
}
