use std::{
    sync::{Arc, Weak},
    time::SystemTime,
};

use dashmap::DashMap;
use futures_util::{Sink, SinkExt};
use log::*;
use tokio::sync::{Mutex, RwLock};
use axum::extract::ws::Message;

pub struct App<T> {
    room_map: DashMap<String, Arc<Room<T>>>,
    user_map: DashMap<String, Arc<User<T>>>,
}

pub struct Room<T> {
    id: String,
    users: DashMap<String, Weak<User<T>>>,
    current_final_time: RwLock<u64>,
    admins: DashMap<String, Weak<User<T>>>,
}

pub struct User<T> {
    id: String,
    writer: Mutex<T>,
    room: Mutex<Option<Weak<Room<T>>>>,
}

impl<T> App<T> {
    pub fn new() -> Self {
        Self {
            room_map: DashMap::new(),
            user_map: DashMap::new(),
        }
    }

    pub fn create_user(&self, user_id: String, tx: Mutex<T>) -> Arc<User<T>>
    where
        T: Sink<Message>,
    {
        let user = Arc::new(User::new(user_id.clone(), tx));
        self.user_map.insert(user_id, Arc::clone(&user));
        user
    }

    pub fn create_room(&self, room_id: String) -> Arc<Room<T>> {
        let room = Arc::new(Room::new(room_id.clone()));
        self.room_map.insert(room_id, Arc::clone(&room));
        room
    }

    pub fn get_user(&self, user_id: &str) -> Option<Arc<User<T>>> {
        self.user_map
            .get(user_id)
            .map(|user_ref| Arc::clone(&user_ref))
    }

    pub async fn remove_user(&self, user_id: &str) {
        let user = self.user_map.remove(user_id).map(|x| x.1);

        if let Some(user) = user {
            let lock = user.room.lock().await;

            if let Some(room) = lock.as_ref().and_then(Weak::upgrade) {
                room.remove_user(user_id);
            }
        }
    }

    pub async fn _remove_room(&self, room_id: &str) {
        let room = self.room_map.remove(room_id).map(|x| x.1);

        if let Some(room) = room {
            for user in room
                .users
                .iter()
                .chain(room.admins.iter())
                .filter_map(|user| user.upgrade())
            {
                user.leave().await;
            }
        }
    }

    pub fn get_room(&self, room_id: &str) -> Option<Arc<Room<T>>> {
        self.room_map
            .get(room_id)
            .map(|room_ref| Arc::clone(&room_ref))
    }

    pub async fn bind_user_room(&self, user: Arc<User<T>>, room: Arc<Room<T>>)
    where
        T: Sink<Message>,
    {
        user.join(Arc::clone(&room)).await;
        room.add_user(user.id.clone(), Arc::downgrade(&user));
    }

    pub async fn unbind_user_room(&self, user: Arc<User<T>>, room: Arc<Room<T>>) {
        user.leave().await;
        room.remove_user(&user.id);
        room.remove_admin(&user.id);
    }
}

impl<T> Room<T> {
    pub fn new(room_id: String) -> Self {
        Self {
            id: room_id,
            users: DashMap::new(),
            current_final_time: RwLock::new(
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            ),
            admins: DashMap::new(),
        }
    }

    pub async fn broadcast<U>(&self, payload: U)
    where
        T: Sink<Message> + Unpin,
        <T as Sink<Message>>::Error: std::error::Error,
        U: Into<Message> + Clone,
    {
        // todo: make this async
        for user in self.users.iter().chain(self.admins.iter()) {
            if let Some(user) = user.upgrade() {
                user.send(payload.clone()).await;
            }
        }
    }

    pub async fn get_current_final_time(&self) -> u64 {
        *self.current_final_time.read().await
    }

    pub async fn set_current_final_time(&self, time: u64) {
        *self.current_final_time.write().await = time
    }

    pub fn add_user(&self, user_id: String, user: Weak<User<T>>)
    where
        T: Sink<Message>,
    {
        self.users.insert(user_id, user);
    }

