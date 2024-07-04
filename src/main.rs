use clap::Parser;
use svc_atc_client_rest::types::{Pose, Cargo, Phase, FlightPlan as AtcFlightPlan};
use lib_common::time::{ Utc, DateTime};
use std::sync::{Arc, Mutex};
use mavlink::common::MissionState;

// mod orders;
// mod parcel;
mod proxy;
mod telemetry;
mod provision;
mod config;

// use orders::*;
const AIRCRAFT_PREFIX: &str = "AETH-PX4-SIM";

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// PX4 instance number
    #[arg(long, short, default_value_t = 0)]
    px4: u32,

}

// #[derive(Debug, Clone)]
struct FlightPlan {
    flight_uuid: String,
    session_id: String,
    origin_timeslot_start: DateTime<Utc>,
    origin_timeslot_end: DateTime<Utc>,
    target_timeslot_start: DateTime<Utc>,
    target_timeslot_end: DateTime<Utc>,
    // acquire: Vec<Cargo>,
    // deliver: Vec<Cargo>,
    phases: Vec<Phase>,
}

impl From<AtcFlightPlan> for FlightPlan {
    fn from(value: AtcFlightPlan) -> Self {
        FlightPlan {
            flight_uuid: value.flight_uuid,
            session_id: value.session_id,
            origin_timeslot_start: value.origin_timeslot_start,
            origin_timeslot_end: value.origin_timeslot_end,
            target_timeslot_start: value.target_timeslot_start,
            target_timeslot_end: value.target_timeslot_end,
            // acquire: value.acquire,
            // deliver: value.deliver,
            phases: value.phases
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

#[derive(Debug)]
struct State {
    session_id: String,
    aircraft_id: String,
    hangar_id: String,
    hangar_bay_id: String,
    scanner_id: String,
    mission_state: MissionState,
}


#[tokio::main]
async fn main() -> Result<(), ()> {
    let config = config::Config::try_from_env()
        .map_err(|e| {
            format!("could not load config: {}", e);
        })?;

    let args = Args::parse();
    let base_url = config.realm_host;
    let tlm_uri = format!("http://{base_url}:{}/telemetry", config.telemetry_host_port_rest);
    let itest_uri = format!("http://{base_url}:{}/demo", config.itest_host_port_rest);

    // let atc_uri = format!("{base_url}:{}/atc", config.atc_host_port_rest);
    // let cargo_uri = format!("{base_url}:{}/cargo", config.cargo_host_port_rest);
    // println!("({}) aircraft startup.", identifier);
    let identifier = format!("{}-{}", AIRCRAFT_PREFIX, args.px4);

    //
    // Spawn mavlink passthrough for this px4 instance
    // Each instance will broadcast to a unique port, starting from the base port 14550
    let state_mtx = Arc::new(Mutex::new(State {
        session_id: identifier.clone(),
        aircraft_id: String::new(),
        hangar_id: String::new(),
        hangar_bay_id: String::new(),
        scanner_id: String::new(),
        mission_state: MissionState::MISSION_STATE_UNKNOWN,
    }));

    //
    // Data needed for the proxy
    let (tx, mut cmd_rx) = std::sync::mpsc::channel();
    let proxy_udp_address = format!("127.0.0.1:{}", config.udp_port as u32 + args.px4);
    let proxy_tlm_uri = tlm_uri.clone();
    let proxy_itest_uri = itest_uri.clone();
    let proxy_state_mtx = Arc::clone(&state_mtx);
    let proxy_handle = tokio::spawn(async move {
        let _ = proxy::mavlink_proxy(
            proxy_udp_address,
            proxy_tlm_uri,
            proxy_itest_uri,
            proxy_state_mtx,
            args.px4,
            cmd_rx
        ).await;
    });

    // Orders loop
    const SLEEP_TIME_MS: u64 = 1000;
    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(SLEEP_TIME_MS));
    let mut last_tick = Utc::now().timestamp_millis() as u64;

    loop {
        interval.tick().await;
        let current_tick = Utc::now().timestamp_millis() as u64;
        last_tick = current_tick;

        // if proxy_handle.is_finished() {
        //     println!("| {} | mavlink proxy exited, shutting down.", identifier);
        //     break;
        // }

        println!("| {} | tick: {}", identifier, current_tick);

        {
            let Ok(mut state) = state_mtx.lock() else {
                println!("could not lock mission state mutex");
                continue;
            };

            match state.mission_state {
                MissionState::MISSION_STATE_ACTIVE => {

                },
                MissionState::MISSION_STATE_NO_MISSION => {
                    if let Some(ref next) = plans.front() {
                        // check if time to start
                        if next.origin_timeslot_start.timestamp_millis() as u64 < current_tick {
                            let _ = orders::init_plan(&client, &mut state, &cargo_uri, &current_tick, plan).await;
                        }
                    };
                },
                MissionState::MISSION_STATE_COMPLETE => {
                    let _ = plans.pop_front();
                    (*state).mission_state = MissionState::MISSION_STATE_NO_MISSION;
                },
                MissionState::MISSION_STATE_PAUSED | MissionState::MISSION_NOT_STARTED => {
                    // TODO home? automatic?
                },
            }
        }

        // if let Some(ref plan) = state.current_plan {
            // if plan.path.is_empty() {
            //     old_sessions.push_back(plan.session_id.clone());
            //     orders::end_plan(&client, &mut state, &cargo_uri).await;

            //     while old_sessions.len() > 10 {
            //         old_sessions.pop_front();
            //     }
            // }
        // }

        // Every 15000ms
        // Get Orders
        // if (current_tick - state.last_order_check) > config.interval_order_check_ms {
        //     // get orders
        //     state.last_order_check = current_tick;

        //     let result = get_orders(&client, &atc_uri, uuid.clone(), &identifier).await;
        //     let Ok(orders) = result else {
        //         println!("| ({}) | could not get orders.", state.id);
        //         continue;
        //     };
            
        //     for order in orders {
        //         if let Some(ref plan) = state.current_plan {
        //             if plan.session_id == order.session_id {
        //                 continue;
        //             }
        //         }

        //         if old_sessions.contains(&order.session_id) {
        //             continue;
        //         }

        //         if plans.iter().find(|p| p.session_id == order.session_id).is_none() {
        //             plans.push(order.clone())
        //         }

        //         let _ = orders::acknowledge_order(&client, &atc_uri, &order.flight_uuid, &identifier).await;
        //     }

        //     if let Some(ref plan) = plans.peek() {
        //         let next_flight_s = (plan.origin_timeslot_end.timestamp_millis() as u64 - current_tick) / 1000;
        //         println!("| {} | next flight time: {} (T-{} s)", state.id, plan.origin_timeslot_end, next_flight_s);
        //     }
        // }
    }

    proxy_handle.abort();

    Ok(())
}
