use near_crypto::PublicKey;
use near_primitives::{
    action::delegate::SignedDelegateAction,
    hash::CryptoHash,
    transaction::{SignedTransaction},
    types::{BlockHeight, Nonce},
    views::FinalExecutionOutcomeView,
};
use reqwest::Response;

use crate::{
    config::NetworkConfig,
    errors::{
        ExecuteMetaTransactionsError, ExecuteTransactionError, MetaSignError, SignerError,
        ValidationError,
    },
    signer::SignerTrait,
    types::transactions::PrepopulateTransaction,
};

use super::{
    signed_delegate_action::SignedDelegateActionAsBase64, META_TRANSACTION_VALID_FOR_DEFAULT,
};

#[async_trait::async_trait]
pub trait Transactionable {
    fn prepopulated(&self) -> PrepopulateTransaction;
    async fn validate_with_network(&self, network: &NetworkConfig) -> Result<(), ValidationError>;
}

pub enum TransactionableOrSigned<Signed> {
    Transactionable(Box<dyn Transactionable>),
    Signed((Signed, Box<dyn Transactionable>)),
}

impl<Signed> TransactionableOrSigned<Signed> {
    pub fn signed(self) -> Option<Signed> {
        match self {
            TransactionableOrSigned::Signed((signed, _)) => Some(signed),
            TransactionableOrSigned::Transactionable(_) => None,
        }
    }
}

impl< S> TransactionableOrSigned< S> {
    pub fn transactionable(self) -> Box<dyn Transactionable> {
        match self {
            TransactionableOrSigned::Transactionable(tr) => tr,
            TransactionableOrSigned::Signed((_, tr)) => tr,
        }
    }
}

impl From<SignedTransaction> for PrepopulateTransaction {
    fn from(tr: SignedTransaction) -> Self {
        PrepopulateTransaction {
            signer_id: tr.transaction.signer_id,
            receiver_id: tr.transaction.receiver_id,
            actions: tr.transaction.actions,
        }
    }
}

pub struct ExecuteSignedTransaction {
    pub tr: TransactionableOrSigned<SignedTransaction>,
    pub signer: Box<dyn SignerTrait>,
    pub retries: u8,
    pub sleep_duration: std::time::Duration,
}

impl ExecuteSignedTransaction {
    pub fn new<T: Transactionable + 'static>(tr: T, signer: Box<dyn SignerTrait>) -> Self {
        Self {
            tr: TransactionableOrSigned::Transactionable(Box::new(tr)),
            signer,
            retries: 5,
            sleep_duration: std::time::Duration::from_secs(1),
        }
    }

    pub fn meta(self) -> ExecuteMetaTransaction {
        ExecuteMetaTransaction::from_box(self.tr.transactionable(), self.signer)
    }

    pub fn with_retries(mut self, retries: u8) -> Self {
        self.retries = retries;
        self
    }

    pub fn with_sleep_duration(mut self, sleep_duration: std::time::Duration) -> Self {
        self.sleep_duration = sleep_duration;
        self
    }

    pub fn presign_offline(
        mut self,
        public_key: PublicKey,
        block_hash: CryptoHash,
        nonce: Nonce,
    ) -> Result<Self, SignerError> {
        let tr = match &self.tr {
            TransactionableOrSigned::Transactionable(tr) => tr,
            TransactionableOrSigned::Signed(_) => return Ok(self),
        };
        let signed_tr = self
            .signer
            .sign(tr.prepopulated(), public_key, nonce, block_hash)?;
        self.tr = TransactionableOrSigned::Signed((signed_tr, self.tr.transactionable()));
        Ok(self)
    }

    pub async fn presign_with(
        self,
        network: &NetworkConfig,
    ) -> Result<Self, ExecuteTransactionError> {
        let tr = match &self.tr {
            TransactionableOrSigned::Transactionable(tr) => tr,
            TransactionableOrSigned::Signed(_) => return Ok(self),
        };

        let signer_key = self.signer.get_public_key()?;
        let tr = tr.prepopulated();
        let response = crate::account::Account(tr.signer_id.clone())
            .access_key(signer_key.clone())
            .fetch_from(network)
            .await?;
        Ok(self.presign_offline(signer_key, response.block_hash, response.data.nonce + 1)?)
    }

    pub async fn presign_with_mainnet(self) -> Result<Self, ExecuteTransactionError> {
        let network = NetworkConfig::mainnet();
        self.presign_with(&network).await
    }

    pub async fn presign_with_testnet(self) -> Result<Self, ExecuteTransactionError> {
        let network = NetworkConfig::testnet();
        self.presign_with(&network).await
    }

    pub async fn send_to(
        self,
        network: &NetworkConfig,
    ) -> Result<FinalExecutionOutcomeView, ExecuteTransactionError> {
        let sleep_duration = self.sleep_duration;
        let retries = self.retries;

        let (signed, transactionable) = match &self.tr {
            TransactionableOrSigned::Transactionable(tr) => (None, tr),
            TransactionableOrSigned::Signed((s, tr)) => (Some(s.clone()), tr),
        };

        transactionable.validate_with_network(network).await?;

        let signed = match signed {
            Some(s) => s,
            None => self
                .presign_with(network)
                .await?
                .tr
                .signed()
                .expect("Expect to have it signed"),
        };
        Self::send_impl(network, signed, retries, sleep_duration).await
    }

    pub async fn send_to_mainnet(
        self,
    ) -> Result<FinalExecutionOutcomeView, ExecuteTransactionError> {
        let network = NetworkConfig::mainnet();
        self.send_to(&network).await
    }

    pub async fn send_to_testnet(
        self,
    ) -> Result<FinalExecutionOutcomeView, ExecuteTransactionError> {
        let network = NetworkConfig::testnet();
        self.send_to(&network).await
    }

    async fn send_impl(
        network: &NetworkConfig,
        signed_tr: SignedTransaction,
        retries: u8,
        sleep_duration: std::time::Duration,
    ) -> Result<FinalExecutionOutcomeView, ExecuteTransactionError> {
        let mut retries = (1..=retries).rev();
        let transaction_info = loop {
            let transaction_info_result = network
                .json_rpc_client()
                .call(
                    near_jsonrpc_client::methods::broadcast_tx_commit::RpcBroadcastTxCommitRequest {
                        signed_transaction: signed_tr.clone(),
                    },
                )
                .await;
            match transaction_info_result {
                Ok(response) => {
                    break response;
                }
                Err(err) => {
                    if is_critical_error(&err) {
                        return Err(ExecuteTransactionError::CriticalTransactionError(err));
                    } else if retries.next().is_some() {
                        tokio::time::sleep(sleep_duration).await;
                    } else {
                        return Err(ExecuteTransactionError::RetriesExhausted(err));
                    }
                }
            };
        };
        Ok(transaction_info)
    }
}

