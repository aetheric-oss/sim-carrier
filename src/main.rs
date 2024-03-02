use clap::Parser;
use hyper::{
    body::{Body, Bytes},
    client::connect::HttpConnector,
    client::Client,
    Method, Request, StatusCode,
};
use packed_struct::PackedStruct;
use svc_telemetry_client_rest::netrid_types::*;
use geo::prelude::*;
use geo::point;

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Name of the person to greet
    #[arg(long)]
    host: String,

    /// Number of times to greet
    #[arg(long)]
    port: u16,
}

#[derive(Debug)]
pub struct PointZ {
    longitude: f64,
    latitude: f64,
    altitude: f64,
}

pub enum Activity {
    Idle,
    Cruise,
}

pub enum Connectivity {
    Connected,
    Disconnected,
}

pub enum NetworkError {
    Unauthorized,
    Other,
}

const SLEEP_TIME_MS: u64 = 100;

impl std::fmt::Display for NetworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetworkError::Unauthorized => write!(f, "Unauthorized"),
            NetworkError::Other => write!(f, "Other"),
        }
    }
}

struct FlightPlan {
    id: String,
    path: Vec<(PointZ, u64)>,
}

struct State {
    current_plan: Option<FlightPlan>,
    id: String,
    token: Option<String>,
    activity: Activity,
    position: PointZ,
    ground_velocity_m_s: f64,
    vertical_velocity_m_s: f64,
    track_angle_deg: f64,
    last_update_ms: u64,
    last_id_update_ms: u64,
    last_order_check: u64,
    // operational: bool, // for simulating sudden out of service
}

async fn acquire_token(
    client: &Client<HttpConnector>,
    base_url: &str,
    identifier: String,
) -> Result<String, NetworkError> {
    let url = format!("{base_url}/login");

    println!("| {identifier} | acquiring token from {url}.");
    // acquire token
    let req = Request::builder()
        .method(Method::GET)
        .uri(url)
        .header("content-type", "text/plain")
        .body(Bytes::from(identifier.clone()).into())
        .unwrap();

    let res = client.request(req).await.map_err(|e| {
        println!("({identifier}) could not acquire token: {}", e);
        NetworkError::Other
    })?;

    if res.status() != StatusCode::OK {
        println!("({identifier}) could not acquire token: {}", res.status());
        return Err(NetworkError::Unauthorized);
    };

    let body = hyper::body::to_bytes(res.into_body()).await.map_err(|e| {
        println!("({identifier}) could not process token stream: {}", e);
        NetworkError::Other
    })?;

    let token = String::from_utf8(body.to_vec())
        .map_err(|e| {
            println!("({identifier}) could not convert token to string: {}", e);
            NetworkError::Other
        })?
        .trim_matches('"')
        .replace("\"", "");

    println!("| {identifier} | acquired token.");
    Ok(token)
}

async fn id_update(
    client: &Client<HttpConnector>,
    url: &str,
    identifier: &str,
    token: &str,
) -> Result<(), NetworkError> {
    // issue id update
    let Ok(uas_id) = <[u8; 20]>::try_from(format!("{:>20}", identifier).as_ref()) else {
        panic!("({identifier} could not convert identifier to [u8; 20]");
    };

    // build NETRID Packet
    let Ok(message) = BasicMessage {
        id_type: IdType::CaaAssigned,
        ua_type: UaType::Rotorcraft,
        uas_id,
        ..Default::default()
    }
    .pack() else {
        panic!("({identifier} could not pack BasicMessage");
    };

    let Ok(payload) = Frame {
        header: Header {
            message_type: MessageType::Basic,
            ..Default::default()
        },
        message,
    }
    .pack() else {
        panic!("({identifier} could not pack Frame");
    };

    let req = Request::builder()
        .method(Method::POST)
        .uri(format!("{url}/netrid"))
        .header("content-type", "application/octet-stream")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::from(payload.to_vec()))
        .unwrap();

    let result = client.request(req).await.map_err(|e| {
        println!("({identifier}) could not issue id update: {}", e);
        NetworkError::Other
    })?;

    if result.status() != StatusCode::OK {
        println!(
            "({identifier}) could not issue id update: {}",
            result.status()
        );
        return Err(NetworkError::Unauthorized);
    }

    // println!("({identifier}) response {:#?}.", result);
    // println!("({identifier}) issued id update.");
    Ok(())
}

