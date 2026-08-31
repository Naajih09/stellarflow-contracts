#![no_std]

use soroban_sdk::{contract, contractimpl, contracterror, Address, Env, Symbol, symbol_short};
use soroban_token_sdk::TokenClient;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum AmmError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    InvalidDepositRatio = 3,
    SlippageExceeded = 4,
    ZeroDeposit = 5,
    PoolEmpty = 6,
}

#[contract]
pub struct AmmContract;

#[contractimpl]
impl AmmContract {
    pub fn initialize(
        env: Env,
        token_a: Address,
        token_b: Address,
        lp_token: Address,
    ) -> Result<(), AmmError> {
        let key_init = symbol_short!("init");
        if env.storage().instance().has(&key_init) {
            return Err(AmmError::AlreadyInitialized);
        }
        env.storage().instance().set(&key_init, &true);
        env.storage().instance().set(&symbol_short!("token_a"), &token_a);
        env.storage().instance().set(&symbol_short!("token_b"), &token_b);
        env.storage().instance().set(&symbol_short!("lp_token"), &lp_token);
        env.storage().instance().set(&symbol_short!("res_a"), &0i128);
        env.storage().instance().set(&symbol_short!("res_b"), &0i128);
        env.storage().instance().set(&symbol_short!("tot_sh"), &0i128);
        Ok(())
    }

    pub fn deposit(
        env: Env,
        provider: Address,
        amount_a_desired: i128,
        amount_b_desired: i128,
        min_lp_mint: i128,
    ) -> Result<i128, AmmError> {
        provider.require_auth();

        if amount_a_desired <= 0 || amount_b_desired <= 0 {
            return Err(AmmError::ZeroDeposit);
        }

        let token_a_addr: Address = env.storage().instance().get(&symbol_short!("token_a")).ok_or(AmmError::NotInitialized)?;
        let token_b_addr: Address = env.storage().instance().get(&symbol_short!("token_b")).ok_or(AmmError::NotInitialized)?;
        let lp_token_addr: Address = env.storage().instance().get(&symbol_short!("lp_token")).ok_or(AmmError::NotInitialized)?;

        let mut reserve_a: i128 = env.storage().instance().get(&symbol_short!("res_a")).unwrap_or(0);
        let mut total_shares: i128 = env.storage().instance().get(&symbol_short!("tot_sh")).unwrap_or(0);
        let mut reserve_b: i128 = env.storage().instance().get(&symbol_short!("res_b")).unwrap_or(0);

        let (deposit_a, deposit_b, minted_shares) = if total_shares == 0 {
            let initial_shares = amount_a_desired;
            if initial_shares < min_lp_mint {
                return Err(AmmError::SlippageExceeded);
            }
            (amount_a_desired, amount_b_desired, initial_shares)
        } else {
            let required_b = (amount_a_desired * reserve_b) / reserve_a;
            if amount_b_desired < required_b {
                return Err(AmmError::InvalidDepositRatio);
            }
            let optimal_a = (amount_b_desired * reserve_a) / reserve_b;
            let (opt_a, opt_b) = if optimal_a <= amount_a_desired {
                (optimal_a, amount_b_desired)
            } else {
                (amount_a_desired, required_b)
            };

            let shares = (opt_a * total_shares) / reserve_a;
            if shares < min_lp_mint {
                return Err(AmmError::SlippageExceeded);
            }
            (opt_a, opt_b, shares)
        };

        let token_a = TokenClient::new(&env, &token_a_addr);
        let token_b = TokenClient::new(&env, &token_b_addr);
        let lp_token = TokenClient::new(&env, &lp_token_addr);

        token_a.transfer(&provider, &env.current_contract_address(), &deposit_a);
        token_b.transfer(&provider, &env.current_contract_address(), &deposit_b);
        lp_token.mint(&provider, &minted_shares);

        reserve_a += deposit_a;
        reserve_b += deposit_b;
        total_shares += minted_shares;

        env.storage().instance().set(&symbol_short!("res_a"), &reserve_a);
        env.storage().instance().set(&symbol_short!("res_b"), &reserve_b);
        env.storage().instance().set(&symbol_short!("tot_sh"), &total_shares);

        Ok(minted_shares)
    }

    pub fn get_reserves(env: Env) -> (i128, i128) {
        let reserve_a: i128 = env.storage().instance().get(&symbol_short!("res_a")).unwrap_or(0);
        let reserve_b: i128 = env.storage().instance().get(&symbol_short!("res_b")).unwrap_or(0);
        (reserve_a, reserve_b)
    }

    pub fn get_total_shares(env: Env) -> i128 {
        env.storage().instance().get(&symbol_short!("tot_sh")).unwrap_or(0)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{Env, Address};

    #[test]
    fn test_initial_deposit()
    {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let token_a_admin = Address::generate(&env);
        let token_b_admin = Address::generate(&env);
        let lp_admin = Address::generate(&env);

        let token_a = env.register_stellar_asset_contract(token_a_admin);
        let token_b = env.register_stellar_asset_contract(token_b_admin);
        let lp_token = env.register_stellar_asset_contract(lp_admin);

        let contract_id = env.register_contract(None, AmmContract);
        let client = AmmContractClient::new(&env, &contract_id);

        client.initialize(&token_a, &token_b, &lp_token);

        let provider = Address::generate(&env);
        let token_a_client = soroban_sdk::token::Client::new(&env, &token_a);
        let token_b_client = soroban_sdk::token::Client::new(&env, &token_b);

        token_a_client.mint(&provider, &1000);
        token_b_client.mint(&provider, &2000);

        let minted = client.deposit(&provider, &1000, &2000, &1000);
        assert_eq!(minted, 1000);
        assert_eq!(client.get_reserves(), (1000, 2000));
        assert_eq!(client.get_total_shares(), 1000);
    }
}
