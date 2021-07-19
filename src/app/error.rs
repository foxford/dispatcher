use std::error::Error as StdError;
use std::fmt;

use slog::Logger;
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
    WebinarNotFound,
    RecordingNotFound,
    ClassClosingFailed,
    TranscodingFlowFailed,
    EditionFailed,
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
            ErrorKind::WebinarNotFound => ErrorKindProperties {
                status: ResponseStatus::NOT_FOUND,
                kind: "webinar_not_found",
                title: "Webinar not found",
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
        }
    }
}

////////////////////////////////////////////////////////////////////////////////

pub struct Error {
    kind: ErrorKind,
    source: Box<dyn AsRef<dyn StdError + Send + Sync + 'static> + Send + Sync + 'static>,
}

impl Error {
    pub fn new<E>(kind: ErrorKind, source: E) -> Self
    where
        E: AsRef<dyn StdError + Send + Sync + 'static> + Send + Sync + 'static,
    {
        Self {
            kind,
            source: Box::new(source),
        }
    }

    pub fn to_svc_error(&self) -> SvcError {
        let properties: ErrorKindProperties = self.kind.into();

        SvcError::builder()
            .status(properties.status)
            .kind(properties.kind, properties.title)
            .detail(&self.source.as_ref().as_ref().to_string())
            .build()
    }

    pub fn to_tide_response(&self) -> tide::Response {
        let properties: ErrorKindProperties = self.kind.into();
        let body = serde_json::to_string(&self.to_svc_error()).expect("Infallible");
        tide::Response::builder(properties.status.as_u16())
            .body(body.as_str())
            .build()
    }

    pub fn notify_sentry(&self, logger: &Logger) {
        if !self.kind.is_notify_sentry() {
            return;
        }

        sentry::send(self.to_svc_error()).unwrap_or_else(|err| {
            warn!(logger, "Error sending error to Sentry: {}", err);
        });
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Error")
            .field("kind", &self.kind)
            .field("source", &self.source.as_ref().as_ref())
            .finish()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind, self.source.as_ref().as_ref())
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(self.source.as_ref().as_ref())
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
            source: Box::new(anyhow::Error::from(source)),
        }
    }
}

////////////////////////////////////////////////////////////////////////////////

pub trait ErrorExt<T> {
    fn error(self, kind: ErrorKind) -> Result<T, Error>;
}

impl<T, E: AsRef<dyn StdError + Send + Sync + 'static> + Send + Sync + 'static> ErrorExt<T>
    for Result<T, E>
{
    fn error(self, kind: ErrorKind) -> Result<T, Error> {
        self.map_err(|source| Error::new(kind, source))
    }
}
