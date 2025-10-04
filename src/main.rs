// ============================================
// main.rs
// ============================================
mod websocket;
mod message;

use warp::Filter;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use websocket::{Rooms, client_connected};

#[tokio::main]
async fn main() {
    println!("🚀 Bini Chat Server v1.0.0");
    println!("🌐 Open http://127.0.0.1:3030/ in your browser");

    let rooms: Rooms = Arc::new(Mutex::new(HashMap::new()));

    let chat_rooms = warp::path("ws")
        .and(warp::ws())
        .and(warp::query::<HashMap<String, String>>())
        .and(with_rooms(rooms.clone()))
        .map(|ws: warp::ws::Ws, qs: HashMap<String, String>, rooms| {
            let username = qs.get("user").cloned().unwrap_or("Guest".to_string());
            let room_name = qs.get("room_name").cloned().unwrap_or("default".to_string());
            let create_room = qs.get("create").map_or(false, |v| v == "true");

            ws.on_upgrade(move |socket| client_connected(socket, rooms, username, room_name, create_room))
        });

    let static_files = warp::fs::dir("static");
    let routes = chat_rooms.or(static_files);

    warp::serve(routes).run(([127, 0, 0, 1], 3030)).await;
}

fn with_rooms(rooms: Rooms) -> impl Filter<Extract = (Rooms,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || rooms.clone())
}