use anyhow::bail;
use cgmath::{Vector2, Vector3};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, net::SocketAddr, time::Duration};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
    select,
    sync::mpsc::{error::TryRecvError, unbounded_channel, UnboundedReceiver, UnboundedSender},
    time::MissedTickBehavior,
};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Circle {
    pub position: Vector2<f32>,
    pub color: Vector3<f32>,
    pub radius: f32,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ClientToServerMessage {
    Disconnect,
    Ping,
    PlayerChanged(Circle),
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ServerToClientMessage {
    Handshake(Uuid),
    ClientConnected(Uuid),
    ClientDisconnected(Uuid),
    Ping,
    PlayerChanged(Uuid, Circle),
}

pub struct Client {
    uuid: Uuid,
    to_server_messages: UnboundedSender<(ClientToServerMessage, Uuid)>,
    from_server_messages: UnboundedReceiver<ServerToClientMessage>,
}

#[derive(Debug, Error)]
#[error("the server has disconnected")]
pub struct Disconnected;

impl Client {
    pub async fn create_local(addr: SocketAddr) -> anyhow::Result<Self> {
        let (to_server_messages, mut from_clients_messages) = unbounded_channel();
        let (to_client_messages, from_server_messages) = unbounded_channel();

        let listener = TcpListener::bind(addr).await?;

        let uuid = Uuid::new_v4();
        to_client_messages
            .send(ServerToClientMessage::Handshake(uuid))
            .unwrap();
        to_client_messages
            .send(ServerToClientMessage::ClientConnected(uuid))
            .unwrap();

        tokio::spawn({
            let to_server_messages = to_server_messages.clone();
            async move {
                let mut clients = HashMap::from([(uuid, to_client_messages)]);

                async fn handle_client(
                    mut stream: TcpStream,
                    uuid: Uuid,
                    to_server_messages: UnboundedSender<(ClientToServerMessage, Uuid)>,
                    mut from_server_messages: UnboundedReceiver<ServerToClientMessage>,
                ) -> anyhow::Result<()> {
                    let (mut reader, mut writer) = stream.split();

                    'outer: loop {
                        tokio::pin! {
                            let read_message = read_message(&mut reader);
                        }

                        loop {
                            select! {
                                message = from_server_messages.recv() => {
                                    let Some(message) = message else {
                                        break 'outer;
                                    };
                                    write_message(&mut writer, message).await?;
                                }

                                result = &mut read_message => {
                                    let message = result?;
                                    let Ok(()) = to_server_messages.send((message, uuid)) else {
                                        break 'outer;
                                    };
                                    continue 'outer;
                                }
                            }
                        }
                    }

                    stream.shutdown().await?;
                    Ok(())
                }

                async fn handle_message(
                    message: ClientToServerMessage,
                    uuid: Uuid,
                    clients: &mut HashMap<Uuid, UnboundedSender<ServerToClientMessage>>,
                ) {
                    match message {
                        ClientToServerMessage::Disconnect => {
                            clients.remove(&uuid);
                            for client in clients.values() {
                                _ = client.send(ServerToClientMessage::ClientDisconnected(uuid));
                            }
                        }
                        ClientToServerMessage::Ping => {}
                        ClientToServerMessage::PlayerChanged(circle) => {
                            for client in clients.values() {
                                _ = client.send(ServerToClientMessage::PlayerChanged(uuid, circle));
                            }
                        }
                    }
                }

                let mut ping_interval = tokio::time::interval(Duration::from_millis(1000));
                ping_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
                loop {
                    select! {
                        Some((message, uuid)) = from_clients_messages.recv(), if clients.contains_key(&uuid) => {
                            handle_message(message, uuid, &mut clients).await;
                        }

                        Ok((stream, _addr)) = listener.accept() => {
                            let (to_client_messages, from_server_messages) = unbounded_channel();
                            let uuid = Uuid::new_v4();
                            to_client_messages
                                .send(ServerToClientMessage::Handshake(uuid))
                                .unwrap();
                            clients.insert(uuid, to_client_messages);
                            for client in clients.values() {
                                _ = client.send(ServerToClientMessage::ClientConnected(uuid));
                            }
                            tokio::spawn({
                                let to_server_messages = to_server_messages.clone();
                                async move {
                                    match handle_client(stream, uuid, to_server_messages.clone(), from_server_messages).await {
                                        Ok(()) => {}
                                        Err(error) => {
                                            eprintln!("{uuid}: {error}");
                                            _ = to_server_messages.send((ClientToServerMessage::Disconnect, uuid));
                                        }
                                    }
                                }
                            });
                        }

                        _ = ping_interval.tick() => {
                            for client in clients.values() {
                                _ = client.send(ServerToClientMessage::Ping);
                            }
                        }
                    }
                }
            }
        });

        Ok(Self {
            uuid,
            to_server_messages,
            from_server_messages,
        })
    }

    pub async fn connect(addr: SocketAddr) -> anyhow::Result<Self> {
        let (to_server_messages, from_client_messages) = unbounded_channel();
        let (to_client_messages, from_server_messages) = unbounded_channel();

        async fn handle_client(
            mut stream: TcpStream,
            mut from_client_messages: UnboundedReceiver<(ClientToServerMessage, Uuid)>,
            to_client_messages: UnboundedSender<ServerToClientMessage>,
        ) -> anyhow::Result<()> {
            let (mut reader, mut writer) = stream.split();

            'outer: loop {
                tokio::pin! {
                    let read_message = read_message(&mut reader);
                }

                loop {
                    select! {
                        message = from_client_messages.recv() => {
                            let Some((message, _)) = message else {
                                break 'outer;
                            };
                            write_message(&mut writer, message).await?;
                        }

                        result = &mut read_message => {
                            let message = result?;
                            let Ok(()) = to_client_messages.send(message) else {
                                break 'outer;
                            };
                            continue 'outer;
                        }
                    }
                }
            }

            stream.shutdown().await?;
            Ok(())
        }

        let mut stream = TcpStream::connect(addr).await?;
        let ServerToClientMessage::Handshake(uuid) = read_message(&mut stream).await? else {
            bail!("the first message send wasnt a handshake");
        };
        tokio::spawn(async move {
            match handle_client(stream, from_client_messages, to_client_messages).await {
                Ok(()) => {}
                Err(error) => {
                    println!("{uuid}: {error}");
                }
            }
        });

        Ok(Self {
            uuid,
            to_server_messages,
            from_server_messages,
        })
    }

    pub fn send_message(&self, message: ClientToServerMessage) -> Result<(), Disconnected> {
        self.to_server_messages
            .send((message, self.uuid))
            .map_err(|_| Disconnected)
    }

    pub fn get_message(&mut self) -> Option<Result<ServerToClientMessage, Disconnected>> {
        match self.from_server_messages.try_recv() {
            Ok(message) => Some(Ok(message)),
            Err(TryRecvError::Disconnected) => Some(Err(Disconnected)),
            Err(TryRecvError::Empty) => None,
        }
    }
}

async fn write_message<T>(writer: impl AsyncWrite, message: T) -> anyhow::Result<()>
where
    T: serde::Serialize,
{
    tokio::pin!(writer);

    let mut bytes = vec![];
    ciborium::into_writer(&message, &mut bytes)?;

    writer
        .write_all(&u64::to_be_bytes(bytes.len().try_into()?))
        .await?;
    writer.write_all(&bytes).await?;

    Ok(())
}

async fn read_message<T>(reader: impl AsyncRead) -> anyhow::Result<T>
where
    T: serde::de::DeserializeOwned,
{
    tokio::pin!(reader);

    let mut length_bytes = [0; std::mem::size_of::<u64>()];
    reader.read_exact(&mut length_bytes).await?;
    let length: usize = u64::from_be_bytes(length_bytes).try_into()?;

    let mut bytes = vec![0; length];
    reader.read_exact(bytes.as_mut_slice()).await?;

    Ok(ciborium::from_reader(bytes.as_slice())?)
}