pub struct ExecuteMetaTransaction{
    pub tr: TransactionableOrSigned<SignedDelegateAction>,
    pub signer: Box<dyn SignerTrait>,
    pub tx_live_for: Option<BlockHeight>,
}

impl ExecuteMetaTransaction {
    pub fn new<T: Transactionable + 'static>(tr: T, signer: Box<dyn SignerTrait>) -> Self {
        Self {
            tr: TransactionableOrSigned::Transactionable(Box::new(tr)),
            signer,
            tx_live_for: None,
        }
    }

    pub fn from_box(tr: Box<dyn Transactionable>, signer: Box<dyn SignerTrait>) -> Self {
        Self {
            tr: TransactionableOrSigned::Transactionable(tr), signer, tx_live_for: None
        }
    }

    pub fn tx_live_for(mut self, tx_live_for: BlockHeight) -> Self {
        self.tx_live_for = Some(tx_live_for);
        self
    }

    pub fn presign_offline(
        mut self,
        public_key: PublicKey,
        block_hash: CryptoHash,
        nonce: Nonce,
        block_height: BlockHeight,
    ) -> Result<Self, MetaSignError> {
        let tr = match &self.tr {
            TransactionableOrSigned::Transactionable(tr) => tr,
            TransactionableOrSigned::Signed(_) => return Ok(self),
        };

        let max_block_height = block_height
            + self
                .tx_live_for
                .unwrap_or(META_TRANSACTION_VALID_FOR_DEFAULT);
        let signed_tr = self.signer.sign_meta(
            tr.prepopulated(),
            public_key,
            nonce,
            block_hash,
            max_block_height,
        )?;
        self.tr = TransactionableOrSigned::Signed((signed_tr, self.tr.transactionable()));
        Ok(self)
    }

    pub async fn presign_with(
        self,
        network: &NetworkConfig,
    ) -> Result<Self, ExecuteMetaTransactionsError> {
        let tr = match &self.tr {
            TransactionableOrSigned::Transactionable(tr) => tr,
            TransactionableOrSigned::Signed(_) => return Ok(self),
        };

        let signer_key = self.signer.get_public_key().map_err(MetaSignError::from)?;
        let response = crate::account::Account(tr.prepopulated().signer_id.clone())
            .access_key(signer_key.clone())
            .fetch_from(network)
            .await?;
        Ok(self.presign_offline(
            signer_key,
            response.block_hash,
            response.data.nonce + 1,
            response.block_height,
        )?)
    }

    pub async fn presign_with_mainnet(self) -> Result<Self, ExecuteMetaTransactionsError> {
        let network = NetworkConfig::mainnet();
        self.presign_with(&network).await
    }

    pub async fn presign_with_testnet(self) -> Result<Self, ExecuteMetaTransactionsError> {
        let network = NetworkConfig::testnet();
        self.presign_with(&network).await
    }

    pub async fn send_to(
        self,
        network: &NetworkConfig,
    ) -> Result<Response, ExecuteMetaTransactionsError> {
        let (signed, transactionable) = match &self.tr {
            TransactionableOrSigned::Transactionable(tr) => (None, tr),
            TransactionableOrSigned::Signed((s, tr)) => (Some(s.clone()), tr),
        };

        transactionable.validate_with_network(network).await?;

        let signed = match signed {
            Some(s) => s,
            None => self
                .presign_with(network)
                .await?
                .tr
                .signed()
                .expect("Expect to have it signed"),
        };
        let transaction_info = Self::send_impl(network, signed).await?;
        Ok(transaction_info)
    }

    pub async fn send_to_mainnet(self) -> Result<reqwest::Response, ExecuteMetaTransactionsError> {
        let network = NetworkConfig::mainnet();
        self.send_to(&network).await
    }

    pub async fn send_to_testnet(self) -> Result<reqwest::Response, ExecuteMetaTransactionsError> {
        let network = NetworkConfig::testnet();
        self.send_to(&network).await
    }

    async fn send_impl(
        network: &NetworkConfig,
        tr: SignedDelegateAction,
    ) -> Result<reqwest::Response, ExecuteMetaTransactionsError> {
        let client = reqwest::Client::new();
        let json_payload = serde_json::json!({
            "signed_delegate_action": SignedDelegateActionAsBase64::from(
                tr
            ).to_string(),
        });
        let resp = client
            .post(
                network
                    .meta_transaction_relayer_url
                    .clone()
                    .ok_or(ExecuteMetaTransactionsError::RelayerIsNotDefined)?,
            )
            .json(&json_payload)
            .send()
            .await?;
        Ok(resp)
    }
}

