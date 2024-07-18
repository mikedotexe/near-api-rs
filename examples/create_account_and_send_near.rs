use near::{signer::Signer, NetworkConfig};
use near_token::NearToken;
use near_workspaces::AccountId;

#[tokio::main]
async fn main() {
    let network = near_workspaces::sandbox().await.unwrap();
    let account = network.dev_create_account().await.unwrap();
    let network = NetworkConfig::from(network);

    let balance = near::Tokens(account.id().clone())
        .near_balance()
        .fetch_from(&network)
        .await
        .unwrap();

    println!("Balance: {}", balance.liquid);

    let new_account: AccountId = format!("{}.{}", "bob", account.id()).parse().unwrap();

    near::Account::create_account()
        .fund_myself(
            new_account.clone(),
            account.id().clone(),
            NearToken::from_near(1),
        )
        .new_keypair()
        .save_generated_seed_to_file("./new_account_seed".into())
        .unwrap()
        .with_signer(Signer::from_workspace(&account))
        .send_to(&network)
        .await
        .unwrap();

    near::Tokens(account.id().clone())
        .send_near(new_account.clone(), NearToken::from_near(1))
        .with_signer(Signer::from_workspace(&account))
        .send_to(&network)
        .await
        .unwrap();

    let new_acccount_balance = near::Tokens(account.id().clone())
        .near_balance()
        .fetch_from(&network)
        .await
        .unwrap();
    let bob_balance = near::Tokens(new_account)
        .near_balance()
        .fetch_from(&network)
        .await
        .unwrap();

    println!("Balance: {}", new_acccount_balance.liquid);
    // Expect to see 2 NEAR in Bob's account. 1 NEAR from create_account and 1 NEAR from send_near
    println!("Bob balance: {}", bob_balance.liquid);
}