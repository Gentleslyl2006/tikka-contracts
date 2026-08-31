use soroban_sdk::{contractevent, Address};

#[derive(Clone)]
#[contractevent]
pub struct ContractPaused {
    pub paused_by: Address,
    pub timestamp: u64,
}

#[derive(Clone)]
#[contractevent]
pub struct ContractUnpaused {
    pub unpaused_by: Address,
    pub timestamp: u64,
}
