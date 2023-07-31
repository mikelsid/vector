use std::time::Duration;

use chrono::Local;
use futures_util::future::join_all;
use tokio::sync::oneshot;
use vector_api_client::{connect_subscription_client, Client};

use super::{
    dashboard::{init_dashboard, is_tty},
    metrics,
    state::{self, ConnectionStatus, EventType},
};
use crate::config;

/// Delay (in milliseconds) before attempting to reconnect to the Vector API
const RECONNECT_DELAY: u64 = 5000;

/// CLI command func for displaying Vector components, and communicating with a local/remote
/// Vector API server via HTTP/WebSockets
pub async fn cmd(opts: &super::Opts) -> exitcode::ExitCode {
    // Exit early if the terminal is not a teletype
    if !is_tty() {
        #[allow(clippy::print_stderr)]
        {
            eprintln!("Terminal must be a teletype (TTY) to display a Vector dashboard.");
        }
        return exitcode::IOERR;
    }

    // Use the provided URL as the Vector GraphQL API server, or default to the local port
    // provided by the API config. This will work despite `api` and `api-client` being distinct
    // features; the config is available even if `api` is disabled
    let url = opts
        .url
        .clone()
        .unwrap_or_else(config::api::default_graphql_url);

    // Create a new API client for connecting to the local/remote Vector instance.
    let client = Client::new(url.clone());
    if client.healthcheck().await.is_err() {
        eprintln!(
            indoc::indoc! {"
            Vector API server isn't reachable ({}).

            Have you enabled the API?

            To enable the API, add the following to your `vector.toml` config file:

            [api]
                enabled = true"},
            url
        );
        return exitcode::UNAVAILABLE;
    }

    // Create a channel for updating state via event messages
    let (tx, rx) = tokio::sync::mpsc::channel(20);
    let state_rx = state::updater(rx).await;

    // Change the HTTP schema to WebSockets
    let mut ws_url = url.clone();
    ws_url
        .set_scheme(match url.scheme() {
            "https" => "wss",
            _ => "ws",
        })
        .expect("Couldn't build WebSocket URL. Please report.");

    let opts_clone = opts.clone();
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    // This task handles reconnecting the subscription client and all
    // subscriptions in the case of a web socket disconnect
    let connection = tokio::spawn(async move {
        loop {
            // Initialize state. On future reconnects, we re-initialize state in
            // order to accurately capture added, removed, and edited
            // components.
            let state = match metrics::init_components(&client).await {
                Ok(state) => state,
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(RECONNECT_DELAY)).await;
                    continue;
                }
            };
            _ = tx.send(EventType::InitializeState(state)).await;

            let subscription_client = match connect_subscription_client(ws_url.clone()).await {
                Ok(c) => c,
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(RECONNECT_DELAY)).await;
                    continue;
                }
            };

            // Subscribe to updated metrics
            let finished =
                metrics::subscribe(subscription_client, tx.clone(), opts_clone.interval as i64);

            _ = tx
                .send(EventType::ConnectionUpdated(ConnectionStatus::Connected(
                    Local::now(),
                )))
                .await;
            // Tasks spawned in metrics::subscribe finish when the subscription
            // streams have completed. Currently, subscription streams only
            // complete when the underlying web socket connection to the GraphQL
            // server drops.
            _ = join_all(finished).await;
            _ = tx
                .send(EventType::ConnectionUpdated(
                    ConnectionStatus::Disconnected(RECONNECT_DELAY),
                ))
                .await;
            if opts_clone.no_reconnect {
                _ = shutdown_tx.send(());
                break;
            }
        }
    });

    // Initialize the dashboard
    match init_dashboard(url.as_str(), opts, state_rx, shutdown_rx).await {
        Ok(_) => {
            connection.abort();
            exitcode::OK
        }
        Err(err) => {
            #[allow(clippy::print_stderr)]
            {
                eprintln!("Encountered error: {}", err);
            }
            connection.abort();
            exitcode::IOERR
        }
    }
}
