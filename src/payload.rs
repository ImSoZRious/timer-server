use std::time::{Duration, SystemTime};

macro_rules! wrap {
    ($($t:ident),*) => {
        $(
            impl ::std::convert::From<$t> for ::axum::extract::ws::Message {
                fn from(value: $t) -> Self {
                    Self::Text(::serde_json::to_string(&Payload::$t(value)).unwrap())
                }
            }
        )*
    };
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum Payload {
    #[serde(rename = "final_time_change")]
    FinalTimeChangeEvent(FinalTimeChangeEvent),
    #[serde(rename = "pause")]
    PauseEvent(PauseEvent),
    #[serde(rename = "resume")]
    ResumeEvent(ResumeEvent),
    #[serde(rename = "admin_notice")]
    AdminNotice(AdminNotice),
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct FinalTimeChangeEvent {
    pub new_final_time: u128,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone, Default)]
pub struct AdminNotice {}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct PauseEvent {}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct ResumeEvent {
    // only server will send this attribute
    pub new_final_time: Option<u128>,
}

#[derive(serde::Deserialize, Clone)]
pub struct Handshake {
    pub user_id: String,
    pub room_id: String,
}

impl FinalTimeChangeEvent {
    pub fn from_u128(new_final_time: u128) -> Self {
        FinalTimeChangeEvent { new_final_time }
    }

    pub fn now() -> Self {
        let current_timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("We're traveling through time");
        let extra_duration = Duration::from_secs(65);
        let new_final_time = current_timestamp
            .checked_add(extra_duration)
            .unwrap()
            .as_millis();
        FinalTimeChangeEvent { new_final_time }
    }
}

impl AdminNotice {
    pub fn new() -> Self {
        AdminNotice {}
    }
}

impl ResumeEvent {
    pub fn new(new_final_time: Option<u128>) -> Self {
        Self { new_final_time }
    }
}

wrap!(FinalTimeChangeEvent, AdminNotice, PauseEvent, ResumeEvent);
