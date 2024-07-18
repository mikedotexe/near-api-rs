use near::{signer::Signer, types::Data, Contract, NetworkConfig};

#[tokio::main]
async fn main() {
    let network = near_workspaces::sandbox().await.unwrap();
    let account = network.dev_create_account().await.unwrap();
    let network = NetworkConfig::from(network);

    // Let's deploy the contract. The contract is simple counter with `get_num`, `increase`, `decrease` arguments
    Contract(account.id().clone())
        .deploy(include_bytes!("./resources/counter.wasm").to_vec())
        // You can add init call as well using `with_init_call`
        .without_init_call()
        .with_signer(Signer::from_workspace(&account))
        .send_to(&network)
        .await
        .unwrap();

    // Let's fetch current value on a contract
    let current_value: Data<i8> = Contract(account.id().clone())
        // Please note that you can add any argument as long as it is deserializable by serde :)
        // feel free to use serde_json::json macro as well
        .call_function("get_num", ())
        .unwrap()
        .read_only()
        .fetch_from(&network)
        .await
        .unwrap();

    println!("Current value: {}", current_value.data);

    // Here is a transaction that require signing compared to view call that was used before.
    Contract(account.id().clone())
        .call_function("increment", ())
        .unwrap()
        .transaction()
        .with_signer(account.id().clone(), Signer::from_workspace(&account))
        .send_to(&network)
        .await
        .unwrap()
        .assert_success();

    let current_value: Data<i8> = Contract(account.id().clone())
        .call_function("get_num", ())
        .unwrap()
        .read_only()
        .fetch_from(&network)
        .await
        .unwrap();

    println!("Current value: {}", current_value.data);
}