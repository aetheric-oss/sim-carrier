use std::net::UdpSocket;
use mavlink::{ MavFrame, MavHeader, MavlinkVersion};
use hyper::{
    client::connect::HttpConnector,
    client::Client,
};
use std::sync::{Arc, Mutex};
use crate::provision;

// use svc_telemetry_client_rest::netrid_types::*;
use crate::telemetry;

pub async fn mavlink_proxy(
    address: String,
    tlm_uri: String,
    itest_uri: String,
    state_mtx: Arc<Mutex<crate::State>>,
    px4: u32,
    cmd_rx: std::sync::mpsc::Receiver<String>,
) -> Result<(), ()> {
    println!("| {address} | starting mavlink proxy.");
    let socket = UdpSocket::bind(&address).map_err(|e| {
        println!("could not bind to {address}: {e}");
    })?;
    
    let client: Client<HttpConnector> = Client::builder()
        .pool_idle_timeout(std::time::Duration::from_secs(10))
        .build_http();

    let mut provisioned = false;

    let mut token: Option<String> = None;
    let mut buf = [0_u8; 280];

    const RETRIES_MAX: u8 = 3;
    let mut retry: u8 = 0;
        
    loop {
        // get commands
        if let Ok(cmd) = cmd_rx.try_recv() {
            match cmd.as_str() {
                "exit" => {
                    println!("| {address} | exiting mavlink proxy.");
                    return Ok(());
                }
                _ => {
                    println!("| {address} | received unknown command: {cmd}");
                }
            }
        }


        // println!("| {address} | waiting for mavlink frame.");
        let (mut amt, _src) = socket.recv_from(&mut buf).map_err(|e| {
            println!("could not receive from socket: {e}");
        })?;
        let mut s = buf[4..amt].to_vec();
        amt -= 4; // lop off flags and length

        // PX4-Autopilot bug: there's a missing byte in the OPEN_DRONE_ID_LOCATION message
        if (s[3..=5] == [0x65, 0x32, 0x00]) && s.len() == 66 {
            s.insert(amt - 2, 0x00);
            // amt += 1;
        }

        let frame = match MavFrame::deser(
            MavlinkVersion::V2,
            &s
        ) {
            Ok(frame) => frame,
            Err(e) => {
                println!("could not deserialize frame: {}", e);
                continue;
            }
        };

        let identifier = state_mtx
            .lock()
            .map_err(|_| {
                println!("could not lock session id mutex");
            })?
            .session_id
            .clone();

        let tk: String = match token {
            Some(ref t) => t.clone(),
            None => {
                println!("| {identifier} | acquiring token.");
                let Ok(t) = telemetry::acquire_token(&client, &tlm_uri, &identifier)
                    .await else {
                    println!("could not acquire token for {identifier}.");
                    if retry < RETRIES_MAX {
                        retry += 1;
                    } else {
                        println!("retries exhausted, exiting.");
                        return Err(());
                    }
                    continue;
                };

                token = Some(t.clone());
                retry = 0;
                t
            }
        };

        let header: MavHeader = frame.header();
        match frame.msg {
            mavlink::common::MavMessage::OPEN_DRONE_ID_LOCATION(data) => {
                let Ok(decoded_packet) = telemetry::process_open_drone_id_location_data(header.sequence, data) else {
                    println!("could not process open drone id location data");
                    continue;
                };

                if !provisioned {
                    let (hangar_id, hangar_bay_id) = provision::provision_hangar(&client, &itest_uri, decoded_packet.latitude, decoded_packet.longitude, px4).await?;
                    let aircraft_id = provision::provision_aircraft(&client, &itest_uri, &identifier, &hangar_id, &hangar_bay_id, px4).await?;
                    let scanner_id: String = "".to_string(); // TODO register scanner

                    provisioned = true;

                    let Ok(mut state) = state_mtx.lock() else {
                        println!("could not lock state mutex");
                        continue;
                    };

                    (*state).aircraft_id = aircraft_id;
                    (*state).hangar_id = hangar_id;
                    (*state).hangar_bay_id = hangar_bay_id;
                    (*state).scanner_id = scanner_id;
                }

                let _ = telemetry::location_update(&client, &tlm_uri, &tk, decoded_packet)
                    .await
                    .map(|_| {
                        println!("Issued location update for {identifier}");
                    })
                    .map_err(|e| {
                        println!("could not issue location update for {identifier}: {e}");
                        token = None; // invalidate token
                    });

                let _ = telemetry::id_update(&client, &tlm_uri, &identifier, &tk)
                    .await
                    .map(|_| {
                        println!("Issued id update for {identifier}");
                    })
                    .map_err(|e| {
                        println!("could not issue id update for {identifier}: {e}");
                        token = None; // invalidate token
                    });
            }
            mavlink::common::MavMessage::MISSION_CURRENT(data) => {
                let Ok(mut state) = state_mtx.lock() else {
                    println!("could not lock mission state mutex");
                    continue;
                };

                (*state).mission_state = data.mission_state;
            }
            _ => {
                // println!("Received unknown message: {:?}", frame.msg);
            }
        }

    }
}