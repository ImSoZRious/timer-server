use crate::{
    error::{Error, Result},
    payload::{AdminNotice, FinalTimeChangeEvent},
    room::User,
};
use axum::{
    extract::{
        ws::{Message, WebSocket},
        State,
    },
    response::IntoResponse,
    routing::get,
    Server,
};
use futures_util::{stream::SplitSink, StreamExt};
use log::*;
use payload::Payload;
use room::App;
use std::{net::SocketAddr, sync::Arc, time::SystemTime};
use tokio::sync::Mutex;

mod error;
mod payload;
mod room;

type Writer = SplitSink<WebSocket, Message>;

async fn handle_user_message(user: Arc<User<Writer>>, _app: Arc<App<Writer>>, payload: String) {
    use Payload::*;
    let payload: Payload = match serde_json::from_str(&payload) {
        Ok(x) => x,
        Err(..) => {
            user.send(Error::BadPayload).await;
            return;
        }
    };

    log::debug!("recv {payload:?}");

    match payload {
        FinalTimeChangeEvent(payload) => {
            if let Some(room) = user.get_current_room().await {
                room.set_current_final_time(payload.new_final_time).await;
                room.broadcast(payload).await;
            }
        }
        PauseEvent(payload) => {
            if let Some(room) = user.get_current_room().await {
                let now = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis();
                room.pause(now).await;
                room.broadcast(payload).await;
            }
        }
        ResumeEvent(..) => {
            if let Some(room) = user.get_current_room().await {
                room.resume().await;
                let current_time = room.get_current_final_time().await;
                let payload = payload::ResumeEvent::new(Some(current_time));
                room.broadcast(payload).await;
            }
        }
        ResetEvent(payload) => {
            if let Some(room) = user.get_current_room().await {
                room.set_current_final_time(0).await;
                room.broadcast(payload).await;
            }
        }
        SetNoStartEvent(payload) => {
            if let Some(room) = user.get_current_room().await {
                let pause_start_time = payload.new_final_time;
                room.pause(pause_start_time).await;
                room.set_current_final_time(payload.new_final_time).await;
                room.broadcast(payload).await;
            }
        }
        _ => (),
    }
}

async fn handle_connection(ws_stream: WebSocket, app: Arc<App<Writer>>) -> Result<()> {
    use axum::extract::ws::Message::{Close, Text};

    let (tx, mut rx) = ws_stream.split();

    let tx = Mutex::new(tx);

    let user: Arc<User<Writer>>;

    // handshake
    if let Some(msg) = rx.next().await {
        let msg = msg?;

        match msg {
            Text(payload) => {
                let payload::Handshake { user_id, room_id } = match serde_json::from_str(&payload) {
                    Ok(x) => x,
                    Err(_) => return Err(Error::UnexpectedPayload("handshake")),
                };

                info!("{user_id} joins {room_id}");

                user = app.create_user(user_id, tx);

                let room = app.get_room(&room_id).unwrap_or_else(|| {
                    info!("Creating new room {room_id}");
                    app.create_room(room_id.clone())
                });

                app.bind_user_room(Arc::clone(&user), Arc::clone(&room))
                    .await;

                if room.admin_count() == 0 {
                    room.promote(user.id());
                    user.send(AdminNotice::new()).await;
                }

                user.send(FinalTimeChangeEvent::from_u128(
                    room.get_current_final_time().await,
                ))
                .await;
            }
            _ => {
                return Err(Error::InvalidPayloadType);
            }
        }
    } else {
        return Err(Error::ConnectionClosed);
    }

    loop {
        let msg = rx.next().await;

        match msg {
            Some(Ok(Text(payload))) => {
                handle_user_message(Arc::clone(&user), Arc::clone(&app), payload).await;
            }
            None | Some(Ok(Close(..))) => {
                if let Some(room) = user.get_current_room().await {
                    info!(
                        "User {user_id} leaves room {room_id:?}",
                        user_id = user.id(),
                        room_id = room.id()
                    );
                    app.unbind_user_room(Arc::clone(&user), room).await;
                } else {
                    info!("User {user_id} leaves", user_id = user.id());
                }
                app.remove_user(user.id()).await;
                return Err(Error::ConnectionClosed);
            }
            Some(Ok(..)) => {
                user.send(Error::InvalidPayloadType).await;
            }
            _ => return Err(Error::Unknown),
        }
    }
}

async fn handle_ws(
    ws: axum::extract::ws::WebSocketUpgrade,
    State(app): State<Arc<App<Writer>>>,
) -> impl IntoResponse {
    ws.on_failed_upgrade(|error| {
        warn!("unable to upgrade socket {error:?}");
    })
    .on_upgrade(move |socket| async {
        if let Err(e) = handle_connection(socket, app).await {
            match e {
                Error::ConnectionClosed => {
                    info!("Connection close grace fully");
                }
                _ => {
                    warn!("Connection close with reason {e:?}");
                }
            }
        }
    })
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let app = Arc::new(App::new());

    let port = std::env::var("PORT").expect("`PORT` to be set");

    let addr = format!("0.0.0.0:{port}");

    let cors = tower_http::cors::CorsLayer::new()
        .allow_methods(tower_http::cors::Any)
        .allow_origin(tower_http::cors::Any);

    let app = axum::Router::new()
        .route("/", get(|| async { "OK" }))
        .route("/ws", get(handle_ws))
        .with_state(app)
        .layer(cors);

    info!("Start listening on {addr}");
    _ = Server::bind(&addr.parse::<SocketAddr>().unwrap())
        .serve(app.into_make_service())
        .await;
    info!("Server closed")
}