/// Issue position update to network
async fn position_update(client: &Client<HttpConnector>, url: &str, token: &str, state: &State) -> Result<(), NetworkError> {
    let altitude = LocationMessage::encode_altitude(state.position.altitude as f32);

    let adjusted_track = if state.track_angle_deg < 0.0 {
        state.track_angle_deg + 360.0
    } else {
        state.track_angle_deg
    };

    let Ok((ew_direction, track_direction)) = LocationMessage::encode_direction(adjusted_track as u16) else {
        panic!("({}) could not encode direction", state.id);
    };

    let Ok((speed_multiplier, speed)) = LocationMessage::encode_speed(state.ground_velocity_m_s as f32) else {
        panic!("({}) could not encode speed", state.id);
    };

    let vertical_speed = LocationMessage::encode_vertical_speed(state.vertical_velocity_m_s as f32);
    let latitude = LocationMessage::encode_latitude(state.position.latitude);
    let longitude = LocationMessage::encode_longitude(state.position.longitude);
    let Ok(timestamp) = LocationMessage::encode_timestamp(chrono::Utc::now()) else {
        panic!("({}) could not encode timestamp", state.id);
    };

    let Ok(message) = LocationMessage {
        speed,
        speed_multiplier,
        speed_accuracy: SpeedAccuracyMetersPerSecond::Lt1,
        ew_direction,
        track_direction,
        vertical_speed,
        latitude,
        longitude,
        pressure_altitude: altitude.clone(),
        geodetic_altitude: altitude.clone(),
        height: altitude,
        height_type: HeightType::AboveGroundLevel,
        vertical_accuracy: VerticalAccuracyMeters::Lt1,
        barometric_altitude_accuracy: VerticalAccuracyMeters::Lt1,
        horizontal_accuracy: HorizontalAccuracyMeters::Lt1,
        timestamp,
        timestamp_accuracy: 0.into(),
        operational_status: match state.activity {
            Activity::Idle => OperationalStatus::Ground,
            Activity::Cruise => OperationalStatus::Airborne,
        },
        reserved_0: 0.into(),
        reserved_1: 0.into(),
        reserved_2: 0,
    }
    .pack() else {
        panic!("({}) could not pack LocationMessage", state.id);
    };

    let Ok(payload) = Frame {
        header: Header {
            message_type: MessageType::Location,
            ..Default::default()
        },
        message,
    }
    .pack() else {
        panic!("({}) could not pack location frame", state.id);
    };

    let req = Request::builder()
        .method(Method::POST)
        .uri(format!("{url}/netrid"))
        .header("content-type", "application/octet-stream")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::from(payload.to_vec()))
        .unwrap();

    let result = client.request(req).await.map_err(|e| {
        println!("({}) could not issue position update: {}", state.id, e);
        NetworkError::Other
    })?;

    if result.status() != StatusCode::OK {
        println!(
            "({}) could not issue position update: {}",
            state.id,
            result.status()
        );
        return Err(NetworkError::Unauthorized);
    }

    // println!("({}) response {:#?}.", state.id, result);
    // println!("({}) issued position update.", state.id);
    Ok(())
}

fn adjust_velocity(current_ms: &u64, state: &mut State) {
    println!("| {} | {current_ms} | adjusting velocity.", state.id);
    println!("| {} | {current_ms} | current location: {:?}", state.id, state.position);
    let Some(ref plan) = state.current_plan else {
        state.current_plan = None;
        state.activity = Activity::Idle;
        return;
    };

    let Some((ref next_point, tick)) = plan.path.get(0) else {
        println!("| {} | {current_ms} | no more points in plan.", state.id);
        state.current_plan = None;
        state.activity = Activity::Idle;
        return;
    };

    let time_to_next_point_s = (tick - *current_ms) as f64 / 1000.0;
    println!("| {} | {} | next point: {:?} in {} s", state.id, current_ms, next_point, time_to_next_point_s);

    let p1 = point!(x: state.position.longitude, y: state.position.latitude);
    let p2 = point!(x: next_point.longitude, y: next_point.latitude);
    state.ground_velocity_m_s = p1.haversine_distance(&p2) / time_to_next_point_s;
    state.vertical_velocity_m_s = (next_point.altitude - state.position.altitude) / time_to_next_point_s;
    state.track_angle_deg = p1.haversine_bearing(p2);

    println!("| {} | {} | adjusted velocity; hor m/s: {}, vert m/s: {}, bearing (deg): {}", state.id, current_ms, state.ground_velocity_m_s, state.vertical_velocity_m_s, state.track_angle_deg);
}

fn update_location(current_tick: &u64, state: &mut State) {
    let Some(ref mut plan) = state.current_plan else {
        state.activity = Activity::Idle;
        return;
    };

    // update state
    let elapsed_s = (SLEEP_TIME_MS as f64) / 1000.0;
    let vertical_travel_distance_m = state.vertical_velocity_m_s * elapsed_s;
    let horizontal_travel_distance_m = state.ground_velocity_m_s * elapsed_s;
    state.position.altitude += vertical_travel_distance_m;

    let p1 = point!(x: state.position.longitude, y: state.position.latitude);
    let p2 = p1.haversine_destination(state.track_angle_deg, horizontal_travel_distance_m);

    state.position.longitude = p2.x();
    state.position.latitude = p2.y();

    let Some((ref next_point, _)) = plan.path.get(0) else {
        state.current_plan = None;
        state.activity = Activity::Idle;
        return;
    };

    let p3 = point!(x: next_point.longitude, y: next_point.latitude);
    if p2.haversine_distance(&p3) >= 5.0 {
        return;
    }

    // Arrived at point
    println!("| {} | {} | arrived at intermediate point.", state.id, current_tick);
    plan.path.remove(0);

    adjust_velocity(current_tick, state);
}

