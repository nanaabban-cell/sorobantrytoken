//! A non-fungible token (NFT) contract for Soroban.
//!
//! The interface follows the shape of ERC-721 / SEP-50: every token has a
//! unique `u32` id and a single owner, owners can approve a spender for one
//! token or an operator for all of their tokens, and each token exposes a
//! metadata URI built from a collection-wide base URI.
//!
//! Only the admin can mint. Owners (or approved spenders) can transfer and burn.
#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, Address,
    Env, String,
};

const DAY_IN_LEDGERS: u32 = 17_280;
const INSTANCE_BUMP_AMOUNT: u32 = 7 * DAY_IN_LEDGERS;
const INSTANCE_LIFETIME_THRESHOLD: u32 = INSTANCE_BUMP_AMOUNT - DAY_IN_LEDGERS;
const PERSISTENT_BUMP_AMOUNT: u32 = 30 * DAY_IN_LEDGERS;
const PERSISTENT_LIFETIME_THRESHOLD: u32 = PERSISTENT_BUMP_AMOUNT - DAY_IN_LEDGERS;

/// Longest base URI accepted, so `token_uri` can be built in a fixed buffer.
const MAX_BASE_URI_LEN: u32 = 200;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum NftError {
    NonExistentToken = 1,
    IncorrectOwner = 2,
    InsufficientApproval = 3,
    InvalidLiveUntilLedger = 4,
    BaseUriTooLong = 5,
    TokenIdOverflow = 6,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    // Instance storage
    Admin,
    Name,
    Symbol,
    BaseUri,
    NextTokenId,
    TotalSupply,
    // Persistent storage
    Owner(u32),
    Balance(Address),
    // Temporary storage (approvals expire on their own)
    Approval(u32),
    ApprovalForAll(Address, Address),
}

#[contracttype]
#[derive(Clone)]
pub struct ApprovalData {
    pub approved: Address,
    pub live_until_ledger: u32,
}

#[contractevent]
pub struct Mint {
    #[topic]
    pub to: Address,
    pub token_id: u32,
}

#[contractevent]
pub struct Transfer {
    #[topic]
    pub from: Address,
    #[topic]
    pub to: Address,
    pub token_id: u32,
}

#[contractevent]
pub struct Burn {
    #[topic]
    pub from: Address,
    pub token_id: u32,
}

#[contractevent]
pub struct Approve {
    #[topic]
    pub owner: Address,
    #[topic]
    pub token_id: u32,
    pub approved: Address,
    pub live_until_ledger: u32,
}

#[contractevent]
pub struct ApproveForAll {
    #[topic]
    pub owner: Address,
    pub operator: Address,
    pub live_until_ledger: u32,
}

#[contractevent]
pub struct SetAdmin {
    #[topic]
    pub admin: Address,
    pub new_admin: Address,
}

#[contract]
pub struct NftContract;

#[contractimpl]
impl NftContract {
    pub fn __constructor(env: Env, admin: Address, name: String, symbol: String, base_uri: String) {
        if base_uri.len() > MAX_BASE_URI_LEN {
            panic_with_error!(&env, NftError::BaseUriTooLong);
        }
        let s = env.storage().instance();
        s.set(&DataKey::Admin, &admin);
        s.set(&DataKey::Name, &name);
        s.set(&DataKey::Symbol, &symbol);
        s.set(&DataKey::BaseUri, &base_uri);
        s.set(&DataKey::NextTokenId, &0u32);
        s.set(&DataKey::TotalSupply, &0u32);
    }

    // ---------------------------------------------------------------------
    // Admin
    // ---------------------------------------------------------------------

    /// Mints the next sequential token to `to`. Admin only. Returns the new id.
    pub fn mint(env: Env, to: Address) -> u32 {
        Self::admin(env.clone()).require_auth();
        bump_instance(&env);

        let s = env.storage().instance();
        let token_id: u32 = s.get(&DataKey::NextTokenId).unwrap();
        let next = token_id
            .checked_add(1)
            .unwrap_or_else(|| panic_with_error!(&env, NftError::TokenIdOverflow));
        s.set(&DataKey::NextTokenId, &next);
        let supply: u32 = s.get(&DataKey::TotalSupply).unwrap();
        s.set(&DataKey::TotalSupply, &(supply + 1));

        set_owner(&env, token_id, &to);
        change_balance(&env, &to, 1);

        Mint { to, token_id }.publish(&env);
        token_id
    }

