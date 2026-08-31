use soroban_sdk::{contracttype, symbol_short, Address, BytesN, Env, Map, Symbol, Vec};
use crate::{ContractData, ContractError, DATA_KEY, SIGNERS_KEY};

const BALLOT_TTL_LEDGERS: u32 = 17_280;
const BALLOT_TTL_THRESHOLD: u32 = 5_000;

pub(crate) const GOVERNANCE_UPGRADE_KEY: Symbol = symbol_short!("GOVUPG");
pub(crate) const GOVERNANCE_CONFIG_KEY: Symbol = symbol_short!("GVNCFG");
pub(crate) const SIGNER_WEIGHTS_KEY: Symbol = symbol_short!("SIGWT");
pub(crate) const QUORUM_WEIGHT_THRESHOLD_KEY: Symbol = symbol_short!("QWTH");
pub(crate) const PROPOSAL_WEIGHT_KEY: Symbol = symbol_short!("PROPWT");

pub(crate) const VALIDATORS_KEY: Symbol = symbol_short!("VALIDS");
pub(crate) const VALIDATOR_SEQUENCE_KEY: Symbol = symbol_short!("VALSEQ");
pub(crate) const BRIDGE_VALIDATORS_UPDATED_EVENT: Symbol = symbol_short!("BridgeValidatorsUpdated");

#[contracttype]
#[derive(Clone)]
pub struct GovernanceConfig {
    pub quorum_threshold: u32,
}

#[contracttype]
#[derive(Clone)]
pub struct MultiSigConfig {
    /// Total weight required for quorum (N in N-of-M)
    pub required_weight: u32,
    /// Maximum weight any single signer can hold
    pub max_signer_weight: u32,
}

impl Default for MultiSigConfig {
    fn default() -> Self {
        Self {
            required_weight: 1,
            max_signer_weight: 1,
        }
    }
}
impl Default for GovernanceConfig {
    fn default() -> Self {
        Self { quorum_threshold: 2 }
    }
}

/// Proposal state enumeration for governance lifecycle management.
///
/// Proposals transition through states as they move through voting, approval,
/// and execution phases. The `Vetoed` state is terminal and prevents execution.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProposalState {
    /// Proposal has been created and is awaiting voting.
    Pending,
    /// Proposal is currently in the voting/discussion phase.
    Active,
    /// Proposal has been approved by the required threshold and awaits execution.
    Approved,
    /// Proposal was rejected during voting (failed to reach threshold).
    Rejected,
    /// Proposal has been executed and is complete.
    Executed,
    /// Proposal was vetoed by the Security Council (terminal state).
    Vetoed,
}

/// Get multi-signature weight configuration for WASM upgrade governance
pub fn get_multisig_config(env: &Env) -> MultiSigConfig {
    env.storage()
        .instance()
        .get(&QUORUM_WEIGHT_THRESHOLD_KEY)
        .unwrap_or_default()
}

/// Set multi-signature weight configuration for WASM upgrade governance
pub fn set_multisig_config(env: &Env, config: &MultiSigConfig) {
    env.storage()
        .instance()
        .set(&QUORUM_WEIGHT_THRESHOLD_KEY, config);
}

/// Get the weight for a specific signer (returns 0 if signer not registered)
pub fn get_signer_weight(env: &Env, signer: &Address) -> u32 {
    let weights: Map<Address, u32> = env
        .storage()
        .instance()
        .get(&SIGNER_WEIGHTS_KEY)
        .unwrap_or_else(|| Map::new(env));
    weights.get(signer.clone()).unwrap_or(0u32)
}

/// Register or update a signer's weight in multi-sig governance
pub fn set_signer_weight(env: &Env, signer: &Address, weight: u32) {
    let mut weights: Map<Address, u32> = env
        .storage()
        .instance()
        .get(&SIGNER_WEIGHTS_KEY)
        .unwrap_or_else(|| Map::new(env));
    if weight == 0 {
        weights.remove(signer.clone());
    } else {
        weights.set(signer.clone(), weight);
    }
    env.storage()
        .instance()
        .set(&SIGNER_WEIGHTS_KEY, &weights);
}
pub fn get_governance_config(env: &Env) -> GovernanceConfig {
    env.storage()
        .instance()
        .get(&GOVERNANCE_CONFIG_KEY)
        .unwrap_or_default()
}

pub fn set_governance_config(env: &Env, config: &GovernanceConfig) {
    env.storage().instance().set(&GOVERNANCE_CONFIG_KEY, config);
}

