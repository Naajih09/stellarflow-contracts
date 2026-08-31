pub mod events;
pub mod governance;
pub mod liquidity;
pub mod swaps;

pub use events::*;
pub use governance::{publish_proposal_created, ProposalCreatedEvent};
pub use liquidity::{publish_liquidity_added, publish_liquidity_removed, LiquidityAddedEvent, LiquidityRemovedEvent};
pub use swaps::publish_swap_executed, SwapExecutedEvent};

pub mod governance {
    use soroban_sdk::{contracttype, Bytes, Env, Symbol};

    #[contracttype]
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct ProposalCreatedEvent {
        pub proposal_id: u64,
        pub ipfs_cid: Bytes,
    }

    pub fn publish_proposal_created(
        env: &Env,
        proposal_id: u64,
        ipfs_cid: Bytes,
    ) {
        env.events().publish(
            (Symbol::new(env, "ProposalCreated"),),
            ProposalCreatedEvent {
                proposal_id,
                ipfs_cid,
            },
        );
    }
}