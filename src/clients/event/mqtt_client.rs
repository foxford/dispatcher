#![allow(clippy::type_complexity)]
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use serde_json::Value as JsonValue;
use svc_agent::{
    error::Error as AgentError,
    mqtt::{
        OutgoingMessage, OutgoingRequest, OutgoingRequestProperties, ShortTermTimingProperties,
        SubscriptionTopic,
    },
    request::Dispatcher,
    AccountId, AgentId, Subscription,
};
use tower::ServiceBuilder;

use super::layer::{
    LogLayer, LogMiddleware, MetricsLayer, MetricsMiddleware, ToJsonLayer, ToJsonMiddleware,
};
use super::{generate_correlation_data, ClientError, MqttRequest};

pub struct JsonMqttRequest {
    method: &'static str,
    payload: JsonValue,
}

impl JsonMqttRequest {
    pub fn method(&self) -> &'static str {
        self.method
    }

    fn payload(self) -> JsonValue {
        self.payload
    }
}

impl<T> From<T> for JsonMqttRequest
where
    T: MqttRequest,
{
    fn from(v: T) -> Self {
        JsonMqttRequest {
            method: v.method(),
            payload: v.payload(),
        }
    }
}

#[derive(Clone)]
struct ClientSettings {
    me: AgentId,
    event_account_id: AccountId,
    api_version: String,
}

#[derive(Clone)]
pub struct MqttBaseClient {
    dispatcher: Arc<Dispatcher>,
    settings: Arc<ClientSettings>,
}

impl MqttBaseClient {
    pub fn new(
        me: AgentId,
        event_account_id: AccountId,
        dispatcher: Arc<Dispatcher>,
        api_version: &str,
    ) -> Self {
        let settings = Arc::new(ClientSettings {
            me,
            event_account_id,
            api_version: api_version.to_owned(),
        });

        Self {
            dispatcher,
            settings,
        }
    }

    fn build_message(
        &self,
        req: JsonMqttRequest,
    ) -> Result<OutgoingRequest<JsonValue>, ClientError> {
        let method = req.method();
        let payload = req.payload();
        let reqp = self.build_reqp(method)?;

        let msg = if let OutgoingMessage::Request(msg) = OutgoingRequest::multicast(
            payload,
            reqp,
            &self.settings.event_account_id,
            &self.settings.api_version,
        ) {
            msg
        } else {
            unreachable!()
        };

        Ok(msg)
    }

    fn response_topic(&self) -> Result<String, ClientError> {
        let me = self.settings.me.clone();

        Subscription::unicast_responses_from(&self.settings.event_account_id)
            .subscription_topic(&me, &self.settings.api_version)
            .map_err(|e| AgentError::new(&e.to_string()).into())
    }

    fn build_reqp(&self, method: &str) -> Result<OutgoingRequestProperties, ClientError> {
        let reqp = OutgoingRequestProperties::new(
            method,
            &self.response_topic()?,
            &generate_correlation_data(),
            ShortTermTimingProperties::new(Utc::now()),
        );

        Ok(reqp)
    }
}

impl tower::Service<JsonMqttRequest> for MqttBaseClient {
    type Response = JsonValue;
    type Error = ClientError;
    type Future = Pin<
        Box<
            dyn futures::Future<Output = Result<Self::Response, Self::Error>>
                + Send
                + Sync
                + 'static,
        >,
    >;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: JsonMqttRequest) -> Self::Future {
        match self.build_message(req) {
            Err(e) => Box::pin(async move { Err(e) }),
            Ok(msg) => {
                let dispatcher = self.dispatcher.clone();
                Box::pin(async move {
                    let request = dispatcher.request::<_, JsonValue>(msg);
                    let payload_result = request.await;
                    let payload =
                        payload_result.map_err(|e| ClientError::Payload(e.to_string()))?;

                    let data = payload.extract_payload();
                    Ok(data)
                })
            }
        }
    }
}

#[derive(Clone)]
pub struct MqttClient<R: MqttRequest> {
    r: PhantomData<R>,
    client: LogMiddleware<
        MetricsMiddleware<
            ToJsonMiddleware<
                tower::util::MapErr<
                    tower::timeout::Timeout<MqttBaseClient>,
                    fn(tower::BoxError) -> ClientError,
                >,
            >,
        >,
    >,
}

impl<R: MqttRequest> MqttClient<R> {
    pub fn new(timeout: Option<Duration>, svc: MqttBaseClient) -> Self {
        let client = ServiceBuilder::new()
            .layer(LogLayer::new("event"))
            .layer(MetricsLayer::new("event"))
            .layer(ToJsonLayer::new())
            .layer(tower::util::MapErrLayer::new(
                map_err as fn(tower::BoxError) -> ClientError,
            ))
            .layer(tower::timeout::TimeoutLayer::new(
                timeout.unwrap_or_else(|| Duration::from_secs(30)),
            ))
            .service(svc);

        Self {
            client,
            r: PhantomData,
        }
    }
}

fn map_err(_e: tower::BoxError) -> ClientError {
    ClientError::Timeout
}

impl<R> tower::Service<R> for MqttClient<R>
where
    R: MqttRequest + 'static,
{
    type Response = <R as MqttRequest>::Response;
    type Error = ClientError;
    type Future = Pin<
        Box<
            dyn futures::Future<Output = Result<Self::Response, Self::Error>>
                + Send
                + Sync
                + 'static,
        >,
    >;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        tower::Service::<R>::poll_ready(&mut self.client, cx)
    }

    fn call(&mut self, request: R) -> Self::Future {
        self.client.call(request)
    }
}
