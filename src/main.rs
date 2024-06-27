use async_tungstenite::tungstenite::{Error, Message};
use aws_config::BehaviorVersion;
use aws_sdk_s3::{presigning::PresigningConfig, primitives::ByteStream, Client};
use bytes::Bytes;
use chrono::Utc;
use futures_util::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use hass_rs::client::{check_if_event, HassClient};
use hass_rs::WSEvent;
use lazy_static::lazy_static;
use serde_json::json;
use std::{env::var, time::Duration};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, mpsc::Receiver, mpsc::Sender};
use tokio::try_join;
use tokio_tungstenite::{connect_async, WebSocketStream};
use tracing::{info, level_filters::LevelFilter, warn};
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt};

lazy_static! {
    static ref TOKEN: String =
        var("HASS_TOKEN").expect("please set up the HASS_TOKEN env variable before running this");
}

const CAMERA_SNAP_URL: &str = "http://10.0.1.100/snap.jpeg";
const HOME_ASSISTANT_WEB_SOCKET_URL: &str = "ws://10.0.1.3:8123/api/websocket";

async fn ws_incoming_messages(
    mut stream: SplitStream<WebSocketStream<impl AsyncRead + AsyncWrite + Unpin>>,
    to_user: Sender<Result<Message, Error>>,
    event_sender: Sender<WSEvent>,
) {
    loop {
        while let Some(message) = stream.next().await {
            // check if it is a WSEvent, if so send to the spawned tokio task, that should handle the event
            // otherwise process the message and respond accordingly
            match check_if_event(&message) {
                Ok(event) => {
                    let _ = event_sender.send(event).await;
                    continue;
                }
                _ => {
                    let _ = to_user.send(message).await;
                    continue;
                }
            }
        }
    }
}

async fn ws_outgoing_messages(
    mut sink: SplitSink<WebSocketStream<impl AsyncRead + AsyncWrite + Unpin>, Message>,
    mut from_user: Receiver<Message>,
) {
    loop {
        match from_user.recv().await {
            Some(msg) => sink.send(msg).await.expect("Failed to send message"),
            None => todo!(),
        }
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .init();

    info!("Connecting to - {}", HOME_ASSISTANT_WEB_SOCKET_URL);
    let (wsclient, _) = connect_async(HOME_ASSISTANT_WEB_SOCKET_URL)
        .await
        .expect("Failed to connect");
    let (sink, stream) = wsclient.split();

    //Channels to recieve the Client Command and send it over to Websocket server
    let (to_gateway, from_user) = mpsc::channel(20);
    //Channels to receive the Response from the Websocket server and send it over to Client
    let (to_user, from_gateway) = mpsc::channel(20);
    //Channel to receive the Event message from Websocket
    let (event_sender, mut event_receiver) = mpsc::channel::<WSEvent>(20);

    // Handle incoming messages in a separate task
    let read_handle = tokio::spawn(ws_incoming_messages(stream, to_user, event_sender));

    // Read from command line and send messages
    let write_handle = tokio::spawn(ws_outgoing_messages(sink, from_user));

    let mut client = HassClient::new(to_gateway, from_gateway);
    info!("Client created\n");

    client
        .auth_with_longlivedtoken(&TOKEN)
        .await
        .expect("Not able to autheticate");

    info!("WebSocket connection and authethication works\n");

    let _x = client
        .subscribe_event("state_changed")
        .await
        .expect("Cannot subscribe to state_changed!");

    let subscriptions = client.subscriptions.clone();

    while let Some(message) = event_receiver.recv().await {
        // process only events you have subscribed to
        match subscriptions.get(&message.id) {
            Some(_) => {
                if message
                    .event
                    .data
                    .entity_id
                    .is_some_and(|x| x.eq("sensor.entrance_doorbell_action"))
                    && message
                        .event
                        .data
                        .new_state
                        .is_some_and(|x| x.state.eq("on"))
                {
                    info!("Ding dong!");

                    let _ = play_ding_dong(&mut client)
                        .await
                        .inspect_err(|err| warn!(err));

                    let _ = send_notification(&mut client)
                        .await
                        .inspect_err(|err| warn!(err));
                }
            }

            None => warn!("Wrong event received: {:?}", message),
        }
    }

    let _ = try_join!(read_handle, write_handle);
}

async fn play_ding_dong(client: &mut HassClient) -> Result<(), String> {
    client
        .call_service(
            "media_player".to_owned(),
            "play_media".to_owned(),
            Some(json!({
                "entity_id": ["media_player.livingroom"],
                "media_content_id":"http://cdn.freesound.org/previews/192/192761_475267-lq.mp3",
                "media_content_type":"music"
            })),
        )
        .await
        .map_err(|err| format!("Failed to play audio notification: {:?}", err))?;

    Ok(())
}

async fn send_notification(client: &mut HassClient) -> Result<(), String> {
    let picture = snap_picture().await.unwrap();
    let url = upload_and_get_public_url(picture).await?;

    client
        .call_service(
            "notify".to_owned(),
            "all_mobile_phones".to_owned(),
            Some(json!({
                "message":"There is someone at the door!",
                "title": "Ding Dong",
                "data":{
                    "push":{
                        "sound":{
                            "name":"RingtoneDucked_US_Haptic.caf",
                            "critical":0,
                            "volume":0.5
                        }
                    },
                    "attachment":{
                        "url": url,
                        "content-type":"jpeg",
                        "hide-thumbnail":false
                    },
                    "actions":[
                        {
                            "action": "URI",
                            "title": "Enhance",
                            "uri": url,
                            "activationMode":"background",
                            "icon":"sfsymbols:plus.magnifyingglass"
                        }
                    ]
                }
            })),
        )
        .await
        .map_err(|err| format!("Failed to send push notification: {:?}", err))?;

    Ok(())
}

async fn snap_picture() -> Result<Bytes, reqwest::Error> {
    let response = reqwest::get(CAMERA_SNAP_URL).await?.error_for_status()?;
    response.bytes().await
}

//public-pushnotification-n7bkv1uxd0s7mbgqkjrhgq
async fn upload_and_get_public_url(bytes: Bytes) -> Result<String, String> {
    let bucket_name = "public-pushnotification-n7bkv1uxd0s7mbgqkjrhgq";
    let key = Utc::now().format("%Y-%m-%d-%H-%M-%S").to_string();

    let config = aws_config::defaults(BehaviorVersion::v2024_03_28())
        .region("eu-west-1")
        .load()
        .await;

    let client = Client::new(&config);
    let bs = ByteStream::from(bytes);

    client
        .put_object()
        .bucket(bucket_name)
        .key(key.clone())
        .content_type("image/jpeg")
        .body(bs)
        .send()
        .await
        .map_err(|err| format!("Failed to save image to s3: {}", err))?;

    let p = client
        .get_object()
        .bucket(bucket_name)
        .key(key)
        .presigned(PresigningConfig::expires_in(Duration::from_secs(60)).unwrap())
        .await
        .map_err(|err| format!("Failed to build pre-signed url: {}", err))?;

    Ok(p.uri().to_owned())
}

#[cfg(test)]
mod tests {

    #[test]
    fn this_is_where_i_would_put_my_tests_if_i_had_any() {
        let condition = true;
        assert!(
            condition,
            "This is where I would put my tests if I had any!"
        );
    }
}
