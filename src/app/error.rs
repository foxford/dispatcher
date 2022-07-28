use std::fmt;
use std::sync::Arc;

use axum::response::IntoResponse;
use axum::{body::BoxBody, Json};
use hyper::Response;
use svc_agent::mqtt::ResponseStatus;
use svc_error::{extension::sentry, Error as SvcError};

////////////////////////////////////////////////////////////////////////////////

struct ErrorKindProperties {
    status: ResponseStatus,
    kind: &'static str,
    title: &'static str,
    is_notify_sentry: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum ErrorKind {
    AccessDenied,
    AuthorizationFailed,
    DbConnAcquisitionFailed,
    DbQueryFailed,
    InvalidParameter,
    InvalidPayload,
    MqttRequestFailed,
    SerializationFailed,
    Unauthorized,
    ClassNotFound,
    RecordingNotFound,
    ClassClosingFailed,
    TranscodingFlowFailed,
    EditionFailed,
    ClassPropertyNotFound,
    AudienceDoesNotMatch,
}

impl ErrorKind {
    pub fn is_notify_sentry(self) -> bool {
        let properties: ErrorKindProperties = self.into();
        properties.is_notify_sentry
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let properties: ErrorKindProperties = self.to_owned().into();
        write!(f, "{}", properties.title)
    }
}

impl From<ErrorKind> for ErrorKindProperties {
    fn from(k: ErrorKind) -> Self {
        match k {
            ErrorKind::AccessDenied => ErrorKindProperties {
                status: ResponseStatus::FORBIDDEN,
                kind: "access_denied",
                title: "Access denied",
                is_notify_sentry: false,
            },
            ErrorKind::AuthorizationFailed => ErrorKindProperties {
                status: ResponseStatus::UNPROCESSABLE_ENTITY,
                kind: "authorization_failed",
                title: "Authorization failed",
                is_notify_sentry: false,
            },
            ErrorKind::DbConnAcquisitionFailed => ErrorKindProperties {
                status: ResponseStatus::UNPROCESSABLE_ENTITY,
                kind: "database_connection_acquisition_failed",
                title: "Database connection acquisition failed",
                is_notify_sentry: true,
            },
            ErrorKind::DbQueryFailed => ErrorKindProperties {
                status: ResponseStatus::UNPROCESSABLE_ENTITY,
                kind: "database_query_failed",
                title: "Database query failed",
                is_notify_sentry: true,
            },
            ErrorKind::InvalidParameter => ErrorKindProperties {
                status: ResponseStatus::BAD_REQUEST,
                kind: "invalid_parameter",
                title: "Invalid parameter",
                is_notify_sentry: false,
            },
            ErrorKind::InvalidPayload => ErrorKindProperties {
                status: ResponseStatus::BAD_REQUEST,
                kind: "invalid_payload",
                title: "Invalid payload",
                is_notify_sentry: false,
            },
            ErrorKind::ClassNotFound => ErrorKindProperties {
                status: ResponseStatus::NOT_FOUND,
                kind: "class_not_found",
                title: "Class not found",
                is_notify_sentry: false,
            },
            ErrorKind::MqttRequestFailed => ErrorKindProperties {
                status: ResponseStatus::INTERNAL_SERVER_ERROR,
                kind: "mqtt_request_failed",
                title: "Mqtt request failed",
                is_notify_sentry: true,
            },
            ErrorKind::SerializationFailed => ErrorKindProperties {
                status: ResponseStatus::UNPROCESSABLE_ENTITY,
                kind: "serialization_failed",
                title: "Serialization failed",
                is_notify_sentry: true,
            },
            ErrorKind::Unauthorized => ErrorKindProperties {
                status: ResponseStatus::UNAUTHORIZED,
                kind: "authentication_failed",
                title: "Authentication failed",
                is_notify_sentry: false,
            },
            ErrorKind::RecordingNotFound => ErrorKindProperties {
                status: ResponseStatus::NOT_FOUND,
                kind: "recording_not_found",
                title: "Recording not found",
                is_notify_sentry: false,
            },
            ErrorKind::ClassClosingFailed => ErrorKindProperties {
                status: ResponseStatus::UNPROCESSABLE_ENTITY,
                kind: "class_closing_failed",
                title: "Class closing failed",
                is_notify_sentry: true,
            },
            ErrorKind::TranscodingFlowFailed => ErrorKindProperties {
                status: ResponseStatus::UNPROCESSABLE_ENTITY,
                kind: "transcoding_flow_failed",
                title: "Transcoding flow failed",
                is_notify_sentry: true,
            },
            ErrorKind::EditionFailed => ErrorKindProperties {
                status: ResponseStatus::UNPROCESSABLE_ENTITY,
                kind: "edition_flow_failed",
                title: "Edition flow failed",
                is_notify_sentry: true,
            },
            ErrorKind::ClassPropertyNotFound => ErrorKindProperties {
                status: ResponseStatus::NOT_FOUND,
                kind: "class_property_not_found",
                title: "Class property not found",
                is_notify_sentry: false,
            },
            ErrorKind::AudienceDoesNotMatch => ErrorKindProperties {
                status: ResponseStatus::UNPROCESSABLE_ENTITY,
                kind: "audience_does_not_match",
                title: "Audience does not match",
                is_notify_sentry: false,
            },
        }
    }
}

////////////////////////////////////////////////////////////////////////////////

pub struct Error {
    kind: ErrorKind,
    err: Arc<anyhow::Error>,
}

impl Error {
    pub fn new(kind: ErrorKind, err: anyhow::Error) -> Self {
        Self {
            kind,
            err: Arc::new(err),
        }
    }

    pub fn to_svc_error(&self) -> SvcError {
        let properties: ErrorKindProperties = self.kind.into();

        SvcError::builder()
            .status(properties.status)
            .kind(properties.kind, properties.title)
            .detail(&self.err.to_string())
            .build()
    }

    pub fn notify_sentry(&self) {
        if !self.kind.is_notify_sentry() {
            return;
        }

        if let Err(e) = sentry::send(self.err.clone()) {
            tracing::error!("Failed to send error to sentry, reason = {:?}", e);
        }
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response<BoxBody> {
        let properties: ErrorKindProperties = self.kind.into();

        (properties.status, Json(&self.to_svc_error())).into_response()
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Error")
            .field("kind", &self.kind)
            .field("source", &self.err)
            .finish()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind, self.err)
    }
}

impl From<svc_authz::Error> for Error {
    fn from(source: svc_authz::Error) -> Self {
        let kind = match source.kind() {
            svc_authz::ErrorKind::Forbidden(_) => ErrorKind::AccessDenied,
            _ => ErrorKind::AuthorizationFailed,
        };

        Self {
            kind,
            err: Arc::new(source.into()),
        }
    }
}

////////////////////////////////////////////////////////////////////////////////

pub trait ErrorExt<T> {
    fn error(self, kind: ErrorKind) -> Result<T, Error>;
}

impl<T> ErrorExt<T> for Result<T, anyhow::Error> {
    fn error(self, kind: ErrorKind) -> Result<T, Error> {
        self.map_err(|source| Error::new(kind, source))
    }
}