#[tokio::main]
async fn main() {
    let identifier = format!("AETH-{:>05}", rand::random::<u8>());

    println!("({}) aircraft startup.", identifier);

    let args = Args::parse();
    let url = format!("http://{}:{}", args.host, args.port);
    let client: Client<HttpConnector> = Client::builder()
        .pool_idle_timeout(std::time::Duration::from_secs(10))
        .build_http();
    let uri = format!("{url}/telemetry");

    let mut state = State {
        id: identifier,
        current_plan: None,
        activity: Activity::Idle,
        token: None,
        position: PointZ {
            longitude: 5.133053153910531,
            latitude: 52.64237411858314,
            altitude: 0.0,
        },
        ground_velocity_m_s: 0.0,
        vertical_velocity_m_s: 0.0,
        track_angle_deg: 0.0,
        last_update_ms: 0,
        last_id_update_ms: 0,
        last_order_check: 0,
        // operational: true,
    };

    let mut plans: Vec<FlightPlan> = vec![
        FlightPlan {
            id: "1".to_string(),
            path: vec![
                (
                    PointZ {
                        longitude: 5.133053153910531,
                        latitude: 52.64237411858314,
                        altitude: 0.0,
                    },
                    chrono::Utc::now().timestamp_millis() as u64,
                ),
                (
                    PointZ {
                        longitude: 5.070817616006518,
                        latitude: 52.615021990689414,
                        altitude: 100.0,
                    },
                    (chrono::Utc::now() + chrono::Duration::seconds(60)).timestamp_millis() as u64,
                ),
                (
                    PointZ {
                        longitude: 5.014984013754936,
                        latitude: 52.61957604802006,
                        altitude: 0.0,
                    },
                    (chrono::Utc::now() + chrono::Duration::seconds(120)).timestamp_millis() as u64,
                ),
            ]
        },
    ];

    let mut retry = 0;
    let max_retries = 5;

    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(SLEEP_TIME_MS));

    loop {
        interval.tick().await;
        let current_tick = chrono::Utc::now().timestamp_millis() as u64;
        update_location(&current_tick, &mut state);

        // Check for new orders
        if state.current_plan.is_none() {
            if let Some(fp) = plans.get(0) {
                if let Some((_, tick)) = fp.path.get(0) {
                    if *tick < current_tick {
                        let plan = plans.remove(0);
                        println!("| {} | {current_tick} | new flight plan: {}", state.id, plan.id);
                        state.current_plan = Some(plan);
                        adjust_velocity(&current_tick, &mut state);
                    }
                }
            }
        }

        // Acquire network token if not present
        let Some(ref token) = state.token else {
            if let Ok(token) = acquire_token(&client, &uri, state.id.clone()).await {
                state.token = Some(token);
                retry = 0;

                continue;
            } else {
                retry += 1;
                if retry > max_retries {
                    panic!(
                        "({}) could not acquire token, expeded all retries.",
                        state.id
                    );
                }

                continue;
            }
        };

        // Every 2000ms (0.5 Hz)
        if current_tick - state.last_id_update_ms > 2000 {
            // issue id update
            let result = id_update(&client, &uri, &state.id, &token).await;

            match result {
                Ok(_) => {
                    state.last_id_update_ms = current_tick;
                }
                Err(e) => {
                    println!("({}) could not issue id update: {}", state.id, e);
                    state.token = None;
                    continue;
                }
            }
        }

        // Every 500ms (2 Hz)
        if current_tick - state.last_update_ms > 500 {
            // issue position and velocity update
            let result = position_update(&client, &uri, &token, &state).await;

            match result {
                Ok(_) => {
                    state.last_update_ms = current_tick;
                }
                Err(e) => {
                    println!("({}) could not issue position update: {}", state.id, e);
                    state.token = None;
                    continue;
                }
            }
        }

        // Every 10s (0.1 Hz)
        // if current_tick - state.last_order_check > 10000 {
        //     // check for orders
        //     state.last_order_check = current_tick;
        // }

        match state.activity {
            Activity::Idle => {
                if state.current_plan.is_some() {
                    state.activity = Activity::Cruise;
                    continue;
                }
            }
            Activity::Cruise => {
                // if current position is within 1 meters of destination
                // switch to Idle
                // state.current_plan = None;
                // state.activity = Activity::Idle;
            }
        }
    }
}
