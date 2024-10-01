// use aws_sdk_s3::{presigning::PresigningConfig, primitives::ByteStream, Client};

use lazy_static::lazy_static;
use std::{env::var, time::Duration};
use tracing::{info, level_filters::LevelFilter};
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt};

lazy_static! {
    static ref TOKEN: String =
        var("HASS_TOKEN").expect("please set up the HASS_TOKEN env variable before running this");
}

const CAMERA_SNAP_URL: &str = "http://10.0.1.100/snap.jpeg";
const HOME_ASSISTANT_WEB_SOCKET_URL: &str = "ws://10.0.1.3:8123/api/websocket";

mod haclient;

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

    let client = haclient::Client::connect(HOME_ASSISTANT_WEB_SOCKET_URL, &TOKEN).await;

    // let mut iterator = client.subscribe_events().await.unwrap();
    let mut iterator = client.subscribe_trigger().await.unwrap();

    loop {
        //for  message in iterator.next().await {
        let message = iterator.next().await;
        info!("Received a message => {:?}", message);
        client.call_service().await;
    }

    info!("Waiting a bit...");
    //await 4 seocnds
    tokio::time::sleep(Duration::from_secs(4)).await;

    info!("Shutting down...");
    // let _ = try_join!(read_handle, write_handle);
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
