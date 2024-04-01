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
use chrono::Utc;
use crate::State;

pub enum NetworkError {
    Unauthorized,
    Other,
}

impl std::fmt::Display for NetworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetworkError::Unauthorized => write!(f, "Unauthorized"),
            NetworkError::Other => write!(f, "Other"),
        }
    }
}

pub(crate) async fn acquire_token(
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

pub(crate) async fn id_update(
    client: &Client<HttpConnector>,
    url: &str,
    id_type: IdType,
    uas_id: &str,
    token: &str,
) -> Result<(), NetworkError> {
    // issue id update
    let Ok(uas_id_formatted) = <[u8; 20]>::try_from(format!("{:>20}", uas_id).as_ref()) else {
        panic!("({uas_id} could not convert identifier to [u8; 20]");
    };

    // build NETRID Packet
    let Ok(message) = BasicMessage {
        ua_type: UaType::Rotorcraft,
        id_type,
        uas_id: uas_id_formatted,
        ..Default::default()
    }
    .pack() else {
        panic!("({uas_id} could not pack BasicMessage");
    };

    let Ok(payload) = Frame {
        header: Header {
            message_type: MessageType::Basic,
            ..Default::default()
        },
        message,
    }
    .pack() else {
        panic!("({uas_id} could not pack Frame");
    };

    let req = Request::builder()
        .method(Method::POST)
        .uri(format!("{url}/netrid"))
        .header("content-type", "application/octet-stream")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::from(payload.to_vec()))
        .unwrap();

    let result = client.request(req).await.map_err(|e| {
        println!("({uas_id}) could not issue id update: {}", e);
        NetworkError::Other
    })?;

    if result.status() != StatusCode::OK {
        println!(
            "({uas_id}) could not issue id update: {}",
            result.status()
        );
        return Err(NetworkError::Unauthorized);
    }

    // println!("({uas_id}) response {:#?}.", result);
    // println!("({uas_id}) issued id update.");
    Ok(())
}

