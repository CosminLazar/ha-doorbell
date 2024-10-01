use std::sync::atomic::{AtomicUsize, Ordering};

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use tracing::{info, warn};

trait HAIdentifiedCommand {
    fn id(&self) -> u64;
}

enum HACommandFailure {
    CommandFailed,
}

enum HACommandType {
    Subscribe {
        command: Value,
        comand_result_sender: tokio::sync::oneshot::Sender<Result<Value, HACommandFailure>>,
        event_sender: tokio::sync::mpsc::UnboundedSender<Value>,
    },
    Unsubscribe {
        command: Value,
        comand_result_sender: tokio::sync::oneshot::Sender<Result<Value, HACommandFailure>>,
    },
    CallService {
        command: Value,
        comand_result_sender: tokio::sync::oneshot::Sender<Result<Value, HACommandFailure>>,
    },
}

impl HAIdentifiedCommand for HACommandType {
    fn id(&self) -> u64 {
        match self {
            HACommandType::Subscribe { command, .. } => command["id"].as_u64().unwrap(),
            HACommandType::Unsubscribe { command, .. } => command["id"].as_u64().unwrap(),
            HACommandType::CallService { command, .. } => command["id"].as_u64().unwrap(),
        }
    }
}

// struct HomeAssistantCommand {
//     command: HomeAssistantCommandType,
//     responder: tokio::sync::oneshot::Sender<HomeAssistantCommandTypeT>,
// }

impl Client<Connected> {
    fn new(socket: WebSocketStream<MaybeTlsStream<TcpStream>>) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel::<HACommandType>(10);
        tokio::spawn(Self::handle_messages(socket, rx));