pub fn verify_upgrade_quorum(env: &Env, signers: &Vec<Address>) -> Result<(), ContractError> {
    let data: ContractData = env
        .storage()
        .instance()
        .get(&DATA_KEY)
        .ok_or(ContractError::NotInitialized)?;

    let authorized_signers: Map<Address, ()> = env
        .storage()
        .instance()
        .get(&SIGNERS_KEY)
        .unwrap_or_else(|| Map::new(env));

    let config = get_governance_config(env);
    let multisig_config = get_multisig_config(env);
    
    // Legacy count-based check
    let mut valid_count: u32 = 0;
    let mut collected_weight: u32 = 0;
    let mut seen_signers: Map<Address, ()> = Map::new(env);
    
    for signer in signers.iter() {
        // Skip duplicate signers
        if seen_signers.contains_key(signer.clone()) {
            continue;
        }
        seen_signers.set(signer.clone(), ());
        
        // Check if signer is authorized (admin or in authorized_signers)
        let is_authorized = signer == data.admin || authorized_signers.contains_key(signer.clone());
        if !is_authorized {
            continue;
        }
        
        valid_count += 1;
        
        // Get weight for this signer (admin gets weight 1 if not explicitly set)
        let weight = if signer == data.admin {
            get_signer_weight(env, &data.admin).max(1u32)
        } else {
            get_signer_weight(env, &signer)
        };
        
        collected_weight = collected_weight.checked_add(weight)
            .ok_or(ContractError::Overflow)?;
    }

    // Fail if count-based quorum not met
    if valid_count < config.quorum_threshold {
        return Err(ContractError::ThresholdNotReached);
    }
    
    // Fail if weight-based quorum not met
    if collected_weight < multisig_config.required_weight {
        return Err(ContractError::ThresholdNotReached);
    }
    
    Ok(())
}

pub fn rotate_admin_keys(
    env: &Env,
    signers: &Vec<Address>,
    new_signers: Vec<Address>,
    new_threshold: u32,
) -> Result<(), ContractError> {
    verify_upgrade_quorum(env, signers)?;

    let mut signer_set: Map<Address, ()> = Map::new(env);
    for signer in new_signers.iter() {
        signer_set.set(signer.clone(), ());
    }

    if new_threshold == 0 || new_threshold > signer_set.len() {
        return Err(ContractError::InvalidThreshold);
    }

    let mut weights: Map<Address, u32> = Map::new(env);
    for signer in new_signers.iter() {
        weights.set(signer.clone(), 1u32);
    }

    let mut data: ContractData = env
        .storage()
        .instance()
        .get(&DATA_KEY)
        .ok_or(ContractError::NotInitialized)?;
    data.admin = new_signers
        .get(0)
        .ok_or(ContractError::InvalidThreshold)?
        .clone();

    env.storage().instance().set(&DATA_KEY, &data);
    env.storage().instance().set(&SIGNERS_KEY, &signer_set);
    env.storage().instance().set(&SIGNER_WEIGHTS_KEY, &weights);

    set_governance_config(env, &GovernanceConfig {
        quorum_threshold: new_threshold,
    });
    set_multisig_config(env, &MultiSigConfig {
        required_weight: new_threshold,
        max_signer_weight: get_multisig_config(env).max_signer_weight.max(1u32),
    });

    env.events().publish(
        (Symbol::new(env, "AdminKeysRotated"),),
        new_signers,
    );

    Ok(())
}

#[contracttype]
#[derive(Clone)]
pub struct StagedUpgrade {
    pub new_wasm_hash: BytesN<32>,
    pub proposer: Address,
    pub staged_at: u64,
    /// Earliest ledger timestamp at which the replacement may execute.
    pub execute_at: u64,
}

#[contracttype]
#[derive(Clone)]
pub struct GovernanceUpgradeProposal {
    pub new_wasm_hash: BytesN<32>,
    pub proposer: Address,
    pub staged_at: u64,
    pub signers: Vec<Address>,
}

/// Event emitted when a governance upgrade is proposed
pub fn calculate_collected_weight(env: &Env, signers: &Vec<Address>, data: &ContractData) -> Result<u32, ContractError> {
    let authorized_signers: Map<Address, ()> = env
        .storage()
        .instance()
        .get(&SIGNERS_KEY)
        .unwrap_or_else(|| Map::new(env));
    
    let mut collected_weight: u32 = 0;
    let mut seen_signers: Map<Address, ()> = Map::new(env);
    
    for signer in signers.iter() {
        // Skip duplicate signers
        if seen_signers.contains_key(signer.clone()) {
            continue;
        }
        seen_signers.set(signer.clone(), ());
        
        // Check if signer is authorized
        let is_authorized = signer == data.admin || authorized_signers.contains_key(signer.clone());
        if !is_authorized {
            continue;
        }
        
        // Get weight for this signer (admin gets weight 1 if not explicitly set)
        let weight = if signer == data.admin {
            get_signer_weight(env, &data.admin).max(1u32)
        } else {
            get_signer_weight(env, &signer)
        };
        
        collected_weight = collected_weight.checked_add(weight)
            .ok_or(ContractError::Overflow)?;
    }
    
    Ok(collected_weight)
}
pub fn get_validator_set(env: &Env) -> Map<BytesN<32>, ()> {
    env.storage()
        .instance()
        .get(&VALIDATORS_KEY)
        .unwrap_or_else(|| Map::new(env))
}

