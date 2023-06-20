#[derive(Debug)]
pub(crate) enum Error {
    ConnectionClosed,
    BadPayload,
    UnexpectedPayload(&'static str),
    InvalidPayloadType,
    Tungstenite(tungstenite::Error),
    Unknown,
}

pub(crate) type Result<T> = std::result::Result<T, Error>;

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{:?}", self))
    }
}

impl std::error::Error for Error {}

impl From<tungstenite::Error> for Error {
    fn from(value: tungstenite::Error) -> Self {
        Self::Tungstenite(value)
    }
}

impl From<Error> for tungstenite::Message {
    fn from(value: Error) -> Self {
        Self::Text(format!("Error: {}", value.to_string()))
    }
}
