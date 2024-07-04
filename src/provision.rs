use hyper::{client::{connect::HttpConnector, Client}, Body, Request, Method, StatusCode};

fn get_rectangle(latitude: f64, longitude: f64) -> Vec<(f64, f64)> {
    const MARGIN: f64 = 0.0001;
    vec![
        (latitude + MARGIN, longitude + MARGIN),
        (latitude + MARGIN, longitude - MARGIN),
        (latitude - MARGIN, longitude - MARGIN),
        (latitude - MARGIN, longitude + MARGIN),
        (latitude + MARGIN, longitude + MARGIN),
    ]
}

pub async fn provision_hangar(
    client: &Client<HttpConnector>,
    url: &str,
    latitude: f64,
    longitude: f64,
    px4_instance: u32,
) -> Result<(String, String), ()> {
    let hangar_id: String;
    let hangar_bay_id: String;

    {
        let payload = svc_itest_client_rest::types::AddVertiportRequest {
            label: format!("HANGAR-{px4_instance}"),
            address: "".to_string(),
            vertices: get_rectangle(latitude, longitude),
            altitude: 0.0
        };

        let payload = serde_json::to_string(&payload).map_err(|e| {
            println!("could not serialize addvertiportrequest payload: {}", e);
        })?;

        let req = Request::builder()
            .method(Method::PUT)
            .uri(format!("{url}/vertiport"))
            .header("content-type", "application/json")
            .body(Body::from(payload))
            .unwrap();

        let result = client.request(req).await.map_err(|e| {
            println!("could not add vertiport to database: {}", e);
            ()
        })?;

        if result.status() != StatusCode::OK {
            println!(
                "Hangar database addition failed: {}",
                result.status()
            );
            return Err(());
        }

        let bytes = hyper::body::to_bytes(result.into_body()).await.unwrap();
        hangar_id = serde_json::from_slice(&bytes).unwrap();
    }

    {
        let payload = svc_itest_client_rest::types::AddVertipadRequest {
            vertiport_id: hangar_id.clone(),
            latitude,
            longitude,
            altitude: 0.0,
            label: "PAD-0".to_string()
        };

        let payload = serde_json::to_string(&payload).map_err(|e| {
            println!("could not serialize addvertiportrequest payload: {}", e);
        })?;

        let req = Request::builder()
            .method(Method::PUT)
            .uri(format!("{url}/vertiport"))
            .header("content-type", "application/json")
            .body(Body::from(payload))
            .unwrap();

        let result = client.request(req).await.map_err(|e| {
            println!("could not add vertiport to database: {}", e);
            ()
        })?;

        if result.status() != StatusCode::OK {
            println!(
                "Hangar bay database addition failed: {}",
                result.status()
            );
            return Err(());
        }

        let bytes = hyper::body::to_bytes(result.into_body()).await.unwrap();
        hangar_bay_id = serde_json::from_slice(&bytes).unwrap();
    }

    Ok((hangar_id, hangar_bay_id))
}

pub async fn provision_aircraft(
    client: &Client<HttpConnector>,
    url: &str,
    identifier: &str,
    hangar_id: &str,
    _hangar_bay_id: &str,
    px4_instance: u32
) -> Result<String, ()> {
    let aircraft_id: String;

    // Delete any aircraft with the same identifier
    {
        let payload = svc_itest_client_rest::types::DeleteAircraftRequest {
            registration_number: identifier.to_string()
        };

        let payload = serde_json::to_string(&payload).map_err(|e| {
            println!("({identifier}) could not serialize id update: {}", e);
        })
        .map_err(|_| {
            println!("({identifier}) could not serialize id update.");
        })?;

        let req = Request::builder()
            .method(Method::DELETE)
            .uri(format!("{url}/aircraft"))
            .header("content-type", "application/json")
            .body(Body::from(payload))
            .unwrap();

        let result = client.request(req).await.map_err(|e| {
            println!("({identifier}) could not issue id update: {}", e);
            ()
        })?;

        if result.status() != StatusCode::OK {
            println!(
                "({identifier}) could not issue id update: {}",
                result.status()
            );
            return Err(());
        }
    }

    // Add the aircraft to the database
    {
        let payload = svc_itest_client_rest::types::AddAircraftRequest {
            registration_number: identifier.to_string(),
            hangar_id: hangar_id.to_string(),
            hangar_bay_id: format!("BAY-{px4_instance}"),
            nickname: format!("PX4-{px4_instance}"),
        };

        let payload = serde_json::to_string(&payload).map_err(|e| {
            println!("({identifier}) could not serialize addaircraftrequest payload: {}", e);
        })?;

        let req = Request::builder()
            .method(Method::PUT)
            .uri(format!("{url}/aircraft"))
            .header("content-type", "application/json")
            .body(Body::from(payload))
            .unwrap();

        let result = client.request(req).await.map_err(|e| {
            println!("({identifier}) could not add aircraft to database: {}", e);
            ()
        })?;

        if result.status() != StatusCode::OK {
            println!(
                "({identifier}) database addition failed: {}",
                result.status()
            );
            return Err(());
        }
        let bytes = hyper::body::to_bytes(result.into_body()).await.unwrap();
        aircraft_id = serde_json::from_slice(&bytes).unwrap();
    }

    Ok(aircraft_id)
}