use svc_cargo_client_rest::types::*;
use hyper::{
    body::Body,
    client::connect::HttpConnector,
    client::Client,
    Method, Request, StatusCode,
};

use chrono::Utc;

pub(crate) async fn parcel_scan(
    client: &Client<HttpConnector>,
    identifier: &str,
    scanner_id: &str,
    cargo_id: &str,
    latitude: f64,
    longitude: f64,
    url: &str,
) -> Result<(), StatusCode> {
    // build NETRID Packet
    let message = CargoScan {
        scanner_id: scanner_id.to_string(),
        cargo_id: cargo_id.to_string(),
        latitude,
        longitude,
        timestamp: Utc::now()
    };

    let body = serde_json::to_string(&message).map_err(|e| {
        println!("({identifier}) could not serialize id update: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let req = Request::builder()
        .method(Method::PUT)
        .uri(format!("{url}/scan"))
        .header("content-type", "application/octet-stream")
        .body(Body::from(body))
        .unwrap();

    let result = client.request(req).await.map_err(|e| {
        println!("({identifier}) could not issue id update: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    if result.status() != StatusCode::OK {
        println!(
            "({identifier}) could not issue id update: {}",
            result.status()
        );

        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    // println!("({uas_id}) response {:#?}.", result);
    // println!("({uas_id}) issued id update.");
    Ok(())
}