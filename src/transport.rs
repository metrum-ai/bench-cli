// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Injectable HTTP transport used by modality implementations and tests.

use reqwest::{Client, Request, Response};
use std::future::Future;

pub trait Transport: Send + Sync {
    fn send(
        &self,
        request: Request,
    ) -> impl Future<Output = Result<Response, reqwest::Error>> + Send;
}

#[derive(Clone, Debug)]
pub struct ReqwestTransport {
    client: Client,
}

impl ReqwestTransport {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

impl Transport for ReqwestTransport {
    async fn send(&self, request: Request) -> Result<Response, reqwest::Error> {
        self.client.execute(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_transport<T: Transport>() {}

    #[test]
    fn reqwest_transport_implements_contract() {
        assert_transport::<ReqwestTransport>();
    }
}
