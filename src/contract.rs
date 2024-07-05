use near_primitives::types::AccountId;
use serde::de::DeserializeOwned;

use crate::query::{CallResultHandler, QueryBuilder};

pub struct Contract {
    contract_id: AccountId,
}

impl Contract {
    pub fn new(contract_id: AccountId) -> Self {
        Self { contract_id }
    }

    pub fn view<Args, Response>(
        &self,
        method_name: &str,
        args: Args,
    ) -> anyhow::Result<QueryBuilder<CallResultHandler<Response, Response>>>
    where
        Args: serde::Serialize,
        Response: DeserializeOwned,
    {
        let args = serde_json::to_vec(&args)?;
        let request = near_primitives::views::QueryRequest::CallFunction {
            account_id: self.contract_id.clone(),
            method_name: method_name.to_owned(),
            args: near_primitives::types::FunctionArgs::from(args),
        };

        Ok(QueryBuilder::new(request, CallResultHandler::default()))
    }
}

#[cfg(test)]
mod tests {
    #[derive(serde::Serialize)]
    pub struct Paging {
        limit: u32,
        page: u32,
    }

    #[tokio::test]
    async fn fetch_from_contract() {
        let result: serde_json::Value =
            crate::contract::Contract::new("race-of-sloths-stage.testnet".parse().unwrap())
                .view("prs", Paging { limit: 5, page: 1 })
                .unwrap()
                .fetch_from_testnet()
                .await
                .unwrap();

        assert!(result.is_array());
    }
}