pub fn get_validator_sequence(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&VALIDATOR_SEQUENCE_KEY)
        .unwrap_or(0u64)
}

pub fn rotate_validators(
    env: &Env,
    signers: &Vec<Address>,
    new_validators: Vec<BytesN<32>>,
) -> Result<u64, ContractError> {
    verify_upgrade_quorum(env, signers)?;

    let mut validator_set: Map<BytesN<32>, ()> = Map::new(env);
    for validator in new_validators.iter() {
        validator_set.set(validator.clone(), ());
    }

    let sequence = get_validator_sequence(env)
        .checked_add(1)
        .ok_or(ContractError::Overflow)?;

    env.storage().instance().set(&VALIDATORS_KEY, &validator_set);
    env.storage().instance().set(&VALIDATOR_SEQUENCE_KEY, &sequence);
    env.events().publish(
        (BRIDGE_VALIDATORS_UPDATED_EVENT, sequence),
        new_validators,
    );

    Ok(sequence)
}

#[contracttype]
#[derive(Clone)]
pub struct GovernanceUpgradeProposedEvent {
    pub new_wasm_hash: BytesN<32>,
    pub proposer: Address,
    pub signers: Vec<Address>,
    pub staged_at: u64,
    pub required_weight: u32,
    pub collected_weight: u32,
}
pub fn verify_staged_delay(staged_at: u64, current_time: u64, delay_seconds: u64) -> bool {
    current_time.saturating_sub(staged_at) >= delay_seconds
}

#[contracttype]
pub enum BallotKey {
    Proposal(Symbol),
}

#[contracttype]
#[derive(Clone)]
pub struct VotingBallot {
    pub target: Address,
    pub replacement: Address,
    pub proposer: Address,
    pub proposed_at: u64,
    pub votes: Map<Address, ()>,
}

pub fn open_ballot(
    env: &Env,
    proposal_id: Symbol,
    target: Address,
    replacement: Address,
    proposer: Address,
) -> Result<(), ContractError> {
    let key = BallotKey::Proposal(proposal_id);
    if env.storage().temporary().has(&key) {
        return Err(ContractError::ProposalAlreadyActive);
    }
    let ballot = VotingBallot {
        target,
        replacement,
        proposer,
        proposed_at: env.ledger().timestamp(),
        votes: Map::new(env),
    };
    env.storage().temporary().set(&key, &ballot);
    env.storage().temporary().extend_ttl(&key, BALLOT_TTL_THRESHOLD, BALLOT_TTL_LEDGERS);
    crate::instance::bump_instance_ttl(env);
    Ok(())
}

pub fn cast_vote(
    env: &Env,
    proposal_id: Symbol,
    voter: Address,
) -> Result<VotingBallot, ContractError> {
    let key = BallotKey::Proposal(proposal_id);
    let mut ballot: VotingBallot = env
        .storage()
        .temporary()
        .get(&key)
        .ok_or(ContractError::NoActiveProposal)?;
    if ballot.votes.contains_key(voter.clone()) {
        return Err(ContractError::AlreadyVoted);
    }
    ballot.votes.set(voter, ());
    env.storage().temporary().set(&key, &ballot);
    env.storage().temporary().extend_ttl(&key, BALLOT_TTL_THRESHOLD, BALLOT_TTL_LEDGERS);
    crate::instance::bump_instance_ttl(env);
    Ok(ballot)
}

pub fn get_ballot(env: &Env, proposal_id: Symbol) -> Option<VotingBallot> {
    env.storage().temporary().get(&BallotKey::Proposal(proposal_id))
}

pub fn close_ballot(env: &Env, proposal_id: Symbol) {
    env.storage().temporary().remove(&BallotKey::Proposal(proposal_id));
    crate::instance::bump_instance_ttl(env);
}

pub fn verify_block_height(target_height: u32, active_index: u32) -> bool {
    target_height > active_index
}