    pub fn set_admin(env: Env, new_admin: Address) {
        let admin = Self::admin(env.clone());
        admin.require_auth();
        bump_instance(&env);
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        SetAdmin { admin, new_admin }.publish(&env);
    }

    pub fn set_base_uri(env: Env, base_uri: String) {
        Self::admin(env.clone()).require_auth();
        if base_uri.len() > MAX_BASE_URI_LEN {
            panic_with_error!(&env, NftError::BaseUriTooLong);
        }
        bump_instance(&env);
        env.storage().instance().set(&DataKey::BaseUri, &base_uri);
    }

    // ---------------------------------------------------------------------
    // Transfers
    // ---------------------------------------------------------------------

    /// Transfers `token_id` from `from` (who must be the owner and authorize) to `to`.
    pub fn transfer(env: Env, from: Address, to: Address, token_id: u32) {
        from.require_auth();
        do_transfer(&env, &from, &to, token_id);
    }

    /// Transfers `token_id` on behalf of `from`, authorized by an approved `spender`.
    pub fn transfer_from(env: Env, spender: Address, from: Address, to: Address, token_id: u32) {
        spender.require_auth();
        check_spender(&env, &spender, &from, token_id);
        do_transfer(&env, &from, &to, token_id);
    }

    /// Destroys `token_id`. `from` must be the owner and authorize.
    pub fn burn(env: Env, from: Address, token_id: u32) {
        from.require_auth();
        do_burn(&env, &from, token_id);
    }

    /// Destroys `token_id` on behalf of `from`, authorized by an approved `spender`.
    pub fn burn_from(env: Env, spender: Address, from: Address, token_id: u32) {
        spender.require_auth();
        check_spender(&env, &spender, &from, token_id);
        do_burn(&env, &from, token_id);
    }

    // ---------------------------------------------------------------------
    // Approvals
    // ---------------------------------------------------------------------

    /// Lets `approved` move `token_id` until `live_until_ledger`. `approver` must
    /// be the owner or an operator of the owner. A `live_until_ledger` of 0 revokes.
    pub fn approve(
        env: Env,
        approver: Address,
        approved: Address,
        token_id: u32,
        live_until_ledger: u32,
    ) {
        approver.require_auth();
        let owner = Self::owner_of(env.clone(), token_id);
        if approver != owner && !Self::is_approved_for_all(env.clone(), owner.clone(), approver) {
            panic_with_error!(&env, NftError::InsufficientApproval);
        }

        let key = DataKey::Approval(token_id);
        if live_until_ledger == 0 {
            env.storage().temporary().remove(&key);
        } else {
            let ttl = ttl_until(&env, live_until_ledger);
            let data = ApprovalData { approved: approved.clone(), live_until_ledger };
            env.storage().temporary().set(&key, &data);
            env.storage().temporary().extend_ttl(&key, ttl, ttl);
        }

        Approve { owner, token_id, approved, live_until_ledger }.publish(&env);
    }

    /// Lets `operator` manage all of `owner`'s tokens until `live_until_ledger`.
    /// A `live_until_ledger` of 0 revokes.
    pub fn approve_for_all(env: Env, owner: Address, operator: Address, live_until_ledger: u32) {
        owner.require_auth();
        let key = DataKey::ApprovalForAll(owner.clone(), operator.clone());
        if live_until_ledger == 0 {
            env.storage().temporary().remove(&key);
        } else {
            let ttl = ttl_until(&env, live_until_ledger);
            env.storage().temporary().set(&key, &live_until_ledger);
            env.storage().temporary().extend_ttl(&key, ttl, ttl);
        }
        ApproveForAll { owner, operator, live_until_ledger }.publish(&env);
    }

    pub fn get_approved(env: Env, token_id: u32) -> Option<Address> {
        let data: Option<ApprovalData> =
            env.storage().temporary().get(&DataKey::Approval(token_id));
        data.filter(|d| d.live_until_ledger >= env.ledger().sequence())
            .map(|d| d.approved)
    }

    pub fn is_approved_for_all(env: Env, owner: Address, operator: Address) -> bool {
        let live_until: Option<u32> = env
            .storage()
            .temporary()
            .get(&DataKey::ApprovalForAll(owner, operator));
        live_until.is_some_and(|l| l >= env.ledger().sequence())
    }

    // ---------------------------------------------------------------------
    // Views
    // ---------------------------------------------------------------------

