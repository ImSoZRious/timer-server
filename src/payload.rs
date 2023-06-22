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
    #[serde(rename = "admin_notice")]
    AdminNotice(AdminNotice),
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct FinalTimeChangeEvent {
    pub new_final_time: u64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct AdminNotice {}

#[derive(serde::Deserialize, Clone)]
pub struct Handshake {
    pub user_id: String,
    pub room_id: String,
}

impl FinalTimeChangeEvent {
    pub fn from_u64(new_final_time: u64) -> Self {
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
            .as_secs();
        FinalTimeChangeEvent { new_final_time }
    }
}

impl AdminNotice {
    pub fn new() -> Self {
        AdminNotice {}
    }
}

wrap!(FinalTimeChangeEvent, AdminNotice);
