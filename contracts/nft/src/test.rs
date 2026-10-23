#![cfg(test)]
extern crate std;

use super::*;
use soroban_sdk::{testutils::Address as _, testutils::Ledger, Address, Env, String};

struct Setup<'a> {
    env: Env,
    admin: Address,
    client: NftContractClient<'a>,
}

fn setup() -> Setup<'static> {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(
        NftContract,
        (
            admin.clone(),
            String::from_str(&env, "Try Collection"),
            String::from_str(&env, "TRY"),
            String::from_str(&env, "ipfs://bafybeigdyrzt/"),
        ),
    );
    let client = NftContractClient::new(&env, &contract_id);
    Setup { env, admin, client }
}

#[test]
fn metadata() {
    let Setup { env, admin, client } = setup();
    assert_eq!(client.name(), String::from_str(&env, "Try Collection"));
    assert_eq!(client.symbol(), String::from_str(&env, "TRY"));
    assert_eq!(client.admin(), admin);
    assert_eq!(client.total_supply(), 0);
}

#[test]
fn mint_assigns_sequential_ids() {
    let Setup { env, client, .. } = setup();
    let alice = Address::generate(&env);

    assert_eq!(client.mint(&alice), 0);
    assert_eq!(client.mint(&alice), 1);
    assert_eq!(client.owner_of(&1), alice);
    assert_eq!(client.balance(&alice), 2);
    assert_eq!(client.total_supply(), 2);
    assert_eq!(client.token_uri(&1), String::from_str(&env, "ipfs://bafybeigdyrzt/1"));
}

#[test]
fn token_uri_multi_digit() {
    let Setup { env, client, .. } = setup();
    let alice = Address::generate(&env);
    for _ in 0..124 {
        client.mint(&alice);
    }
    assert_eq!(client.token_uri(&123), String::from_str(&env, "ipfs://bafybeigdyrzt/123"));
}

#[test]
fn mint_requires_admin_auth() {
    let Setup { env, admin, client } = setup();
    let alice = Address::generate(&env);
    client.mint(&alice);
    let auths = env.auths();
    assert_eq!(auths.len(), 1);
    assert_eq!(auths[0].0, admin);
}

#[test]
fn transfer_moves_ownership() {
    let Setup { env, client, .. } = setup();
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let id = client.mint(&alice);

    client.transfer(&alice, &bob, &id);
    assert_eq!(client.owner_of(&id), bob);
    assert_eq!(client.balance(&alice), 0);
    assert_eq!(client.balance(&bob), 1);
}

#[test]
fn transfer_by_non_owner_fails() {
    let Setup { env, client, .. } = setup();
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let id = client.mint(&alice);

    let res = client.try_transfer(&bob, &bob, &id);
    assert_eq!(res, Err(Ok(NftError::IncorrectOwner.into())));
}

#[test]
fn approved_spender_can_transfer_once() {
    let Setup { env, client, .. } = setup();
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let carol = Address::generate(&env);
    let id = client.mint(&alice);

    client.approve(&alice, &bob, &id, &1000);
    assert_eq!(client.get_approved(&id), Some(bob.clone()));

    client.transfer_from(&bob, &alice, &carol, &id);
    assert_eq!(client.owner_of(&id), carol);
    // Approval is cleared on transfer.
    assert_eq!(client.get_approved(&id), None);
    let res = client.try_transfer_from(&bob, &carol, &bob, &id);
    assert_eq!(res, Err(Ok(NftError::InsufficientApproval.into())));
}

#[test]
fn unapproved_spender_fails() {
    let Setup { env, client, .. } = setup();
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let id = client.mint(&alice);

    let res = client.try_transfer_from(&bob, &alice, &bob, &id);
    assert_eq!(res, Err(Ok(NftError::InsufficientApproval.into())));
}

#[test]
fn operator_can_transfer_and_approve() {
    let Setup { env, client, .. } = setup();
    let alice = Address::generate(&env);
    let operator = Address::generate(&env);
    let bob = Address::generate(&env);
    let a = client.mint(&alice);
    let b = client.mint(&alice);

    client.approve_for_all(&alice, &operator, &1000);
    assert!(client.is_approved_for_all(&alice, &operator));

    client.transfer_from(&operator, &alice, &bob, &a);
    assert_eq!(client.owner_of(&a), bob);

    // Operators may grant single-token approvals on the owner's behalf.
    client.approve(&operator, &bob, &b, &1000);
    assert_eq!(client.get_approved(&b), Some(bob));

    client.approve_for_all(&alice, &operator, &0);
    assert!(!client.is_approved_for_all(&alice, &operator));
}

#[test]
fn approval_expires() {
    let Setup { env, client, .. } = setup();
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let id = client.mint(&alice);

    let now = env.ledger().sequence();
    client.approve(&alice, &bob, &id, &(now + 10));
    env.ledger().with_mut(|l| l.sequence_number = now + 11);

    assert_eq!(client.get_approved(&id), None);
    let res = client.try_transfer_from(&bob, &alice, &bob, &id);
    assert_eq!(res, Err(Ok(NftError::InsufficientApproval.into())));
}

#[test]
fn burn_removes_token() {
    let Setup { env, client, .. } = setup();
    let alice = Address::generate(&env);
    let id = client.mint(&alice);

    client.burn(&alice, &id);
    assert_eq!(client.balance(&alice), 0);
    assert_eq!(client.total_supply(), 0);
    assert_eq!(client.try_owner_of(&id), Err(Ok(NftError::NonExistentToken.into())));
}

#[test]
fn set_admin_transfers_mint_rights() {
    let Setup { env, client, .. } = setup();
    let new_admin = Address::generate(&env);
    client.set_admin(&new_admin);
    assert_eq!(client.admin(), new_admin);

    client.mint(&new_admin);
    assert_eq!(env.auths()[0].0, new_admin);
}
