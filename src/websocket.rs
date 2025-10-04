// ============================================
// websocket.rs
// ============================================
use std::sync::{Arc, Mutex};
use std::collections::HashMap;

use futures::{StreamExt, SinkExt};
use tokio::sync::mpsc;
use warp::ws::{WebSocket, Message};

use crate::message::Message as ChatMessage;
use serde_json::json;

pub type Rooms = Arc<Mutex<HashMap<String, Room>>>;

#[derive(Clone)]
pub struct Client {
    pub username: String,
    pub sender: mpsc::UnboundedSender<Message>,
}

pub struct Room {
    pub name: String,
    pub admin: String,
    pub clients: Vec<Client>,
    pub message_history: Vec<ChatMessage>,
}

impl Room {
    pub fn new(name: String, admin: String) -> Self {
        Self {
            name,
            admin,
            clients: Vec::new(),
            message_history: Vec::new(),
        }
    }
}

fn send_to_senders(senders: Vec<mpsc::UnboundedSender<Message>>, text: &str) {
    for s in senders {
        s.send(Message::text(text)).ok();
    }
}

fn build_users_list_msg(room: &Room) -> String {
    let users_info: Vec<_> = room.clients.iter().map(|c| {
        json!({
            "username": c.username,
            "admin": c.username == room.admin
        })
    }).collect();
    format!("__users__{}", serde_json::to_string(&users_info).unwrap())
}

fn build_typing_msg(username: &str) -> String {
    format!("__typing__{}", username)
}

pub async fn client_connected(
    ws: WebSocket,
    rooms: Rooms,
    username: String,
    room_name: String,
    create_room: bool,
) {
    let (mut ws_tx, mut ws_rx) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();

    // Spawn forwarder task: server -> client
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    // --- Join or create room ---
    let (join_senders, join_users_msg, join_broadcast_msg, history_messages) = {
        let mut rooms_guard = rooms.lock().unwrap();

        if create_room && !rooms_guard.contains_key(&room_name) {
            rooms_guard.insert(room_name.clone(), Room::new(room_name.clone(), username.clone()));
            println!("📢 Room '{}' created by {}", room_name, username);
        }

        if let Some(room) = rooms_guard.get_mut(&room_name) {
            // Collect message history for this new client
            let history: Vec<String> = room.message_history.iter()
                .map(|msg| serde_json::to_string(msg).unwrap())
                .collect();

            // add client
            room.clients.push(Client { username: username.clone(), sender: tx.clone() });

            // prepare list of senders and messages
            let senders: Vec<_> = room.clients.iter().map(|c| c.sender.clone()).collect();
            let users_msg = build_users_list_msg(room);
            let join_msg = format!("✅ {} joined '{}'", username, room.name);
            (senders, users_msg, join_msg, history)
        } else {
            // room not found -> inform only this client
            (vec![tx.clone()], String::new(), "❌ Room not found".to_string(), Vec::new())
        }
    };

    // Send message history to new client first
    for history_msg in history_messages {
        tx.send(Message::text(history_msg)).ok();
    }

    // Send join / users update (if room existed)
    if !join_users_msg.is_empty() {
        send_to_senders(join_senders.clone(), &join_broadcast_msg);
        send_to_senders(join_senders, &join_users_msg);
    } else {
        // room not found case: notify only the connecting client
        tx.send(Message::text(join_broadcast_msg)).ok();
        return;
    }

    // --- Message handling loop ---
    while let Some(result) = ws_rx.next().await {
        let msg = match result {
            Ok(m) => m,
            Err(_) => break,
        };

        if let Ok(text) = msg.to_str() {
            let mut should_remove_room: bool = false;
            let mut recipients: Vec<mpsc::UnboundedSender<Message>> = Vec::new();
            let mut notification_text: Option<String> = None;

            {
                let mut rooms_guard = rooms.lock().unwrap();
                if let Some(room) = rooms_guard.get_mut(&room_name) {
                    if text == "/end" && room.admin == username {
                        // Send room ended signal to all clients
                        recipients = room.clients.iter().map(|c| c.sender.clone()).collect();
                        notification_text = Some("__room_ended__".to_string());
                        should_remove_room = true;
                    } else if text.starts_with("__typing__") {
                        // send typing to everyone except the typer
                        let typing_text = build_typing_msg(&username);
                        let recipients_typing: Vec<_> = room.clients.iter()
                            .filter(|c| c.username != username)
                            .map(|c| c.sender.clone())
                            .collect();
                        drop(rooms_guard);
                        send_to_senders(recipients_typing, &typing_text);
                        continue;
                    } else {
                        // regular chat message: construct ChatMessage and store in history
                        let mut chat_msg = ChatMessage::new(&username, &room.name, text);
                        if room.admin == username {
                            chat_msg.admin = Some(true);
                        }

                        // Store message in history
                        room.message_history.push(chat_msg.clone());

                        let json_msg = serde_json::to_string(&chat_msg).unwrap();
                        recipients = room.clients.iter().map(|c| c.sender.clone()).collect();
                        notification_text = Some(json_msg);
                    }
                }
            }

            // act after lock
            if let Some(txt) = notification_text {
                send_to_senders(recipients.clone(), &txt);
            }

            if should_remove_room {
                let mut rooms_guard = rooms.lock().unwrap();
                rooms_guard.remove(&room_name);
                println!("🗑 Room '{}' closed and removed", room_name);
                break;
            }
        }
    }

    // --- Disconnect cleanup ---
    let (senders_after_leave, users_msg_after_leave, remove_room_now, new_admin_username, left_msg) = {
        let mut rooms_guard = rooms.lock().unwrap();
        if let Some(room) = rooms_guard.get_mut(&room_name) {
            room.clients.retain(|c| c.username != username);

            let recipients: Vec<_> = room.clients.iter().map(|c| c.sender.clone()).collect();
            let left_msg = format!("⚠️ {} left '{}'", username, room.name);
            let users_msg = build_users_list_msg(room);

            if room.clients.is_empty() {
                (recipients, users_msg, true, None, left_msg)
            } else {
                let new_admin_user: Option<String> = if room.admin == username {
                    room.admin = room.clients[0].username.clone();
                    Some(room.admin.clone())
                } else {
                    None
                };
                (recipients, users_msg, false, new_admin_user, left_msg)
            }
        } else {
            (Vec::new(), String::new(), false, None, String::new())
        }
    };

    if !senders_after_leave.is_empty() {
        send_to_senders(senders_after_leave.clone(), &left_msg);

        if !users_msg_after_leave.is_empty() {
            send_to_senders(senders_after_leave.clone(), &users_msg_after_leave);
        }

        // Send admin change signal to new admin
        if let Some(new_admin) = new_admin_username {
            let admin_signal = format!("__new_admin__{}", new_admin);
            send_to_senders(senders_after_leave, &admin_signal);
        }

        if remove_room_now {
            let mut rooms_guard = rooms.lock().unwrap();
            rooms_guard.remove(&room_name);
            println!("🗑 Room '{}' removed (empty)", room_name);
        }
    }
}