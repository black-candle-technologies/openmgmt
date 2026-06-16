use crate::{SyncClientError, SyncClientResult};
use openmgmt_protocol::{
    DeviceRegistrationRequest, DeviceRegistrationResponse, SyncHelloRequest, SyncHelloResponse,
    SyncPullRequest, SyncPullResponse, SyncPushRequest, SyncPushResponse,
};
use serde::{Serialize, de::DeserializeOwned};
use std::time::Duration;

pub struct OmgpHttpClient {
    base_url: String,
    client: reqwest::Client,
}

impl OmgpHttpClient {
    pub fn new(server_url: &str, timeout_seconds: u64) -> SyncClientResult<Self> {
        Ok(Self {
            base_url: server_url.trim_end_matches('/').to_owned(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(timeout_seconds))
                .build()?,
        })
    }

    pub async fn hello(&self, request: SyncHelloRequest) -> SyncClientResult<SyncHelloResponse> {
        let response: SyncHelloResponse = self.post("/omgp/v1/hello", &request).await?;
        check_protocol_error(response.error.as_ref())?;
        if !response.compatible {
            return Err(SyncClientError::Protocol(
                "server reported an incompatible protocol version".into(),
            ));
        }
        Ok(response)
    }

    pub async fn register_device(
        &self,
        request: DeviceRegistrationRequest,
    ) -> SyncClientResult<DeviceRegistrationResponse> {
        let response: DeviceRegistrationResponse =
            self.post("/omgp/v1/devices/register", &request).await?;
        check_protocol_error(response.error.as_ref())?;
        if !response.accepted {
            return Err(SyncClientError::Protocol(
                "device registration was not accepted".into(),
            ));
        }
        Ok(response)
    }

    pub async fn push(&self, request: SyncPushRequest) -> SyncClientResult<SyncPushResponse> {
        let response: SyncPushResponse = self.post("/omgp/v1/sync/push", &request).await?;
        check_protocol_error(response.error.as_ref())?;
        Ok(response)
    }

    pub async fn pull(&self, request: SyncPullRequest) -> SyncClientResult<SyncPullResponse> {
        let response: SyncPullResponse = self.post("/omgp/v1/sync/pull", &request).await?;
        check_protocol_error(response.error.as_ref())?;
        Ok(response)
    }

    async fn post<T: Serialize, R: DeserializeOwned>(
        &self,
        path: &str,
        request: &T,
    ) -> SyncClientResult<R> {
        Ok(self
            .client
            .post(self.endpoint(path))
            .json(request)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    pub(crate) fn endpoint(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }
}

fn check_protocol_error(error: Option<&openmgmt_protocol::ProtocolError>) -> SyncClientResult<()> {
    if let Some(error) = error {
        Err(SyncClientError::Protocol(format!(
            "{:?}: {}",
            error.code, error.message
        )))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_trims_trailing_slash() {
        let client = OmgpHttpClient::new("http://127.0.0.1:8787/", 15).unwrap();
        assert_eq!(
            client.endpoint("/omgp/v1/hello"),
            "http://127.0.0.1:8787/omgp/v1/hello"
        );
    }
}