    pub fn add_admin(&self, user_id: String, user: Weak<User<T>>)
    where
        T: Sink<Message>,
    {
        self.admins.insert(user_id, user);
    }

    pub fn promote(&self, user_id: &str)
    where
        T: Sink<Message>,
    {
        if let Some((user_id, user)) = self.users.remove(user_id) {
            self.add_admin(user_id, user)
        }
    }

    pub fn user_count(&self) -> usize {
        self.users.len()
    }

    pub fn admin_count(&self) -> usize {
        self.admins.len()
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    fn remove_user(&self, user_id: &str) -> Option<Weak<User<T>>> {
        self.users.remove(user_id).map(|x| x.1)
    }

    fn remove_admin(&self, user_id: &str) -> Option<Weak<User<T>>> {
        self.admins.remove(user_id).map(|x| x.1)
    }
}

impl<T> User<T> {
    pub fn new(user_id: String, tx: Mutex<T>) -> Self
    where
        T: Sink<Message>,
    {
        Self {
            id: user_id,
            writer: tx,
            room: Mutex::new(None),
        }
    }

    pub async fn send<U>(&self, payload: U)
    where
        T: Sink<Message> + Unpin,
        <T as Sink<Message>>::Error: std::error::Error,
        U: Into<Message>,
    {
        let mut writer = self.writer.lock().await;

        if let Err(e) = writer.send(payload.into()).await {
            warn!("unable to send message to {}: {e:?}", self.id);
            let room_lock = self.room.lock().await;
            if let Some(room) = room_lock.as_ref().and_then(Weak::upgrade) {
                room.remove_user(&self.id);
            }
            drop(room_lock);
            self.leave().await;
        }

        drop(writer)
    }

    pub async fn get_current_room(&self) -> Option<Arc<Room<T>>> {
        self.room.lock().await.as_ref().and_then(Weak::upgrade)
    }

    pub async fn leave(&self) {
        let mut lock = self.room.lock().await;
        *lock = None;
    }

