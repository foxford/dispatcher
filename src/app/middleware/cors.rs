use futures::future::BoxFuture;
use http::{header, HeaderValue};
use hyper::{Request, Response};
use std::task::{Context, Poll};
use tower::{Layer, Service};

#[derive(Clone)]
pub struct CorsMiddleware<S> {
    service: S,
}

const ALLOWED_METHODS: HeaderValue = HeaderValue::from_static("GET, OPTIONS, PUT");
const ALLOWED_ORIGIN: HeaderValue = HeaderValue::from_static("*");
const ALLOWED_HEADERS: HeaderValue = HeaderValue::from_static("*");

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for CorsMiddleware<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        // best practice is to clone the inner service like this
        // see https://github.com/tower-rs/tower/issues/547 for details
        let clone = self.service.clone();
        let mut inner = std::mem::replace(&mut self.service, clone);

        Box::pin(async move {
            let mut res: Response<ResBody> = inner.call(req).await?;
            let h = res.headers_mut();
            h.insert(header::ACCESS_CONTROL_ALLOW_METHODS, ALLOWED_METHODS);
            h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, ALLOWED_ORIGIN);
            h.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, ALLOWED_HEADERS);
            Ok(res)
        })
    }
}

#[derive(Debug, Clone)]
pub struct CorsMiddlewareLayer;

impl<S> Layer<S> for CorsMiddlewareLayer {
    type Service = CorsMiddleware<S>;

    fn layer(&self, service: S) -> Self::Service {
        CorsMiddleware { service }
    }
}