pub(crate) const FEE_TIER_KEY: Symbol = symbol_short!("FEETIER");
pub(crate) const FEE_SPLIT_KEY: Symbol = symbol_short!("FEESPLIT");
pub(crate) const TREASURY_KEY: Symbol = symbol_short!("TREASURY");

pub const LOW_FEE_TIER_BPS: u32 = 5;
pub const MEDIUM_FEE_TIER_BPS: u32 = 30;
pub const HIGH_FEE_TIER_BPS: u32 = 100;
pub const DEFAULT_FEE_TIER_BPS: u32 = MEDIUM_FEE_TIER_BPS;
pub const LP_FEE_SHARE_BPS: u32 = 8000;
pub const TREASURY_FEE_SHARE_BPS: u32 = 2000;

#[contracttype]
#[derive(Clone)]
pub struct FeeTierConfig {
    pub fee_tier_bps: u32,
    pub low_fee_tier_bps: u32,
    pub medium_fee_tier_bps: u32,
    pub high_fee_tier_bps: u32,
}

impl Default for FeeTierConfig {
    fn default() -> Self {
        Self {
            fee_tier_bps: DEFAULT_FEE_TIER_BPS,
            low_fee_tier_bps: LOW_FEE_TIER_BPS,
            medium_fee_tier_bps: MEDIUM_FEE_TIER_BPS,
            high_fee_tier_bps: HIGH_FEE_TIER_BPS,
        }
    }
}

#[contracttype]
#[derive(Clone)]
pub struct FeeSplitConfig {
    pub lp_share_bps: u32,
    pub treasury_share_bps: u32,
}

impl Default for FeeSplitConfig {
    fn default() -> Self {
        Self {
            lp_share_bps: LP_FEE_SHARE_BPS,
            treasury_share_bps: TREASURY_FEE_SHARE_BPS,
        }
    }
}

pub fn get_fee_tier_config(env: &Env) -> FeeTierConfig {
    env.storage()
        .instance()
        .get(&FEE_TIER_KEY)
        .unwrap_or_default()
}

pub fn get_fee_tier(env: &Env) -> u32 {
    get_fee_tier_config(env).fee_tier_bps
}

pub fn set_fee_tier(
    env: &Env,
    signers: &Vec<Address>,
    new_fee_tier_bps: u32,
) -> Result<(), ContractError> {
    verify_upgrade_quorum(env, signers)?;
    let config = get_fee_tier_config(env);
    if new_fee_tier_bps != config.low_fee_tier_bps
        && new_fee_tier_bps != config.medium_fee_tier_bps
        && new_fee_tier_bps != config.high_fee_tier_bps
    {
        return Err(ContractError::InvalidThreshold);
    }
    env.storage().instance().set(&FEE_TIER_KEY, &FeeTierConfig {
        fee_tier_bps: new_fee_tier_bps,
        ..config
    });
    Ok(())
}

pub fn get_fee_split_config(env: &Env) -> FeeSplitConfig {
    env.storage()
        .instance()
        .get(&FEE_SPLIT_KEY)
        .unwrap_or_default()
}

pub fn set_fee_split_config(
    env: &Env,
    signers: &Vec<Address>,
    lp_share_bps: u32,
    treasury_share_bps: u32,
) -> Result<(), ContractError> {
    verify_upgrade_quorum(env, signers)?;
    if lp_share_bps.checked_add(treasury_share_bps) != Some(10000) {
        return Err(ContractError::InvalidThreshold);
    }
    env.storage().instance().set(&FEE_SPLIT_KEY, &FeeSplitConfig {
        lp_share_bps,
        treasury_share_bps,
    });
    Ok(())
}

pub fn split_collected_fees(env: &Env, amount: u128) -> Result<(u128, u128), ContractError> {
    let config = get_fee_split_config(env);
    let lp_amount = amount
        .checked_mul(config.lp_share_bps as u128)
        .ok_or(ContractError::Overflow)? / 10000;
    let treasury_amount = amount
        .checked_mul(config.treasury_share_bps as u128)
        .ok_or(ContractError::Overflow)? / 10000;
    Ok((lp_amount, treasury_amount))
}

pub fn get_treasury_vault(env: &Env) -> Option<Address> {
    env.storage().instance().get(&TREASURY_KEY)
}

pub fn set_treasury_vault(
    env: &Env,
    signers: &Vec<Address>,
    vault: Address,
) -> Result<(), ContractError> {
    verify_upgrade_quorum(env, signers)?;
    env.storage().instance().set(&TREASURY_KEY, &vault);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verify_block_height() {
        assert!(verify_block_height(101, 100));
        assert!(!verify_block_height(100, 100));
        assert!(!verify_block_height(99, 100));
    }
}
