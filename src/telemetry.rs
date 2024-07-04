use hyper::{
    body::{Body, Bytes},
    client::{ connect::HttpConnector, Client },
    Method, Request, StatusCode,
};
use packed_struct::PackedStruct;
use svc_telemetry_client_rest::netrid_types::{
    *, location::DecodedLocationRid, location::EncodedLocationRid,
    basic::DecodedBasicRid, basic::EncodedBasicRid,
};
// use geo::prelude::*;
// use geo::point;
use lib_common::time::Utc;
// use num_derive::FromPrimitive;
use num_traits::FromPrimitive;

pub enum NetworkError {
    Unauthorized,
    StateLock,
    TokenLock,
    DirectionEncode,
    SpeedEncode,
    TimestampEncode,
    Other,
}

pub type NetridPacket = [u8; 25];

impl std::fmt::Display for NetworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetworkError::StateLock => write!(f, "Could not get state lock"),
            NetworkError::TokenLock => write!(f, "Could not get token lock"),
            NetworkError::DirectionEncode => write!(f, "Could not encode direction"),
            NetworkError::SpeedEncode => write!(f, "Could not encode speed"),
            NetworkError::TimestampEncode => write!(f, "Could not encode timestamp"),
            NetworkError::Unauthorized => write!(f, "Unauthorized"),
            NetworkError::Other => write!(f, "Other"),
        }
    }
}

pub(crate) async fn acquire_token(
    client: &Client<HttpConnector>,
    base_url: &str,
    identifier: &String,
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
    uas_id: &str,
    token: &str,
) -> Result<(), NetworkError> {
    // issue id update
    // build NETRID Packet
    let message = DecodedBasicRid {
        ua_type: UaType::Rotorcraft,
        id_type: IdType::CaaAssigned,
        uas_id: uas_id.to_string(),
    };

    let encoded = EncodedBasicRid::try_from(message).map_err(|_| {
        // println!("({uas_id}) could not encode BasicMessage: {}", e);
        NetworkError::Other
    })?;

    let message = encoded.pack().map_err(|_| {
        // println!("({uas_id}) could not pack BasicMessage: {}", e);
        NetworkError::Other
    })?;

    let frame = Frame {
        header: Header {
            message_type: MessageType::Basic,
            ..Default::default()
        },
        message,
    };

    let payload = frame.pack().map_err(|_| {
        // println!("({uas_id}) could not pack Frame: {}", e);
        NetworkError::Other
    })?;

    let req = Request::builder()
        .method(Method::POST)
        .uri(format!("{url}/netrid"))
        .header("content-type", "application/octet-stream")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::from(payload.to_vec()))
        .unwrap();

    let result = client.request(req).await.map_err(|_| {
        // println!("({uas_id}) could not issue id update: {}", e);
        NetworkError::Other
    })?;

    if result.status() != StatusCode::OK {
        // println!(
        //     // "({uas_id}) could not issue id update: {}",
        //     result.status()
        // );
        return Err(NetworkError::Unauthorized);
    }

    // println!("({uas_id}) response {:#?}.", result);
    // println!("({uas_id}) issued id update.");
    Ok(())
}

/// Process Open Drone ID Location Data
pub fn process_open_drone_id_location_data(
    _sequence: u8,
    data: mavlink::common::OPEN_DRONE_ID_LOCATION_DATA
) -> Result<DecodedLocationRid, NetworkError> {
    let operational_status: OperationalStatus = FromPrimitive::from_u8(data.status as u8).unwrap_or(OperationalStatus::Undeclared);
    let height_type: HeightType = FromPrimitive::from_u8(data.height_reference as u8).unwrap_or(HeightType::AboveTakeoff);
    let horizontal_accuracy: HorizontalAccuracyMeters = FromPrimitive::from_u8(data.horizontal_accuracy as u8).unwrap_or(HorizontalAccuracyMeters::Gte18520);
    let vertical_accuracy: VerticalAccuracyMeters = FromPrimitive::from_u8(data.vertical_accuracy as u8).unwrap_or(VerticalAccuracyMeters::Gte150Unknown);
    let speed_accuracy: SpeedAccuracyMetersPerSecond = FromPrimitive::from_u8(data.speed_accuracy as u8).unwrap_or(SpeedAccuracyMetersPerSecond::Gte10Unknown);
    let barometric_altitude_accuracy: VerticalAccuracyMeters = FromPrimitive::from_u8(data.barometer_accuracy as u8).unwrap_or(VerticalAccuracyMeters::Gte150Unknown);
    let timestamp = Utc::now(); // PX4 sim doesn't set this field in the data
        // .with_minute(data.timestamp / 60)
        // .with_second(data.timestamp % 60) // from seconds after the hour to actual time

    let decoded_location = DecodedLocationRid {
        operational_status,
        height_type,
        horizontal_accuracy,
        vertical_accuracy,
        speed_accuracy,
        barometric_altitude_accuracy,
        track_direction: data.direction / 100, // centidegrees to degrees
        speed_mps: data.speed_horizontal as f32 / 100., // cm/s to m/s
        vertical_speed_mps: data.speed_vertical as f32 / 100., // cm/s to m/s
        latitude: data.latitude as f64 * 1e-7,
        longitude: data.longitude as f64 * 1e-7,
        pressure_altitude_meters: data.altitude_barometric,
        geodetic_altitude_meters: data.altitude_geodetic,
        height_meters: data.height,
        timestamp,
        timestamp_accuracy: (data.timestamp_accuracy as u8) as f32 / 10.
    };

    Ok(decoded_location)
}

/// Issue position update to network
pub(crate) async fn location_update(
    client: &Client<HttpConnector>,
    url: &str,
    token: &String,
    decoded_location: DecodedLocationRid
) -> Result<(), NetworkError> {
    let encoded_location = EncodedLocationRid::try_from(decoded_location).map_err(|e| {
        println!("could not encode location: {:?}", e);
        NetworkError::Other
    })?;

    let payload = Frame {
        header: Header {
            message_type: MessageType::Location,
            ..Default::default()
        },
        message: encoded_location.pack().map_err(|e| {
            println!("could not pack location frame: {}", e);
            NetworkError::Other
        })?
    }.pack().map_err(|e| {
        println!("could not pack location frame: {}", e);
        NetworkError::Other
    })?;

    let req = Request::builder()
        .method(Method::POST)
        .uri(format!("{url}/netrid"))
        .header("content-type", "application/octet-stream")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::from(payload.to_vec()))
        .unwrap();

    let result = client.request(req).await.map_err(|_| {
        // println!("({}) could not issue position update: {}", state.id, e);
        NetworkError::Other
    })?;

    if result.status() != StatusCode::OK {
        // println!(
        //     "({}) could not issue position update: {}",
        //     state.id,
        //     result.status()
        // );

        return Err(NetworkError::Unauthorized);
    }

    // println!("({}) response {:#?}.", state.id, result);
    // println!("({}) issued position update.", state.id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use svc_atc_client_rest::types::*;

    const METERS_PER_DEGREE_LATITUDE: f64 = 111_320.;
}