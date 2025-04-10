// src/camera_handler.rs
use anyhow::{Context, Result};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

const SAVE_DIR_BASE: &str = "~/Desktop/recordings"; // 경로 확인 필요

pub fn run_recording_blocking(stop_requested: Arc<AtomicBool>) -> Result<PathBuf> {
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();

    // SAVE_DIR_BASE 경로 처리 (홈 디렉토리 '~' 확장)
    let save_dir_base_str = shellexpand::tilde(SAVE_DIR_BASE).into_owned();
    let save_dir = PathBuf::from(save_dir_base_str);
    if !save_dir.exists() {
        println!("Save directory {:?} does not exist. Creating it.", save_dir);
        fs::create_dir_all(&save_dir)
            .with_context(|| format!("Failed to create save directory: {:?}", save_dir))?;
    }

    let final_filename = format!("{}.mp4", timestamp);
    let final_path = save_dir.join(&final_filename);

    println!("Starting libcamera recording...");

    // libcamera-vid 명령어 실행
    let mut child = Command::new("libcamera-vid")
        .arg("--width")
        .arg("1280")
        .arg("--height")
        .arg("720")
        .arg("--framerate")
        .arg("24")
        .arg("--output")
        .arg(final_path.to_str().context("Invalid output path")?)
        .spawn()
        .context("Failed to start libcamera-vid")?;

    // Stop 요청을 대기
    while !stop_requested.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(100));
    }

    println!("Stop signal received. Terminating libcamera-vid...");

    // libcamera-vid 프로세스 종료
    child.kill().context("Failed to stop libcamera-vid")?;
    child
        .wait()
        .context("Failed to wait for libcamera-vid to exit")?;

    println!("Recording complete. Video saved to: {:?}", final_path);
    Ok(final_path)
}
