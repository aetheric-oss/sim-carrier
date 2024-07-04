use clap::Parser;
use hyper::{
    client::connect::HttpConnector,
    client::Client,
};
use svc_telemetry_client_rest::netrid_types::*;
use svc_atc_client_rest::types::{PointZ, Cargo, FlightPlan as AtcFlightPlan};
use lib_common::time::{DateTime, Utc};
use std::collections::{BinaryHeap, VecDeque};

mod orders;
mod parcel;
mod telemetry;
mod config;

use telemetry::*;
use orders::*;

const MAX_RETRIES: u8 = 5;
const RETRY_SLEEP_S: u64 = 5;

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// aircraft name
    #[arg(long)]
    name: String,

    /// aircraft uuid
    #[arg(long)]
    uuid: String,

    /// starting longitude
    #[arg(long)]
    longitude: f64,

    /// starting latitude
    #[arg(long)]
    latitude: f64,

    /// scanner id
    #[arg(long)]
    scanner_id: String
}

const SLEEP_TIME_MS: u64 = 50;

#[derive(Debug, Clone)]
struct FlightPlan {
    flight_uuid: String,
    session_id: String,
    origin_timeslot_start: DateTime<Utc>,
    origin_timeslot_end: DateTime<Utc>,
    target_timeslot_start: DateTime<Utc>,
    // target_timeslot_end: DateTime<Utc>,
    acquire: Vec<Cargo>,
    deliver: Vec<Cargo>,
    path: VecDeque<PointZ>,
}

impl From<AtcFlightPlan> for FlightPlan {
    fn from(value: AtcFlightPlan) -> Self {
        let mut path = VecDeque::new();
        for point in value.path {
            path.push_back(PointZ {
                longitude: point.longitude,
                latitude: point.latitude,
                altitude_meters: point.altitude_meters,
            });
        }

        FlightPlan {
            flight_uuid: value.flight_uuid,
            session_id: value.session_id,
            origin_timeslot_start: value.origin_timeslot_start,
            origin_timeslot_end: value.origin_timeslot_end,
            target_timeslot_start: value.target_timeslot_start,
            // target_timeslot_end: value.target_timeslot_end,
            acquire: value.acquire,
            deliver: value.deliver,
            path
        }
    }
}

#[derive(Debug, Clone)]
struct State {
    current_plan: Option<FlightPlan>,
    id: String,
    scanner_id: String,
    token: Option<String>,
    position: PointZ,
    ground_velocity_m_s: f64,
    vertical_velocity_m_s: f64,
    track_angle_deg: f64,
    last_update_ms: u64,
    last_id_update_ms: u64,
    last_order_check: u64,
}

impl Default for State {
    fn default() -> Self {
        State {
            current_plan: None,
            id: String::new(),
            scanner_id: String::new(),
            token: None,
            position: PointZ {
                longitude: 0.0,
                latitude: 0.0,
                altitude_meters: 0.0,
            },
            ground_velocity_m_s: 0.0,
            vertical_velocity_m_s: 0.0,
            track_angle_deg: 0.0,
            last_update_ms: 0,
            last_id_update_ms: 0,
            last_order_check: 0,
        }
    }
}

impl Ord for FlightPlan {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // min-heap
        other.origin_timeslot_start.cmp(&self.origin_timeslot_start)
    }
}

impl PartialOrd for FlightPlan {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for FlightPlan {
    fn eq(&self, other: &Self) -> bool {
        self.origin_timeslot_start == other.origin_timeslot_start
    }
}

impl Eq for FlightPlan {}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = config::Config::try_from_env()
        .map_err(|e| format!("could not load config: {}", e))?;

    let args = Args::parse();

    let identifier = args.name;
    let uuid = args.uuid;
    let scanner_id = args.scanner_id;
    let base_url = config.host;
    let tlm_uri = format!("{base_url}:{}/telemetry", config.telemetry_host_port_rest);
    let atc_uri = format!("{base_url}:{}/atc", config.atc_host_port_rest);
    let cargo_uri = format!("{base_url}:{}/cargo", config.cargo_host_port_rest);
    
    println!("({}) aircraft startup.", identifier);

    let mut state = State {
        id: identifier.clone(),
        scanner_id,
        position: PointZ {
            longitude: args.longitude,
            latitude: args.latitude,
            altitude_meters: 0.0,
        },
        ..Default::default()
    };

    let mut plans: BinaryHeap<FlightPlan> = BinaryHeap::new();
    let mut old_sessions: VecDeque<String> = VecDeque::new();
    let mut retry: u8 = 0;

    let client: Client<HttpConnector> = Client::builder()
        .pool_idle_timeout(std::time::Duration::from_secs(10))
        .build_http();

    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(SLEEP_TIME_MS));
    let mut last_tick = Utc::now().timestamp_millis() as u64;
    loop {
        interval.tick().await;
        let current_tick = Utc::now().timestamp_millis() as u64;
        update_location(&current_tick, &last_tick, &mut state);
        last_tick = current_tick;

        // Check for new orders
        let _ = flight_plan_update(&client, &cargo_uri, &current_tick, &mut state, &mut plans).await;

        if let Some(ref plan) = state.current_plan {
            if plan.path.is_empty() {
                old_sessions.push_back(plan.session_id.clone());
                orders::end_plan(&client, &mut state, &cargo_uri).await;

                while old_sessions.len() > 10 {
                    old_sessions.pop_front();
                }
            }
        }

        // Acquire network token if not present
        let Some(ref token) = state.token else {
            if let Ok(token) = acquire_token(&client, &tlm_uri, state.id.clone()).await {
                state.token = Some(token);
                retry = 0;
                continue;
            } else {
                retry += 1;
                if retry > MAX_RETRIES {
                    panic!(
                        "({}) could not acquire token, expeded all retries.",
                        state.id
                    );
                }

                tokio::time::sleep(tokio::time::Duration::from_secs(RETRY_SLEEP_S)).await;
                continue;
            }
        };

        // Every 2000ms (0.5 Hz)
        if current_tick - state.last_id_update_ms > 2000 {
            let (id_type, id) = match state.current_plan {
                Some(ref p) => (IdType::SpecificSession, p.session_id.clone()),
                None => (IdType::CaaAssigned, state.id.clone())
            };

            // issue id update
            let result = id_update(
                &client,
                &tlm_uri,
                id_type,
                &id,
                &token
            ).await;

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
            let result = position_update(&client, &tlm_uri, &token, &state).await;

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

        // Every 15000ms
        if current_tick - state.last_order_check > 15000 {
            // issue position and velocity update
            let result = get_orders(&client, &atc_uri, uuid.clone(), &identifier).await;
            state.last_order_check = current_tick;

            let Ok(orders) = result else {
                println!("| ({}) | could not get orders.", state.id);
                continue;
            };
            
            for order in orders {
                if let Some(ref plan) = state.current_plan {
                    if plan.session_id == order.session_id {
                        continue;
                    }
                }

                if old_sessions.contains(&order.session_id) {
                    continue;
                }

                if plans.iter().find(|p| p.session_id == order.session_id).is_none() {
                    plans.push(order.clone())
                }

                let _ = orders::acknowledge_order(&client, &atc_uri, &order.flight_uuid, &identifier).await;
            }

            if let Some(ref plan) = plans.peek() {
                let next_flight_s = (plan.origin_timeslot_end.timestamp_millis() as u64 - current_tick) / 1000;
                println!("| {} | next flight time: {} (T-{} s)", state.id, plan.origin_timeslot_end, next_flight_s);
            }
        }
    }
}