    pub fn owner_of(env: Env, token_id: u32) -> Address {
        let key = DataKey::Owner(token_id);
        let owner: Address = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| panic_with_error!(&env, NftError::NonExistentToken));
        bump_persistent(&env, &key);
        owner
    }

    pub fn balance(env: Env, owner: Address) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::Balance(owner))
            .unwrap_or(0)
    }

    pub fn total_supply(env: Env) -> u32 {
        env.storage().instance().get(&DataKey::TotalSupply).unwrap()
    }

    pub fn admin(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Admin).unwrap()
    }

    pub fn name(env: Env) -> String {
        env.storage().instance().get(&DataKey::Name).unwrap()
    }

    pub fn symbol(env: Env) -> String {
        env.storage().instance().get(&DataKey::Symbol).unwrap()
    }

    /// Returns `base_uri` followed by the decimal token id, e.g. `ipfs://.../7`.
    pub fn token_uri(env: Env, token_id: u32) -> String {
        // Panics if the token doesn't exist.
        Self::owner_of(env.clone(), token_id);

        let base: String = env.storage().instance().get(&DataKey::BaseUri).unwrap();
        let base_len = base.len() as usize;
        let mut buf = [0u8; MAX_BASE_URI_LEN as usize + 10];
        base.copy_into_slice(&mut buf[..base_len]);

        let mut digits = [0u8; 10];
        let mut n = token_id;
        let mut i = digits.len();
        loop {
            i -= 1;
            digits[i] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        let digits = &digits[i..];
        buf[base_len..base_len + digits.len()].copy_from_slice(digits);

        String::from_bytes(&env, &buf[..base_len + digits.len()])
    }
}

// -------------------------------------------------------------------------
// Internal helpers
// -------------------------------------------------------------------------

fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
}

fn bump_persistent(env: &Env, key: &DataKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, PERSISTENT_LIFETIME_THRESHOLD, PERSISTENT_BUMP_AMOUNT);
}

/// Number of ledgers from now until `live_until_ledger`, validating it.
fn ttl_until(env: &Env, live_until_ledger: u32) -> u32 {
    let current = env.ledger().sequence();
    if live_until_ledger < current || live_until_ledger > env.ledger().max_live_until_ledger() {
        panic_with_error!(env, NftError::InvalidLiveUntilLedger);
    }
    live_until_ledger - current
}

fn set_owner(env: &Env, token_id: u32, owner: &Address) {
    let key = DataKey::Owner(token_id);
    env.storage().persistent().set(&key, owner);
    bump_persistent(env, &key);
}

fn change_balance(env: &Env, owner: &Address, delta: i64) {
    let key = DataKey::Balance(owner.clone());
    let current: u32 = env.storage().persistent().get(&key).unwrap_or(0);
    let updated = (current as i64 + delta) as u32;
    if updated == 0 {
        env.storage().persistent().remove(&key);
    } else {
        env.storage().persistent().set(&key, &updated);
        bump_persistent(env, &key);
    }
}

fn check_owner(env: &Env, from: &Address, token_id: u32) {
    if NftContract::owner_of(env.clone(), token_id) != *from {
        panic_with_error!(env, NftError::IncorrectOwner);
    }
}

/// Panics unless `spender` is the owner, the token's approved address, or an operator.
fn check_spender(env: &Env, spender: &Address, owner: &Address, token_id: u32) {
    check_owner(env, owner, token_id);
    if spender == owner
        || NftContract::get_approved(env.clone(), token_id).as_ref() == Some(spender)
        || NftContract::is_approved_for_all(env.clone(), owner.clone(), spender.clone())
    {
        return;
    }
    panic_with_error!(env, NftError::InsufficientApproval);
}

fn do_transfer(env: &Env, from: &Address, to: &Address, token_id: u32) {
    check_owner(env, from, token_id);
    bump_instance(env);

    // A single-token approval does not survive a change of owner.
    env.storage().temporary().remove(&DataKey::Approval(token_id));
    set_owner(env, token_id, to);
    change_balance(env, from, -1);
    change_balance(env, to, 1);

    Transfer { from: from.clone(), to: to.clone(), token_id }.publish(env);
}

fn do_burn(env: &Env, from: &Address, token_id: u32) {
    check_owner(env, from, token_id);
    bump_instance(env);

    env.storage().temporary().remove(&DataKey::Approval(token_id));
    env.storage().persistent().remove(&DataKey::Owner(token_id));
    change_balance(env, from, -1);
    let s = env.storage().instance();
    let supply: u32 = s.get(&DataKey::TotalSupply).unwrap();
    s.set(&DataKey::TotalSupply, &(supply - 1));

    Burn { from: from.clone(), token_id }.publish(env);
}

#[cfg(test)]
mod test;
