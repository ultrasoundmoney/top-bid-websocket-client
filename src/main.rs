//! A simple websocket client that connects to the auction feed and logs the top bid updates
use std::time::SystemTime;
use std::{collections::HashMap, sync::Arc, time::Duration};

use alloy_primitives::{Address, FixedBytes, B256, U256};
use futures::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use ssz::Decode;
use tokio::{net::TcpStream, sync::Mutex};
use tokio_tungstenite::{
    connect_async, tungstenite::protocol::Message, MaybeTlsStream, WebSocketStream,
};

const WS_URL: &str = "ws://relay-builders-eu.ultrasound.money/ws/v1/top_bid";

// The public connection is fast, if you want an even faster connection you'll need a token.
// See: https://github.com/ultrasoundmoney/docs/blob/main/direct-auction-connections.md
const BUILDER_ID: Option<&str> = None;
const API_TOKEN: Option<&str> = None;

type BlsPublicKey = FixedBytes<48>;

#[derive(Debug, Clone, PartialEq, ssz_derive::Decode)]
pub struct TopBidUpdate {
    pub timestamp: u64,
    pub slot: u64,
    pub block_number: u64,
    pub block_hash: B256,
    pub parent_hash: B256,
    pub builder_pubkey: BlsPublicKey,
    pub fee_recipient: Address,
    pub value: U256,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let connection = match (BUILDER_ID, API_TOKEN) {
        (Some(builder_id), Some(api_token)) => {
            let req = http::Request::builder()
                .uri(WS_URL)
                .header("x-builder-id", builder_id)
                .header("x-api-token", api_token)
                .header("sec-websocket-key", "foo")
                .header("sec-websocket-version", 13)
                .header("host", "relay-builders-eu.ultrasound.money")
                .header("upgrade", "websocket")
                .header("connection", "upgrade")
                .body(())
                .unwrap();

            connect_async(req).await
        }
        _ => connect_async(WS_URL).await,
    };

    match connection {
        Ok((ws_stream, _)) => {
            tracing::info!("websocket connected");
            let (mut write, mut read) = ws_stream.split();
            let close_signal = tokio::signal::ctrl_c();
            let stats: Arc<Mutex<HashMap<u64, u64>>> = Arc::new(Mutex::new(HashMap::new()));

            tokio::select! {
                () = handle_websocket(&mut read, &mut write, stats.clone()) => (),
                _ = close_signal => {
                    tracing::info!("received SIGINT, closing connection...");
                    // Send a close frame before exiting
                    let _ = write.send(Message::Close(None)).await;

                    // Log stats
                    let stats = stats.lock().await;
                    let slot_count = stats.len();
                    let update_count: u64 = stats.values().sum();
                    let updates_per_slot = update_count / slot_count as u64;

                    tracing::info!(%slot_count,  %updates_per_slot, "top bid update stats");

                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
        Err(error) => {
            tracing::error!(%error, "failed to connect");
        }
    }

    Ok(())
}

async fn handle_websocket(
    read: &mut SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    write: &mut SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
    stats: Arc<Mutex<HashMap<u64, u64>>>,
) {
    loop {
        tokio::select! {
            Some(Ok(msg)) = read.next() => {
                match msg {
                    Message::Binary(bytes) => {
                        let bid = TopBidUpdate::from_ssz_bytes(&bytes).unwrap();

                        let bid_timestamp: i64 = bid.timestamp.try_into().unwrap();
                        let now_timestamp: i64 = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis().try_into().unwrap();
                        // Because events happen extremely close together, clock drift can cause negative latencies.
                        let latency = now_timestamp - bid_timestamp;

                        let mut stats = stats.lock().await;
                        stats.entry(bid.slot).and_modify(|count| *count += 1).or_insert(1);
                        let count = stats.get(&bid.slot).unwrap();

                        tracing::info!("new top bid, latency: {latency}ms, slot_bid_count: {count}, {:#?}", bid);
                    },
                    Message::Ping(ping) => {
                        tracing::debug!("received ping");
                        if write.send(Message::Pong(ping)).await.is_err() {
                            tracing::error!("failed to send pong");
                            break;
                        }
                    }
                    Message::Close(_) => {
                        tracing::info!("connection closed");
                        break;
                    },
                    _ => (),
                }
            },
        }
    }
}
