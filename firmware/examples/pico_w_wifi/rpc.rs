//! The endpoints. Async handlers run one at a time, so joining and leaving never overlap.

use cyw43::JoinOptions;
use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_time::{Duration, with_timeout};
use picolink_firmware::link::{Spawn, Tx};
use picolink_protocol::wifi::{
    ENDPOINT_LIST, JOIN_TIMEOUT_MS, JoinEndpoint, JoinError, JoinRequest, JoinResult, LeaveEndpoint, SerialNumberEndpoint, Ssid, Status,
    StatusEndpoint, TOPICS_IN_LIST, TOPICS_OUT_LIST,
};
use postcard_rpc::{define_dispatch, header::VarHeader};

use crate::wifi::Wifi;

pub struct Context {
    serial_number: u64,
    wifi: &'static Wifi,
    joined: Option<Ssid>,
}

define_dispatch! {
    app: RpcApp;
    spawn_fn: spawn_fn;
    tx_impl: Tx;
    spawn_impl: Spawn;
    context: Context;

    endpoints: {
        list: ENDPOINT_LIST;

        | EndpointTy            | kind      | handler               |
        | ----------            | ----      | -------               |
        | SerialNumberEndpoint  | blocking  | serial_number         |
        | StatusEndpoint        | blocking  | status                |
        | JoinEndpoint          | async     | join                  |
        | LeaveEndpoint         | async     | leave                 |
    };
    topics_in: {
        list: TOPICS_IN_LIST;

        | TopicTy               | kind      | handler               |
        | -------               | ----      | -------               |
    };
    topics_out: {
        list: TOPICS_OUT_LIST;
    };
}

pub fn app(serial_number: u64, wifi: &'static Wifi, spawner: Spawner) -> RpcApp {
    RpcApp::new(
        Context {
            serial_number,
            wifi,
            joined: None,
        },
        spawner.into(),
    )
}

fn serial_number(context: &mut Context, _header: VarHeader, _request: ()) -> u64 {
    context.serial_number
}

fn status(context: &mut Context, _header: VarHeader, _request: ()) -> Status {
    Status {
        joined: context.joined.clone(),
    }
}

/// Waits for a running scan to finish first.
async fn join(context: &mut Context, _header: VarHeader, request: JoinRequest) -> JoinResult {
    // Never log the passphrase.
    info!("Joining {}", request.ssid.as_str());
    let options = match &request.passphrase {
        Some(passphrase) => JoinOptions::new(passphrase.as_bytes()),
        None => JoinOptions::new_open(),
    };
    let mut control = context.wifi.lock().await;
    let result = match with_timeout(Duration::from_millis(JOIN_TIMEOUT_MS), control.join(&request.ssid, options)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(cyw43::JoinError::NetworkNotFound)) => Err(JoinError::NetworkNotFound),
        Ok(Err(cyw43::JoinError::AuthenticationFailure)) => Err(JoinError::AuthenticationFailed),
        Ok(Err(cyw43::JoinError::JoinFailure(status))) => Err(JoinError::Failed(status)),
        Err(_) => Err(JoinError::Timeout),
    };
    match result {
        Ok(()) => {
            info!("Joined {}", request.ssid.as_str());
            context.joined = Some(request.ssid);
        }
        Err(error) => {
            warn!("Joining {} failed: {}", request.ssid.as_str(), error);
            // Joining starts a new association, so an earlier one may be gone too.
            context.joined = None;
        }
    }
    result
}

async fn leave(context: &mut Context, _header: VarHeader, _request: ()) {
    context.wifi.lock().await.leave().await;
    context.joined = None;
}