    pub async fn join(&self, room: Arc<Room<T>>) {
        let mut lock = self.room.lock().await;
        *lock = Some(Arc::downgrade(&room));
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

#[cfg(test)]
mod tests {
    use std::{convert::Infallible, task::Poll};

    use crate::payload::FinalTimeChangeEvent;

    use super::*;
    use futures::channel::mpsc;
    use futures::StreamExt;
    use futures_util::poll;
    use uuid::Uuid;

    fn random_id() -> String {
        Uuid::new_v4().to_string()
    }

    struct BlackHole;

    impl<T> Sink<T> for BlackHole {
        type Error = Infallible;

        fn poll_close(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            unimplemented!()
        }
        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            unimplemented!()
        }
        fn poll_ready(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            unimplemented!()
        }
        fn start_send(self: std::pin::Pin<&mut Self>, _item: T) -> Result<(), Self::Error> {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn test_one_room_one_user() {
        let app = App::new();

        let room = app.create_room(random_id());

        let (tx, mut rx) = mpsc::unbounded();
        let tx = Mutex::new(tx);

        let user = app.create_user(random_id(), tx);

        app.bind_user_room(user, Arc::clone(&room)).await;

        assert_eq!(poll!(rx.next()), Poll::Pending);

        room.broadcast(FinalTimeChangeEvent::from_u64(180)).await;

        assert_eq!(
            poll!(rx.next()),
            Poll::Ready(Some(Message::Text(
                r#"{"type":"final_time_change","data":{"new_final_time":180}}"#.to_string()
            )))
        );
        assert_eq!(poll!(rx.next()), Poll::Pending);

        rx.close();

        assert_eq!(poll!(rx.next()), Poll::Ready(None));
        // make sure that it'll return None every time
        assert_eq!(poll!(rx.next()), Poll::Ready(None));
    }

    #[tokio::test]
    async fn test_one_room_many_users() {
        let app = App::new();

        let room = app.create_room(random_id());

        let (tx1, mut rx1) = mpsc::unbounded();
        let tx1 = Mutex::new(tx1);
        let (tx2, mut rx2) = mpsc::unbounded();
        let tx2 = Mutex::new(tx2);
        let (tx3, mut rx3) = mpsc::unbounded();
        let tx3 = Mutex::new(tx3);

        let user1 = app.create_user(random_id(), tx1);
        let user2 = app.create_user(random_id(), tx2);
        let user3 = app.create_user(random_id(), tx3);

        app.bind_user_room(user1, Arc::clone(&room)).await;
        app.bind_user_room(user2, Arc::clone(&room)).await;
        app.bind_user_room(user3, Arc::clone(&room)).await;

        assert_eq!(poll!(rx1.next()), Poll::Pending);
        assert_eq!(poll!(rx2.next()), Poll::Pending);
        assert_eq!(poll!(rx3.next()), Poll::Pending);

        Arc::clone(&room)
            .broadcast(FinalTimeChangeEvent::from_u64(180))
            .await;
        Arc::clone(&room)
            .broadcast(FinalTimeChangeEvent::from_u64(1956))
            .await;

        assert_eq!(
            poll!(rx1.next()),
            Poll::Ready(Some(Message::Text(
                r#"{"type":"final_time_change","data":{"new_final_time":180}}"#.to_string()
            )))
        );
        assert_eq!(
            poll!(rx2.next()),
            Poll::Ready(Some(Message::Text(
                r#"{"type":"final_time_change","data":{"new_final_time":180}}"#.to_string()
            )))
        );
        assert_eq!(
            poll!(rx3.next()),
            Poll::Ready(Some(Message::Text(
                r#"{"type":"final_time_change","data":{"new_final_time":180}}"#.to_string()
            )))
        );
        assert_eq!(
            poll!(rx1.next()),
            Poll::Ready(Some(Message::Text(
                r#"{"type":"final_time_change","data":{"new_final_time":1956}}"#.to_string()
            )))
        );
        assert_eq!(
            poll!(rx2.next()),
            Poll::Ready(Some(Message::Text(
                r#"{"type":"final_time_change","data":{"new_final_time":1956}}"#.to_string()
            )))
        );
        assert_eq!(
            poll!(rx3.next()),
            Poll::Ready(Some(Message::Text(
                r#"{"type":"final_time_change","data":{"new_final_time":1956}}"#.to_string()
            )))
        );
        assert_eq!(poll!(rx1.next()), Poll::Pending);
        assert_eq!(poll!(rx2.next()), Poll::Pending);
        assert_eq!(poll!(rx3.next()), Poll::Pending);

        rx1.close();
        rx2.close();
        rx3.close();

        assert_eq!(poll!(rx1.next()), Poll::Ready(None));
        assert_eq!(poll!(rx2.next()), Poll::Ready(None));
        assert_eq!(poll!(rx3.next()), Poll::Ready(None));
        // make sure that it'll return None every time
        assert_eq!(poll!(rx1.next()), Poll::Ready(None));
        assert_eq!(poll!(rx2.next()), Poll::Ready(None));
        assert_eq!(poll!(rx3.next()), Poll::Ready(None));
    }

    #[tokio::test]
    async fn test_many_rooms_many_users() {
        let app = App::new();

        let (tx1, mut rx1) = mpsc::unbounded();
        let tx1 = Mutex::new(tx1);
        let (tx2, mut rx2) = mpsc::unbounded();
        let tx2 = Mutex::new(tx2);
        let (tx3, mut rx3) = mpsc::unbounded();
        let tx3 = Mutex::new(tx3);

        let user1 = app.create_user(random_id(), tx1);
        let user2 = app.create_user(random_id(), tx2);
        let user3 = app.create_user(random_id(), tx3);

        let room1 = app.create_room(random_id());
        let room2 = app.create_room(random_id());

        app.bind_user_room(user1, Arc::clone(&room1)).await;
        app.bind_user_room(user2, Arc::clone(&room2)).await;
        app.bind_user_room(Arc::clone(&user3), Arc::clone(&room2))
            .await;

        room2.promote(user3.id());

        assert_eq!(poll!(rx1.next()), Poll::Pending);
        assert_eq!(poll!(rx2.next()), Poll::Pending);
        assert_eq!(poll!(rx3.next()), Poll::Pending);

        Arc::clone(&room1)
            .broadcast(FinalTimeChangeEvent::from_u64(180))
            .await;
        Arc::clone(&room2)
            .broadcast(FinalTimeChangeEvent::from_u64(420))
            .await;
        Arc::clone(&room2)
            .broadcast(FinalTimeChangeEvent::from_u64(345453))
            .await;

        assert_eq!(
            poll!(rx1.next()),
            Poll::Ready(Some(Message::Text(
                r#"{"type":"final_time_change","data":{"new_final_time":180}}"#.to_string()
            )))
        );
        assert_eq!(
            poll!(rx2.next()),
            Poll::Ready(Some(Message::Text(
                r#"{"type":"final_time_change","data":{"new_final_time":420}}"#.to_string()
            )))
        );
        assert_eq!(
            poll!(rx3.next()),
            Poll::Ready(Some(Message::Text(
                r#"{"type":"final_time_change","data":{"new_final_time":420}}"#.to_string()
            )))
        );

        assert_eq!(poll!(rx1.next()), Poll::Pending);
        assert_eq!(
            poll!(rx2.next()),
            Poll::Ready(Some(Message::Text(
                r#"{"type":"final_time_change","data":{"new_final_time":345453}}"#.to_string()
            )))
        );
        assert_eq!(
            poll!(rx3.next()),
            Poll::Ready(Some(Message::Text(
                r#"{"type":"final_time_change","data":{"new_final_time":345453}}"#.to_string()
            )))
        );

        assert_eq!(poll!(rx1.next()), Poll::Pending);
        assert_eq!(poll!(rx2.next()), Poll::Pending);
        assert_eq!(poll!(rx3.next()), Poll::Pending);

        rx1.close();
        rx2.close();
        rx3.close();

        assert_eq!(poll!(rx1.next()), Poll::Ready(None));
        assert_eq!(poll!(rx2.next()), Poll::Ready(None));
        assert_eq!(poll!(rx3.next()), Poll::Ready(None));
        // make sure that it'll return None every time
        assert_eq!(poll!(rx1.next()), Poll::Ready(None));
        assert_eq!(poll!(rx2.next()), Poll::Ready(None));
        assert_eq!(poll!(rx3.next()), Poll::Ready(None));
    }

    #[tokio::test]
    async fn test_user_getter() {
        let user1_id = random_id();
        let user2_id = random_id();
        let user3_id = random_id();

        let app = App::new();

        let user1 = app.create_user(user1_id.clone(), Mutex::new(BlackHole));
        let user2 = app.create_user(user2_id.clone(), Mutex::new(BlackHole));
        let user3 = app.create_user(user3_id.clone(), Mutex::new(BlackHole));

        assert_eq!(
            Arc::as_ptr(&user1),
            Arc::as_ptr(&app.get_user(&user1_id).unwrap())
        );
        assert_eq!(
            Arc::as_ptr(&user2),
            Arc::as_ptr(&app.get_user(&user2_id).unwrap())
        );
        assert_eq!(
            Arc::as_ptr(&user3),
            Arc::as_ptr(&app.get_user(&user3_id).unwrap())
        );
    }

    #[tokio::test]
    async fn test_room_get_set_final_time() {
        let app = App::<BlackHole>::new();

        let room_id = random_id();

        let room = app.create_room(room_id);

        room.set_current_final_time(5555).await;

        assert_eq!(room.get_current_final_time().await, 5555);
    }
}
