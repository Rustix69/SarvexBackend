use sarvex_contracts::sarvex::v1::{
    matching_engine_client::MatchingEngineClient, AddBookRequest, BookSnapshot, CloseBookRequest,
    CloseBookResponse, ContractKind, GetBookSnapshotRequest, MeAmendOrderRequest,
    MeAmendOrderResponse, MeCancelOrderRequest, MeCancelOrderResponse, MeSubmitOrderRequest,
    MeSubmitOrderResponse, Side,
};
use std::future::Future;
use std::time::Duration;
use thiserror::Error;
use tonic::{
    transport::{Channel, Endpoint},
    Code, Request, Status,
};

pub const FLAG_IOC: u32 = 1 << 0;
pub const FLAG_FOK: u32 = 1 << 1;
pub const FLAG_POST_ONLY: u32 = 1 << 2;
pub const FLAG_REDUCE_ONLY: u32 = 1 << 3;

#[derive(Debug, Error)]
pub enum MeCoreError {
    #[error("matching engine request outcome is unknown after dispatch")]
    OutcomeUnknown,
    #[error("matching engine transport failed: {0}")]
    Transport(#[from] tonic::transport::Error),
    #[error("matching engine returned status: {0}")]
    Status(Box<Status>),
}

impl From<Status> for MeCoreError {
    fn from(status: Status) -> Self {
        Self::Status(Box::new(status))
    }
}

impl MeCoreError {
    /// A timeout is never converted into a terminal order rejection. The caller
    /// must reconcile by order_id before releasing a hold or retrying a command.
    pub fn is_unknown_outcome(&self) -> bool {
        match self {
            Self::OutcomeUnknown => true,
            Self::Status(status) => {
                matches!(status.code(), Code::DeadlineExceeded | Code::Unavailable)
            }
            Self::Transport(_) => true,
        }
    }

    pub fn was_rejected_before_enqueue(&self) -> bool {
        matches!(self, Self::Status(status) if status.code() == Code::ResourceExhausted)
    }
}

#[derive(Clone)]
pub struct MeCoreClient {
    client: MatchingEngineClient<Channel>,
    timeout: Duration,
}

impl MeCoreClient {
    pub fn connect_lazy(address: impl Into<String>, timeout: Duration) -> anyhow::Result<Self> {
        let endpoint = Endpoint::from_shared(address.into())?;
        Ok(Self {
            client: MatchingEngineClient::new(endpoint.connect_lazy()),
            timeout,
        })
    }

    pub async fn add_book(&self, request: AddBookRequest) -> Result<(), MeCoreError> {
        let mut client = self.client.clone();
        self.call(client.add_book(Request::new(request)))
            .await
            .map(|_| ())
    }

    pub async fn submit_order(
        &self,
        request: MeSubmitOrderRequest,
    ) -> Result<MeSubmitOrderResponse, MeCoreError> {
        let mut client = self.client.clone();
        self.call(client.submit_order(Request::new(request))).await
    }

    pub async fn cancel_order(
        &self,
        request: MeCancelOrderRequest,
    ) -> Result<MeCancelOrderResponse, MeCoreError> {
        let mut client = self.client.clone();
        self.call(client.cancel_order(Request::new(request))).await
    }

    pub async fn amend_order(
        &self,
        request: MeAmendOrderRequest,
    ) -> Result<MeAmendOrderResponse, MeCoreError> {
        let mut client = self.client.clone();
        self.call(client.amend_order(Request::new(request))).await
    }

    pub async fn close_book(
        &self,
        ticker: impl Into<String>,
    ) -> Result<CloseBookResponse, MeCoreError> {
        let mut client = self.client.clone();
        self.call(client.close_book(Request::new(CloseBookRequest {
            ticker: ticker.into(),
        })))
        .await
    }

    pub async fn get_book_snapshot(
        &self,
        ticker: impl Into<String>,
        depth: i32,
    ) -> Result<BookSnapshot, MeCoreError> {
        let mut client = self.client.clone();
        self.call(
            client.get_book_snapshot(Request::new(GetBookSnapshotRequest {
                ticker: ticker.into(),
                depth,
            })),
        )
        .await
    }

    async fn call<F, T>(&self, future: F) -> Result<T, MeCoreError>
    where
        F: Future<Output = Result<tonic::Response<T>, Status>>,
    {
        match tokio::time::timeout(self.timeout, future).await {
            Ok(result) => result
                .map(tonic::Response::into_inner)
                .map_err(MeCoreError::from),
            Err(_) => Err(MeCoreError::OutcomeUnknown),
        }
    }
}

pub fn flags_for(tif: i32, post_only: bool, reduce_only: bool) -> u32 {
    let mut flags = 0;
    if tif == 2 {
        flags |= FLAG_IOC;
    }
    if tif == 3 {
        flags |= FLAG_FOK;
    }
    if post_only {
        flags |= FLAG_POST_ONLY;
    }
    if reduce_only {
        flags |= FLAG_REDUCE_ONLY;
    }
    flags
}

pub fn is_buy(side: i32, action: i32) -> bool {
    let side_is_positive = side == Side::Yes as i32 || side == Side::Long as i32;
    let action_is_buy = action == 1;
    side_is_positive == action_is_buy
}

pub fn contract_kind(value: i32) -> Option<ContractKind> {
    ContractKind::try_from(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_preserve_frozen_order_router_mapping() {
        assert_eq!(flags_for(1, false, false), 0);
        assert_eq!(flags_for(2, false, false), FLAG_IOC);
        assert_eq!(
            flags_for(3, true, true),
            FLAG_FOK | FLAG_POST_ONLY | FLAG_REDUCE_ONLY
        );
    }

    #[test]
    fn side_action_mapping_is_deterministic() {
        assert!(is_buy(Side::Yes as i32, 1));
        assert!(!is_buy(Side::Yes as i32, 2));
        assert!(!is_buy(Side::No as i32, 1));
        assert!(is_buy(Side::No as i32, 2));
    }

    #[test]
    fn timeout_is_not_a_terminal_rejection() {
        let error = MeCoreError::OutcomeUnknown;
        assert!(error.is_unknown_outcome());
        assert!(!error.was_rejected_before_enqueue());
    }

    #[test]
    fn queue_full_is_distinct_from_unknown_outcome() {
        let error =
            MeCoreError::Status(Box::new(Status::resource_exhausted("sequencer queue full")));
        assert!(error.was_rejected_before_enqueue());
        assert!(!error.is_unknown_outcome());
    }
}
