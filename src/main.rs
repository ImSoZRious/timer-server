use crate::{
    error::{Error, Result},
    payload::{AdminNotice, FinalTimeChangeEvent},
    room::User,
};
use futures_util::{stream::SplitSink, StreamExt};
use log::*;
use payload::Payload;
use room::App;
use std::{net::SocketAddr, sync::Arc};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::Mutex,
};
use tokio_tungstenite::{accept_async, WebSocketStream};
use tungstenite::Message::{self, Text};

mod error;
mod payload;
mod room;

type Writer = SplitSink<WebSocketStream<TcpStream>, Message>;

async fn handle_user_message(user: Arc<User<Writer>>, _app: Arc<App<Writer>>, payload: String) {
    let payload: Payload = match serde_json::from_str(&payload) {
        Ok(x) => x,
        Err(..) => {
            user.send(Error::BadPayload).await;
            return;
        }
    };

    match payload {
        Payload::FinalTimeChangeEvent(payload) => {
            if let Some(room) = user.get_current_room().await {
                room.set_current_final_time(payload.new_final_time).await;
                room.broadcast(payload).await;
            }
        }
        _ => (),
    }
}

async fn accept_connection(peer: SocketAddr, stream: TcpStream, app: Arc<App<Writer>>) {
    if let Err(e) = handle_connection(peer, stream, app).await {
        match e {
            Error::Tungstenite(
                tungstenite::Error::ConnectionClosed
                | tungstenite::Error::Protocol(_)
                | tungstenite::Error::Utf8,
            ) => (),
            Error::ConnectionClosed => {
                info!("{peer:?} disconnect");
            }
            err => error!("Error processing connection: {}", err),
        }
    }
}

async fn handle_connection(
    peer: SocketAddr,
    stream: TcpStream,
    app: Arc<App<Writer>>,
) -> Result<()> {
    let ws_stream = accept_async(stream).await.expect("Failed to accept");

    let (tx, mut rx) = ws_stream.split();

    let tx = Mutex::new(tx);

    info!("New WebSocket connection: {}", peer);

    let user: Arc<User<Writer>>;

    // handshake
    if let Some(msg) = rx.next().await {
        let msg = msg?;

        match msg {
            Message::Text(payload) => {
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

                user.send(FinalTimeChangeEvent::from_u64(
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
        // tokio::select! {
        let msg = rx.next().await;

        match msg {
            Some(Ok(Text(payload))) => {
                handle_user_message(Arc::clone(&user), Arc::clone(&app), payload).await;
            }
            Some(Ok(..)) => {
                user.send(Error::InvalidPayloadType).await;
            }
            None => {
                if let Some(room) = user.get_current_room().await {
                    app.unbind_user_room(Arc::clone(&user), room).await;
                }
                app.remove_user(user.id()).await;
                return Err(Error::ConnectionClosed);
            }
            _ => return Err(Error::Unknown),
        }
    }
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let app = Arc::new(App::new());

    let port = std::env::var("PORT").expect("`PORT` to be set");

    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr).await.expect("Can't listen");
    info!("Listening on: {}", addr);

    while let Ok((stream, _)) = listener.accept().await {
        let peer = stream
            .peer_addr()
            .expect("connected streams should have a peer address");
        info!("Peer address: {}", peer);

        tokio::spawn(accept_connection(peer, stream, Arc::clone(&app)));
    }
}
