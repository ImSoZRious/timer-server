#[derive(Debug)]
pub(crate) enum Error {
    ConnectionClosed,
    BadPayload,
    UnexpectedPayload(&'static str),
    InvalidPayloadType,
    Axum(axum::Error),
    Unknown,
}

pub(crate) type Result<T> = std::result::Result<T, Error>;

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{:?}", self))
    }
}

impl std::error::Error for Error {}

impl From<axum::Error> for Error {
    fn from(value: axum::Error) -> Self {
        Self::Axum(value)
    }
}

impl From<Error> for axum::extract::ws::Message {
    fn from(value: Error) -> Self {
        Self::Text(format!("Error: {}", value.to_string()))
    }
}