        Client {
            state: Connected { tx },
        }
    }

    async fn handle_messages(
        mut socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
        mut rx: tokio::sync::mpsc::Receiver<HACommandType>,
    ) {
        info!("Handling messages in connected state...");

        let mut command_response_map = std::collections::HashMap::<
            u64,
            tokio::sync::oneshot::Sender<Result<Value, HACommandFailure>>,
        >::new();
        let mut subscription_sinks =
            std::collections::HashMap::<u64, tokio::sync::mpsc::UnboundedSender<Value>>::new();

        loop {
            tokio::select! {
                ha_command = rx.recv() => {
                    if let Some(ha_command) = ha_command {
                        let command_id = ha_command.id();
                        match ha_command {
                            HACommandType::Subscribe {command, comand_result_sender, event_sender} => {
                                let socket_status = socket.send(Message::Text(command.to_string())).await;

                                if socket_status.is_ok() {
                                    command_response_map.insert(command_id, comand_result_sender);
                                    subscription_sinks.insert(command_id, event_sender);
                                } else {
                                    let _ = comand_result_sender.send(Err(HACommandFailure::CommandFailed));
                                }
                            }
                            HACommandType::Unsubscribe {command, comand_result_sender} => {
                                let socket_status = socket.send(Message::Text(command.to_string())).await;
                                if socket_status.is_ok() {
                                    command_response_map.insert(command_id, comand_result_sender);
                                } else {
                                    let _ = comand_result_sender.send(Err(HACommandFailure::CommandFailed));
                                }
                            },
                            HACommandType::CallService {command, comand_result_sender} => {
                                let socket_status = socket.send(Message::Text(command.to_string())).await;
                                if socket_status.is_ok() {
                                    command_response_map.insert(command_id, comand_result_sender);
                                } else {
                                    let _ = comand_result_sender.send(Err(HACommandFailure::CommandFailed));
                                }
                            }
                        }
                    }
                    else{
                        info!("Command channel closed, shutting down HA worker task...");
                        break;
                    }
                },
                socket_message = socket.next() => {
                    if socket_message.is_none() {
                        info!("Socket closed, shutting down HA worker task...");
                        break;
                    }

                    let message = socket_message.unwrap();

                    match message {
                        Ok(msg)=> {
                            if let Message::Text(text_msg) = msg {
                                let response:Value = serde_json::from_str(&text_msg).unwrap();
                                let message_id = response.get("id").and_then(|x| x.as_u64());

                                if message_id.is_none() {
                                    warn!("Message without id received, will be ignored: {:?}",response);
                                    continue;
                                }
                                let message_id = message_id.unwrap();

                                match response.get("type"){
                                    Some(message_type) if message_type == "result" => {
                                                info!("Responding to request/response with id: {}", message_id);

                                                if let Some(sender) = command_response_map.remove(&message_id)
                                                {
                                                     let _ = sender.send(Ok(response));
                                                    }
                                    }
                                    Some(message_type) if message_type == "event" => {
                                                if let Some(sender) = subscription_sinks.get(&message_id){
                                                    if sender.send(response).is_err(){
                                                        warn!("Failed to send event to subscriber, removing subscription");
                                                        let _ = subscription_sinks.remove(&message_id);
                                                    }
                                                }
                                                else{
                                                    warn!("Received an event for an unknown subscription: {:?}",response);
                                                }
                                    }
                                    _ => {
                                        warn!("Unknown message type received (only result and event supported): {:?}",response);
                                    }
                                }
                            }
                            else if let Message::Ping(_payload)= msg {
                                info!("Received a ping - do I need to send a pong?");
                                // let _ = socket.send(Message::Pong(payload)).await;
                            }
                            else{
                                warn!("Unknown message type received: {:?}",msg);
                            }
                        }
                        Err(e) => {
                            info!("Error receiving message: {:?}", e);
                            break;
                        }
                    }
                }
            }
        }
    }

    pub async fn subscribe_events(
        &self,
    ) -> Result<EventIterator, tokio::sync::oneshot::error::RecvError> {
        let (register_event_tx, register_event_rx) =
            tokio::sync::oneshot::channel::<Result<Value, HACommandFailure>>();
        let (events_tx, events_rx) = tokio::sync::mpsc::unbounded_channel::<Value>();

        let _ = self
            .state
            .tx
            .send(HACommandType::Subscribe {
                comand_result_sender: register_event_tx,
                event_sender: events_tx,
                command: json!({
                    "id": get_next_id(),
                    "type": "subscribe_events",
                    "event_type": "state_changed"
                }),
            })
            .await;

        let subscription_response = register_event_rx.await.expect("Subscription failed");

        Ok(EventIterator {
            home_assistant_command_tx: self.state.tx.clone(),
            rx: events_rx,
        })
    }

    pub async fn subscribe_trigger(
        &self,
    ) -> Result<EventIterator, tokio::sync::oneshot::error::RecvError> {
        let (register_event_tx, register_event_rx) =
            tokio::sync::oneshot::channel::<Result<Value, HACommandFailure>>();
        let (events_tx, events_rx) = tokio::sync::mpsc::unbounded_channel::<Value>();

        let _fsa = self
            .state
            .tx
            .send(HACommandType::Subscribe {
                comand_result_sender: register_event_tx,
                event_sender: events_tx,
                command: json!({
                    "id": get_next_id(),
                    "type": "subscribe_trigger",
                    "trigger": {
                        "platform": "state",
                        "entity_id": "sensor.entrance_doorbell_action",
                        "from":"on",
                        "to":""
                    }
                }),
            })
            .await;

        let subscription_response = register_event_rx.await.expect("Subscription failed");

        Ok(EventIterator {
            home_assistant_command_tx: self.state.tx.clone(),
            rx: events_rx,
        })
    }

    pub async fn call_service(&self) {
        let (register_event_tx, register_event_rx) =
            tokio::sync::oneshot::channel::<Result<Value, HACommandFailure>>();

        let _fsa = self
            .state
            .tx
            .send(HACommandType::CallService {
                comand_result_sender: register_event_tx,
                command: json!({
                    "id": get_next_id(),
                    "type": "call_service",
                    "domain": "notify",
                    "service": "all_mobile_phones",
                    "service_data":{
                        "message":"There is someone at the door2!",
                        "title": "Ding Dong",
                        "data":{
                            // "entity_id":"camera.entrance_medium_resolution_channel",
                            "push":{
                                "sound":{
                                    "name":"RingtoneDucked_US_Haptic.caf",
                                    "critical":0,
                                    "volume":0.5
                                }
                            },
                            // "attachment":{
                            //     "url": url,
                            //     "content-type":"jpeg",
                            //     // "content-type":"video",
                            //     "hide-thumbnail":false
                            // },
                            "actions":[
                                {
                                    "action": "URI",
                                    "title": "Enhance",
                                    "uri": "TBA",
                                    "activationMode":"background",
                                    "icon":"sfsymbols:plus.magnifyingglass"
                                }
                            ]
                        }
                    }
                }),
            })
            .await;

        let subscription_response = register_event_rx.await.expect("Failed to call service");
    }
}