pub fn is_critical_error(
    err: &near_jsonrpc_client::errors::JsonRpcError<
        near_jsonrpc_client::methods::broadcast_tx_commit::RpcTransactionError,
    >,
) -> bool {
    match err {
        near_jsonrpc_client::errors::JsonRpcError::TransportError(_rpc_transport_error) => {
            false
        }
        near_jsonrpc_client::errors::JsonRpcError::ServerError(rpc_server_error) => match rpc_server_error {
            near_jsonrpc_client::errors::JsonRpcServerError::HandlerError(rpc_transaction_error) => match rpc_transaction_error {
                near_jsonrpc_client::methods::broadcast_tx_commit::RpcTransactionError::TimeoutError => {
                    false
                }
                near_jsonrpc_client::methods::broadcast_tx_commit::RpcTransactionError::InvalidTransaction { .. } |
                    near_jsonrpc_client::methods::broadcast_tx_commit::RpcTransactionError::DoesNotTrackShard |
                        near_jsonrpc_client::methods::broadcast_tx_commit::RpcTransactionError::RequestRouted{..} |
                            near_jsonrpc_client::methods::broadcast_tx_commit::RpcTransactionError::UnknownTransaction{..} |
                                near_jsonrpc_client::methods::broadcast_tx_commit::RpcTransactionError::InternalError{..} => {
                    true
                }
            }
            near_jsonrpc_client::errors::JsonRpcServerError::RequestValidationError(..) |
            near_jsonrpc_client::errors::JsonRpcServerError::NonContextualError(..) => {
                true
            }
            near_jsonrpc_client::errors::JsonRpcServerError::InternalError{ .. } => {
                false
            }
            near_jsonrpc_client::errors::JsonRpcServerError::ResponseStatusError(json_rpc_server_response_status_error) => match json_rpc_server_response_status_error {
                near_jsonrpc_client::errors::JsonRpcServerResponseStatusError::Unauthorized |
                near_jsonrpc_client::errors::JsonRpcServerResponseStatusError::Unexpected{..} => {
                    true
                }
                near_jsonrpc_client::errors::JsonRpcServerResponseStatusError::TooManyRequests => {
                    false
                }
            }
        }
    }
}
