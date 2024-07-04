use hyper::{
    body::{Body, Bytes},
    client::connect::HttpConnector,
    client::Client,
    Method, Request, StatusCode,
};
use super::FlightPlan;
use svc_atc_client_rest::types::{*, PointZ, FlightPlan as AtcFlightPlan};
use std::collections::{BinaryHeap, VecDeque};

use crate::State;
use geo::prelude::*;
use geo::point;

pub enum OrdersError {
    // Unauthorized,
    Other,
}

impl std::fmt::Display for OrdersError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // OrdersError::Unauthorized => write!(f, "Unauthorized"),
            OrdersError::Other => write!(f, "Other"),
        }
    }
}

pub(crate) async fn acknowledge_order(
    client: &Client<HttpConnector>,
    base_uri: &str,
    flight_id: &str,
    identifier: &str
) -> Result<(), OrdersError> {
    let url = format!("{base_uri}/acknowledge");

    println!("| {identifier} | confirming flight_id from {url}.");

    // acquire plans
    let data = AckRequest {
        fp_id: flight_id.to_string(),
        status: AckStatus::Confirm
    };

    let data_str = serde_json::to_string(&data).unwrap();

    let req = Request::builder()
        .method(Method::POST)
        .uri(url)
        .header("content-type", "application/json")
        .body(Body::from(data_str))
        .map_err(|e| {
            println!("({identifier}) could not build request: {}", e);
            OrdersError::Other
        })?;

    let res = client.request(req).await.map_err(|e| {
        println!("| {identifier} | request to confirm flight plan failed: {}", e);
        OrdersError::Other
    })?;

    if res.status() != StatusCode::OK {
        println!("| {identifier} | could not confirm flight plan: {}", res.status());
        return Err(OrdersError::Other);
    };

    Ok(())
}

pub(crate) async fn get_orders(
    client: &Client<HttpConnector>,
    base_uri: &str,
    uuid: String,
    identifier: &str
) -> Result<VecDeque<FlightPlan>, OrdersError> {
    let url = format!("{base_uri}/plans");

    // println!("| {identifier} | acquiring plans from {url}.");

    // acquire plans
    let req = Request::builder()
        .method(Method::GET)
        .uri(url)
        .header("content-type", "application/json")
        .body(Bytes::from(uuid.clone()).into())
        .map_err(|e| {
            println!("({identifier}) could not build request: {}", e);
            OrdersError::Other
        })?;

    let res = client.request(req).await.map_err(|e| {
        println!("({identifier}) request to acquire plans failed: {}", e);
        OrdersError::Other
    })?;

    if res.status() != StatusCode::OK {
        println!("({identifier}) could not acquire plans: {}", res.status());
        return Err(OrdersError::Other);
    };

    // println!("| {identifier} | acquired plans: {:#?}", res);

    let body = hyper::body::to_bytes(res.into_body()).await.map_err(|e| {
        println!("({identifier}) could not process token stream: {}", e);
        OrdersError::Other
    })?;

    let plans: VecDeque<FlightPlan> = serde_json::from_slice::<VecDeque<AtcFlightPlan>>(&body).map_err(|e| {
        println!("({identifier}) could not parse plans: {}", e);
        OrdersError::Other
    })?
    .into_iter()
    .map(|p| FlightPlan::from(p))
    .collect();

    if !plans.is_empty() {
        println!("| {identifier} | acquired {} plans.", plans.len());
        println!("| {identifier} | plans: {:#?}", plans);
    }
    
    Ok(plans)
}

pub async fn flight_plan_update(
    client: &Client<HttpConnector>,
    cargo_uri: &str,
    tick: &u64,
    state: &mut State,
    plans: &mut BinaryHeap<FlightPlan>,
) -> Result<(), ()> {
    if state.current_plan.is_some() {
        return Ok(());
    }

    let fp: &FlightPlan = plans
        .peek()
        .ok_or_else(|| {
            // println!("| {} | no flight plans available.", state.id);
        })?;

    
    if fp.origin_timeslot_end.timestamp_millis() as u64 > *tick {
        return Err(())
    }

    let plan = plans.pop()
        .ok_or_else(|| {
            // println!("| {} | no flight plans available.", state.id);
        })?;
    
    init_plan(&client, state, &cargo_uri, tick, plan).await;

    Ok(())
}

pub async fn init_plan(
    client: &Client<HttpConnector>,
    state: &mut State,
    cargo_uri: &str,
    current_tick: &u64,
    mut plan: FlightPlan
) {
    println!("| {} | {current_tick} | new flight plan: {}", state.id, plan.session_id);
    for parcel in plan.acquire.iter() {
        let _ = crate::parcel::parcel_scan(
            &client,
            &state.id,
            &state.scanner_id,
            &parcel.id,
            state.position.latitude,
            state.position.longitude,
            &cargo_uri,
        ).await;
    }

    let total_distance: f64 = plan
        .path
        .make_contiguous()
        .windows(2)
        .map(|ps| {
            let p1 = point!(x: ps[0].longitude, y: ps[0].latitude);
            let p2 = point!(x: ps[1].longitude, y: ps[1].latitude);
            p1.haversine_distance(&p2)
        })
        .sum();
    
    if plan.path.is_empty() {
        return;
    }

    let Some(next_point) = plan.path.pop_front() else {
        println!("| {} | no next point in plan.", state.id);
        return;
    };

    // Hacky, jump the aircraft to first point of the plan
    state.position = PointZ {
        latitude: next_point.latitude,
        longitude: next_point.longitude,
        altitude_meters: next_point.altitude_meters,
    };

    let Some(next_point) = plan.path.get(0) else {
        println!("| {} | no next point in plan.", state.id);
        return;
    };

    let total_duration_ms = (plan.target_timeslot_start.timestamp_millis() as u64) - current_tick;
    state.ground_velocity_m_s = total_distance / (total_duration_ms as f64 / 1000.0);
    
    let p1 = point!(x: state.position.longitude, y: state.position.latitude);
    let p2 = point!(x: next_point.longitude, y: next_point.latitude);
    let distance = p1.haversine_distance(&p2);
    let time_to_next_point_s = distance / state.ground_velocity_m_s;
    state.vertical_velocity_m_s = (next_point.altitude_meters - state.position.altitude_meters) / time_to_next_point_s;
    state.track_angle_deg = p1.haversine_bearing(p2);
    if state.track_angle_deg < 0.0 {
        state.track_angle_deg += 360.0;
    }

    state.current_plan = Some(plan);
}

pub async fn end_plan(
    client: &Client<HttpConnector>,
    state: &mut State,
    cargo_uri: &str
) {
    let Some(ref plan) = state.current_plan else {
        println!("| {} | tried to end a non-existent plan.", state.id);
        return;
    };

    for parcel in plan.deliver.iter() {
        let _ = crate::parcel::parcel_scan(
            &client,
            &state.id,
            &state.scanner_id,
            &parcel.id,
            state.position.latitude,
            state.position.longitude,
            &cargo_uri,
        ).await;
    }

    state.current_plan = None;
    state.ground_velocity_m_s = 0.0;
    state.vertical_velocity_m_s = 0.0;
}
