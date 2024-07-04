use svc_atc_client_rest::types::{Pose, Cargo, Phase, FlightPlan as AtcFlightPlan};

// struct Mission {
//     session_id: String,
//     origin_timeslot_start: DateTime<Utc>,
//     origin_timeslot_end: DateTime<Utc>,
//     target_timeslot_start: DateTime<Utc>,
//     target_timeslot_end: DateTime<Utc>,
//     acquire: Vec<Cargo>,
//     deliver: Vec<Cargo>,
// }

#[derive(Serialize)]
#[serde(rename = "camelCase")]
struct Item {
    type_: u8,
    auto_continue: u8,
    command: u16,
    do_jump_id: u16,
    frame: u8,
    params: Vec<f32>,

    #[serde(rename = "Altitude")]
    altitude: f32,

    #[serde(rename = "AltitudeMode")]
    altitude_mode: u8,

    #[serde(rename = "AMSLAltAboveTerrain")]
    amsl_alt_above_terrain: f32,
}

#[derive(Serialize)]
#[serde(rename = "camelCase")]
struct Mission {
    cruise_speed: u32,
    firmware_type: u8,
    global_plan_altitude_mode: u8,
    hover_speed: u32,
    items: Vec<Item>,
    planned_home_position: [f32; 3],
    vehicle_type: u8,
    version: u8
}

#[derive(Serialize)]
#[serde(rename = "camelCase")]
struct GeoFence {
    circles: Vec<Circle>,
    polygons: Vec<Polygon>,
    version: u8,
}

#[derive(Serialize)]
#[serde(rename = "camelCase")]
struct RallyPoints {
    points: Vec<Pose>,
    version: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Plan {
    file_type: String,
    geo_fence: GeoFence,
    ground_station: String,
    mission: Mission,
    rally_points: RallyPoints,
    version: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plan_create() {
        let plan = Plan {
            file_type: 
        }
    }
}