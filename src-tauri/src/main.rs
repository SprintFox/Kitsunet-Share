// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use gethostname::gethostname;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::oneshot;
use tokio::time::interval;
use tauri::AppHandle;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::path::PathBuf;
use uuid::Uuid;
use network_interface::{NetworkInterface, NetworkInterfaceConfig};

const DISCOVERY_PORT: u16 = 5000;
const FILE_TRANSFER_PORT: u16 = 5001;
const PEER_TIMEOUT_SECS: u64 = 2;


#[derive(Debug, Serialize, Deserialize, Clone)]
struct Peer {
    username: String,
    address: String,
    #[serde(skip)]
    last_seen: Option<Instant>,
}

impl PartialEq for Peer {
    fn eq(&self, other: &Self) -> bool {
        self.address == other.address
    }
}

impl Eq for Peer {}

impl std::hash::Hash for Peer {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.address.hash(state);
    }
}

#[derive(Debug, Serialize, Deserialize)]
enum Message {
    Presence(String),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct UserSettings {
    username: String,
    broadcasting_enabled: bool,
    broadcast_address: String,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            username: gethostname().into_string().unwrap_or_else(|_| "Unknown".to_string()),
            broadcasting_enabled: true,
            broadcast_address: "255.255.255.255".to_string(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct NetworkInterfaceInfo {
    name: String,
    ip: String,
    broadcast: String,
}

type FileOffers = Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>;

#[derive(Debug, Default)]
struct SharedState {
    peers: HashSet<Peer>,
    settings: UserSettings,
}

#[derive(Debug, Default)]
struct AppState(Arc<Mutex<SharedState>>);

#[tauri::command]
fn get_network_interfaces() -> Vec<NetworkInterfaceInfo> {
    let mut interfaces = vec![];
    if let Ok(ifaces) = network_interface::NetworkInterface::show() {
        for iface in ifaces {
            if iface.name == "lo" {
                continue;
            }
            for addr in iface.addr {
                if let Some(broadcast) = addr.broadcast() {
                    if let std::net::IpAddr::V4(ipv4) = addr.ip() {
                        interfaces.push(NetworkInterfaceInfo {
                            name: iface.name.clone(),
                            ip: ipv4.to_string(),
                            broadcast: broadcast.to_string(),
                        });
                    }
                }
            }
        }
    }
    interfaces.push(NetworkInterfaceInfo {
                            name: "All".to_string(),
                            ip: "255.255.255.255".to_string(),
                            broadcast: "255.255.255.255".to_string(),
                        });
    interfaces
}

#[tauri::command]
fn get_users(state: tauri::State<AppState>) -> Vec<Peer> {
    let state = state.0.lock().unwrap();
    state.peers.iter().cloned().collect()
}

#[tauri::command]
fn get_settings(state: tauri::State<AppState>) -> UserSettings {
    let state = state.0.lock().unwrap();
    state.settings.clone()
}

#[tauri::command]
fn update_settings(settings: UserSettings, state: tauri::State<AppState>) {
    let mut state = state.0.lock().unwrap();
    state.settings = settings;
}

#[tauri::command]
async fn get_own_address() -> Result<String, String> {
    let socket = UdpSocket::bind("0.0.0.0:0").await.map_err(|e| e.to_string())?;
    socket.connect("8.8.8.8:80").await.map_err(|e| e.to_string())?;
    let local_addr = socket.local_addr().map_err(|e| e.to_string())?;
    Ok(local_addr.ip().to_string())
}

#[tauri::command]
async fn send_files(
    app: AppHandle,
    recipient: String,
    file_paths: Vec<String>,
) -> Result<(), String> {
    let mut files_metadata = Vec::new();
    for path_str in &file_paths {
        let path = PathBuf::from(path_str);
        let file_name = path.file_name()
            .ok_or_else(|| "A file path is invalid".to_string())?
            .to_str()
            .ok_or_else(|| "A file name is not valid UTF-8".to_string())?;
        let file_size = tokio::fs::metadata(path_str).await.map_err(|e| e.to_string())?.len();
        files_metadata.push(FileMetadata { name: file_name.to_string(), size: file_size });
    }

    let target_addr = format!("{}:{}", recipient, FILE_TRANSFER_PORT);
    let mut stream = TcpStream::connect(target_addr).await.map_err(|e| e.to_string())?;

    let metadata_json = serde_json::to_string(&files_metadata).map_err(|e| e.to_string())?;
    let metadata_bytes = metadata_json.as_bytes();

    // Send metadata length and metadata
    stream.write_u64(metadata_bytes.len() as u64).await.map_err(|e| e.to_string())?;
    stream.write_all(metadata_bytes).await.map_err(|e| e.to_string())?;

    // Wait for acceptance
    let mut response = [0; 1];
    stream.read_exact(&mut response).await.map_err(|e| e.to_string())?;
    if response[0] != 1 {
        return Err("File transfer rejected by recipient".to_string());
    }

    for path_str in &file_paths {
        let mut file = tokio::fs::File::open(path_str).await.map_err(|e| e.to_string())?;
        let file_size = file.metadata().await.map_err(|e| e.to_string())?.len();
        let mut sent_for_file: u64 = 0;
        
        let mut buffer = vec![0; 1024 * 1024]; // 1MB buffer
        loop {
            let bytes_read = file.read(&mut buffer).await.map_err(|e| e.to_string())?;
            if bytes_read == 0 {
                break;
            }
            stream.write_all(&buffer[..bytes_read]).await.map_err(|e| e.to_string())?;
            
            sent_for_file += bytes_read as u64;
            app.emit("transfer-progress", FileTransferProgress {
                file_path: Some(path_str.to_string()),
                file_name: None,
                progress: (sent_for_file as f64 / file_size as f64) * 100.0,
            }).unwrap();
        }
        app.emit("transfer-complete", FileTransferComplete {
            file_path: Some(path_str.to_string()),
            file_name: None,
            saved_path: None,
        }).unwrap();
    }

    Ok(())
}

#[tauri::command]
async fn accept_file_offer(offer_id: String, offers: tauri::State<'_, FileOffers>) -> Result<(), String> {
    if let Some(sender) = offers.lock().unwrap().remove(&offer_id) {
        sender.send(true).map_err(|_| "Failed to send acceptance".to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn reject_file_offer(offer_id: String, offers: tauri::State<'_, FileOffers>) -> Result<(), String> {
    if let Some(sender) = offers.lock().unwrap().remove(&offer_id) {
        sender.send(false).map_err(|_| "Failed to send rejection".to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn show_in_folder(path: String) {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .args(["/select,", &path])
            .spawn()
            .expect("failed to open explorer");
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(parent) = std::path::Path::new(&path).parent() {
            std::process::Command::new("xdg-open")
                .arg(parent)
                .spawn()
                .expect("failed to open file manager");
        }
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", &path])
            .spawn()
            .expect("failed to open finder");
    }
}


#[derive(Clone, serde::Serialize, Deserialize, Debug)]
struct FileMetadata {
    name: String,
    size: u64,
}

#[derive(Clone, serde::Serialize, Debug)]
struct FileTransferProgress {
    #[serde(skip_serializing_if = "Option::is_none")]
    file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_name: Option<String>,
    progress: f64,
}

#[derive(Clone, serde::Serialize, Debug)]
struct FileTransferComplete {
    #[serde(skip_serializing_if = "Option::is_none")]
    file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    saved_path: Option<PathBuf>,
}

#[derive(Clone, serde::Serialize)]
struct BatchFileOfferPayload {
    id: String,
    from: String,
    files: Vec<FileMetadata>,
    total_size: u64,
}

use std::error::Error;

async fn handle_incoming_batch(
    app: AppHandle,
    mut stream: TcpStream,
    remote_addr: std::net::SocketAddr,
    offers: FileOffers,
) {
    let result: Result<(), Box<dyn Error + Send + Sync>> = async {
        // Read metadata
        let metadata_len = stream.read_u64().await? as usize;
        let mut metadata_bytes = vec![0; metadata_len];
        stream.read_exact(&mut metadata_bytes).await?;
        let files: Vec<FileMetadata> = serde_json::from_slice(&metadata_bytes)
            .map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)?;

        let total_size = files.iter().map(|f| f.size).sum();

        let offer_id = Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        offers.lock().unwrap().insert(offer_id.clone(), tx);

        app.emit("file-offer", BatchFileOfferPayload {
            id: offer_id.clone(),
            from: remote_addr.ip().to_string(),
            files: files.clone(),
            total_size,
        }).map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)?;

        if let Ok(true) = rx.await {
            // Send acceptance byte
            stream.write_all(&[1]).await?;

            let download_dir = match app.path().download_dir() {
                Ok(path) => path,
                Err(_) => return Err(Box::new(std::io::Error::new(std::io::ErrorKind::NotFound, "Download directory not found")) as Box<dyn Error + Send + Sync>),
            };

            for file_meta in files {
                let file_path = download_dir.join(&file_meta.name);
                let mut file = tokio::fs::File::create(&file_path).await?;

                let mut received_for_file: u64 = 0;
                let mut buffer = vec![0; 1024 * 1024]; // 1MB buffer

                while received_for_file < file_meta.size {
                    let bytes_to_read = std::cmp::min(buffer.len() as u64, file_meta.size - received_for_file) as usize;
                    let bytes_read = stream.read(&mut buffer[..bytes_to_read]).await?;
                    if bytes_read == 0 {
                        return Err(Box::new(std::io::Error::new(std::io::ErrorKind::ConnectionAborted, "Connection closed prematurely")));
                    }
                    file.write_all(&buffer[..bytes_read]).await?;
                    received_for_file += bytes_read as u64;
                    
                    app.emit("transfer-progress", FileTransferProgress {
                        file_path: None,
                        file_name: Some(file_meta.name.clone()),
                        progress: (received_for_file as f64 / file_meta.size as f64) * 100.0,
                    }).map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)?;
                }
                app.emit("transfer-complete", FileTransferComplete {
                    file_path: None,
                    file_name: Some(file_meta.name.clone()),
                    saved_path: Some(file_path),
                }).map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)?;
            }
        } else {
            // Send rejection byte
            stream.write_all(&[0]).await?;
            println!("File offer for batch rejected or timed out");
        }

        Ok(())
    }.await;

    if let Err(e) = result {
        eprintln!("Error handling incoming file batch: {}", e);
    }
}


async fn file_receiver_task(app: AppHandle, offers: FileOffers) {
    let listener = TcpListener::bind(format!("0.0.0.0:{}", FILE_TRANSFER_PORT))
        .await
        .expect("Failed to bind TCP listener");

    loop {
        if let Ok((stream, remote_addr)) = listener.accept().await {
            println!("Accepted connection from {}", remote_addr);
            let app_clone = app.clone();
            let offers_clone = offers.clone();
            tokio::spawn(handle_incoming_batch(app_clone, stream, remote_addr, offers_clone));
        }
    }
}

async fn discovery_task(app_handle: tauri::AppHandle) {
    let state = app_handle.state::<AppState>();
    let socket = UdpSocket::bind(format!("0.0.0.0:{}", DISCOVERY_PORT))
        .await
        .expect("Не удалось привязать сокет");
    socket
        .set_broadcast(true)
        .expect("Не удалось установить broadcast");

    let mut broadcast_interval = interval(Duration::from_secs(1));
    let mut recv_buf = vec![0u8; 1024];

    loop {
        tokio::select! {
            _ = broadcast_interval.tick() => {
                // Peer cleanup
                {
                    let mut state = state.0.lock().unwrap();
                    let now = Instant::now();
                    let old_peer_count = state.peers.len();
                    state.peers.retain(|peer| {
                        if let Some(last_seen) = peer.last_seen {
                            now.duration_since(last_seen).as_secs() < PEER_TIMEOUT_SECS
                        } else {
                            false
                        }
                    });
                    if state.peers.len() < old_peer_count {
                        app_handle.emit("peers_updated", ()).unwrap();
                    }
                }

                // Broadcasting
                let (username, broadcasting_enabled, broadcast_address) = {
                    let state = state.0.lock().unwrap();
                    (   
                        state.settings.username.clone(),
                        state.settings.broadcasting_enabled,
                        state.settings.broadcast_address.clone(),
                    )
                };

                if broadcasting_enabled {
                    let message = Message::Presence(username);
                    let bytes = serde_json::to_vec(&message).unwrap();

                    if broadcast_address == "255.255.255.255" {
                        // "All" mode: broadcast on all interfaces
                        if let Ok(ifaces) = network_interface::NetworkInterface::show() {
                            for iface in ifaces {
                                for addr in &iface.addr {
                                    if let Some(broadcast) = addr.broadcast() {
                                        let target_addr = format!("{}:{}", broadcast, DISCOVERY_PORT);
                                        if let Err(e) = socket.send_to(&bytes, &target_addr).await {
                                            eprintln!("Не удалось отправить broadcast на {}: {}", target_addr, e);
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        // Specific interface mode: broadcast to the given address
                        let target_addr = format!("{}:{}", broadcast_address, DISCOVERY_PORT);
                        if let Err(e) = socket.send_to(&bytes, &target_addr).await {
                            eprintln!("Не удалось отправить broadcast на {}: {}", target_addr, e);
                        }
                    }
                }
            }
            Ok((len, remote_addr)) = socket.recv_from(&mut recv_buf) => {
                let mut local_ips = HashSet::new();
                if let Ok(ifaces) = NetworkInterface::show() {
                    for iface in ifaces {
                        for addr in &iface.addr {
                            local_ips.insert(addr.ip());
                        }
                    }
                }
                if local_ips.contains(&remote_addr.ip()) {
                    continue;
                }

                if let Ok(message) = serde_json::from_slice::<Message>(&recv_buf[..len]) {
                    let Message::Presence(username) = message;
                    let new_peer = Peer {
                        username,
                        address: remote_addr.ip().to_string(),
                        last_seen: Some(Instant::now()),
                    };

                    let mut state = state.0.lock().unwrap();
                    if match state.peers.replace(new_peer.clone()) {
                        None => true, // It's a new peer
                        Some(old) => old.username != new_peer.username, // It's an existing peer, check if username changed
                    } {
                        app_handle.emit("peers_updated", ()).unwrap();
                    }
                }
            }
        }
    }
}

fn main() {
    let state = AppState::default();
    let offers: FileOffers = Arc::new(Mutex::new(HashMap::new()));

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "openbsd", target_os = "netbsd" ))]
    std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");

    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .manage(offers)
        .invoke_handler(tauri::generate_handler![
            get_users,
            send_files,
            get_own_address,
            get_settings,
            update_settings,
            accept_file_offer,
            reject_file_offer,
            get_network_interfaces,
            show_in_folder
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let offers = app.state::<FileOffers>().inner().clone();
            tauri::async_runtime::spawn(discovery_task(handle.clone()));
            tauri::async_runtime::spawn(file_receiver_task(handle.clone(), offers));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Ошибка запуска приложения");
}