/// Issue position update to network
pub(crate) async fn position_update(client: &Client<HttpConnector>, url: &str, token: &str, state: &State) -> Result<(), NetworkError> {
    let altitude = LocationMessage::encode_altitude(state.position.altitude_meters as f32);

    let Ok((ew_direction, track_direction)) = LocationMessage::encode_direction(state.track_angle_deg as u16) else {
        panic!("({}) could not encode direction", state.id);
    };

    // println!("| {} | ew_direction: {:?}, track_direction: {}", state.id, ew_direction, track_direction);

    let Ok((speed_multiplier, speed)) = LocationMessage::encode_speed(state.ground_velocity_m_s as f32) else {
        panic!("({}) could not encode speed", state.id);
    };

    let vertical_speed = LocationMessage::encode_vertical_speed(state.vertical_velocity_m_s as f32);
    let latitude = LocationMessage::encode_latitude(state.position.latitude);
    let longitude = LocationMessage::encode_longitude(state.position.longitude);
    let timestamp = LocationMessage::encode_timestamp(Utc::now())
        .map_err(|e| {
            panic!("({}) could not encode timestamp: {:?}", state.id, e);
        })?;

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
        operational_status: match state.ground_velocity_m_s > 0.0 {
            false => OperationalStatus::Ground,
            true => OperationalStatus::Airborne,
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

pub(crate) fn adjust_vertical_velocity(current_ms: &u64, state: &mut State) {
    // println!("| {} | {current_ms} | adjusting velocity.", state.id);
    // println!("| {} | {current_ms} | current location: {:?}", state.id, state.position);
    let Some(ref plan) = state.current_plan else {
        return;
    };

    let Some(ref next_point) = plan.path.get(0) else {
        println!("| {} | {current_ms} | no more points in plan.", state.id);
        return;
    };

    let p1 = point!(x: state.position.longitude, y: state.position.latitude);
    let p2 = point!(x: next_point.longitude, y: next_point.latitude);
    let distance = p1.haversine_distance(&p2); 
    let time_to_next_point_s = distance / state.ground_velocity_m_s;

    println!("| {} | {} | next point: {:?} in {} s", state.id, current_ms, next_point, time_to_next_point_s);
    
    state.vertical_velocity_m_s = (next_point.altitude_meters - state.position.altitude_meters) / time_to_next_point_s;
    state.track_angle_deg = p1.haversine_bearing(p2);
    if state.track_angle_deg < 0.0 {
        state.track_angle_deg += 360.0;
    }

    println!("| {} | {} | adjusted velocity; hor m/s: {}, vert m/s: {}, bearing (deg): {}", state.id, current_ms, state.ground_velocity_m_s, state.vertical_velocity_m_s, state.track_angle_deg);
}

pub(crate) fn update_location(current_ms: &u64, last_ms: &u64, state: &mut State) {
    let Some(ref mut plan) = state.current_plan else {
        return;
    };

    // update state
    let elapsed_s = ((current_ms - last_ms) as f64) / 1000.0;
    let vertical_travel_distance_m = state.vertical_velocity_m_s * elapsed_s;
    let horizontal_travel_distance_m = state.ground_velocity_m_s * elapsed_s;
    state.position.altitude_meters += vertical_travel_distance_m;

    let p1 = point!(x: state.position.longitude, y: state.position.latitude);
    let p2 = p1.haversine_destination(state.track_angle_deg, horizontal_travel_distance_m);

    state.position.longitude = p2.x();
    state.position.latitude = p2.y();

    let Some(ref next_point) = plan.path.get(0) else {
        println!("| {} | {current_ms} | no more points in plan.", state.id);
        return;
    };

    let p3 = point!(x: next_point.longitude, y: next_point.latitude);
    // println!("| {} | {} | longitude: {}, latitude: {}, altitude: {}", state.id, current_ms, state.position.longitude, state.position.latitude, state.position.altitude_meters);
    if p2.haversine_distance(&p3) >= 10.0 {
        return;
    }

    // Arrived at point
    println!("| {} | {} | arrived at intermediate point.", state.id, current_ms);
    plan.path.remove(0);

    adjust_vertical_velocity(current_ms, state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use svc_atc_client_rest::types::*;

    const METERS_PER_DEGREE_LATITUDE: f64 = 111_320.;

    #[tokio::test]
    async fn test_update_location() {
        let mut state = State {
            current_plan: None,
            id: uuid::Uuid::new_v4().to_string(),
            scanner_id: uuid::Uuid::new_v4().to_string(),
            token: None,
            position: PointZ {
                longitude: 5.167,
                latitude: 52.64,
                altitude_meters: 10.0,
            },
            ground_velocity_m_s: 5.0,
            vertical_velocity_m_s: 5.0,
            track_angle_deg: 0.0,
            last_update_ms: 0,
            last_id_update_ms: 0,
            last_order_check: 0,
        };

        state.current_plan = Some(FlightPlan {
            path: vec![
                PointZ {
                    longitude: state.position.longitude,
                    latitude: state.position.latitude + 0.01,
                    altitude_meters: 20.0,
                },
            ],
            origin_timeslot_end: Utc::now(),
            origin_timeslot_start: Utc::now(),
            target_timeslot_end: Utc::now(),
            target_timeslot_start: Utc::now(),
            aircraft_id: state.id.clone(),
            flight_uuid: uuid::Uuid::new_v4().to_string(),
            session_id: "AETH1234".to_string(),
            origin_vertiport_id: uuid::Uuid::new_v4().to_string(),
            target_vertiport_id: uuid::Uuid::new_v4().to_string(),
            origin_vertipad_id: uuid::Uuid::new_v4().to_string(),
            target_vertipad_id: uuid::Uuid::new_v4().to_string(),
            acquire: vec![],
            deliver: vec![]
        });

        let current_ms: u64 = Utc::now().timestamp_millis() as u64;
        let original = state.clone();
        update_location(&current_ms, &current_ms, &mut state);
        assert!(state.position.latitude - original.position.latitude < 0.00000001);
        assert!(state.position.longitude - original.position.longitude < 0.00000001);

        let duration_s: f64 = 10.;
        let estimated_latitude_delta = (duration_s * state.ground_velocity_m_s) / METERS_PER_DEGREE_LATITUDE;
        let estimated_latitude_delta_ceil = ((duration_s + 0.1) * state.ground_velocity_m_s) / METERS_PER_DEGREE_LATITUDE;

        let new_ms = current_ms + (duration_s * 1000.0) as u64;
        update_location(&new_ms, &current_ms, &mut state);

        // no longitude change
        assert!(state.position.longitude - original.position.longitude < 0.00000001);
        assert!(state.position.latitude - original.position.latitude > estimated_latitude_delta);
        assert!(state.position.latitude - original.position.latitude < estimated_latitude_delta_ceil);
        assert_eq!(state.track_angle_deg, 0.0);
    }
}