pub struct EventIterator {
    rx: tokio::sync::mpsc::UnboundedReceiver<Value>,
    home_assistant_command_tx: tokio::sync::mpsc::Sender<HACommandType>,
}

impl EventIterator {
    pub async fn next(&mut self) -> Option<Value> {
        self.rx.recv().await
    }

    pub async fn unsubscribe(self) {
        //unsubscribe here
        //self.home_assistant_command_tx
    }
}

pub struct Init {}
pub struct Connected {
    tx: tokio::sync::mpsc::Sender<HACommandType>,
}

pub(crate) struct Client<S> {
    state: S,
}

impl Client<Init> {
    pub async fn connect(url: &str, token: &str) -> Client<Connected> {
        info!("Connecting to - {}", url);
        let (socket, response) = connect_async(url).await.unwrap();

        info!("Got response: {:?} in Init phase", response);

        Client::new(Self::handle_authentication(token, socket).await)
    }

    async fn handle_authentication(
        token: &str,
        mut socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    ) -> WebSocketStream<MaybeTlsStream<TcpStream>> {
        while let Some(message) = socket.next().await {
            match message {
                Ok(msg) => {
                    let response = Self::deserialize(msg);
                    match response {
                        AnonymousServerResponse::AuthRequired { .. } => {
                            info!(
                                "Received a message that i should authenticate: {:?}",
                                response
                            );
                            let auth = json!({
                                "type": "auth",
                                "access_token": token,
                            });

                            if socket.send(Message::Text(auth.to_string())).await.is_err() {
                                break;
                            }
                        }
                        AnonymousServerResponse::AuthOk { .. } => {
                            info!(
                                "Received a message that authentication was successful: {:?}",
                                response
                            );
                            return socket;
                        }
                        AnonymousServerResponse::AuthInvalid { .. } => {
                            info!(
                                "Received a message that authentication is invalid: {:?}",
                                response
                            );
                            panic!("Invalid auth token, panic for now");
                        }
                    }
                }
                Err(e) => {
                    info!("Error receiving message: {:?}", e);
                    break;
                }
            }
        }

        panic!("I am lazy to handle this");
    }

    fn deserialize(message: Message) -> AnonymousServerResponse {
        if let Message::Text(response) = message {
            return serde_json::from_str(&response).expect("Very lazy");
        }
        dbg!(message);
        panic!("I AM LAZY TO HANDLE THIS");
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type")]
enum AnonymousServerResponse {
    #[serde(rename = "auth_required")]
    AuthRequired { ha_version: String },
    #[serde(rename = "auth_ok")]
    AuthOk { ha_version: String },
    #[serde(rename = "auth_invalid")]
    AuthInvalid { message: String },
}

struct AuthRequired {
    type_: String,
    access_token: String,
}

fn get_next_id() -> usize {
    static COUNTER: AtomicUsize = AtomicUsize::new(1);
    COUNTER.fetch_add(1, Ordering::SeqCst)
}
