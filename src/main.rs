// src/main.rs

use axum::{Router, extract::State, http::StatusCode, routing::get, serve};
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::net::TcpListener;

mod camera_handler;

#[derive(Clone)]
struct AppState {
    recording_active: Arc<AtomicBool>,
    stop_requested: Arc<AtomicBool>,
}

async fn hello_world() -> &'static str {
    "Hello, World!"
}

async fn handle_start_recording(
    State(state): State<Arc<AppState>>,
) -> Result<String, (StatusCode, String)> {
    if state
        .recording_active
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err((
            StatusCode::CONFLICT,
            "Recording is already in progress.".to_string(),
        ));
    }

    println!("Received request to start recording...");

    state.stop_requested.store(false, Ordering::SeqCst);
    println!("Stop request flag reset to false.");

    let state_clone = state.clone();

    tokio::spawn(async move {
        let stop_flag_clone = state_clone.stop_requested.clone();
        let result: Result<Result<PathBuf, anyhow::Error>, tokio::task::JoinError> =
            tokio::task::spawn_blocking(move || {
                camera_handler::run_recording_blocking(stop_flag_clone)
            })
            .await;

        match result {
            Ok(Ok(final_path)) => {
                println!(
                    "Background recording task finished successfully. Video saved to: {:?}",
                    final_path
                );
            }
            Ok(Err(e)) => {
                eprintln!("Background recording task failed: {}", e);
            }
            Err(e) => {
                eprintln!("Background recording task panicked or was cancelled: {}", e);
            }
        }

        state_clone.recording_active.store(false, Ordering::SeqCst);
        println!("Recording active flag reset to false.");
    });

    Ok("Recording started in the background.".to_string())
}

async fn handle_stop_recording(
    State(state): State<Arc<AppState>>,
) -> Result<String, (StatusCode, String)> {
    if !state.recording_active.load(Ordering::SeqCst) {
        return Err((
            StatusCode::CONFLICT,
            "Recording is not currently active or has already finished.".to_string(),
        ));
    }

    state.stop_requested.store(true, Ordering::SeqCst);
    println!("Stop request signal sent.");

    Ok("Stop request sent. Recording will finalize shortly.".to_string())
}

#[tokio::main]
async fn main() {
    let shared_state = Arc::new(AppState {
        recording_active: Arc::new(AtomicBool::new(false)),
        stop_requested: Arc::new(AtomicBool::new(false)),
    });

    let app = Router::new()
        .route("/", get(hello_world))
        .route("/start", get(handle_start_recording))
        .route("/stop", get(handle_stop_recording))
        .with_state(shared_state);

    const PORT: u16 = 8000;
    let addr = SocketAddr::from(([127, 0, 0, 1], PORT));
    println!("Listening on {}", addr);

    let listener = TcpListener::bind(addr)
        .await
        .expect("Failed to bind address");

    serve(listener, app.into_make_service())
        .await
        .expect("Server failed");
}
