// src/camera_handler.rs
use anyhow::{Context, Result};
use opencv::{
    core::{self, Mat, Size, Vector},
    imgproc,
    prelude::*,
    videoio::{self, VideoCapture, VideoWriter},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const FRAME_WIDTH: i32 = 1280;
const FRAME_HEIGHT: i32 = 720;
const REQUESTED_FPS: f64 = 24.0;
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

    let temp_filename = format!("{}_temp.avi", timestamp);
    let final_filename = format!("{}.avi", timestamp);
    let temp_path = save_dir.join(&temp_filename);
    let final_path = save_dir.join(&final_filename);

    let combined_width = FRAME_WIDTH * 2;

    println!("Attempting to open cameras for recording...");

    let mut cam0 = VideoCapture::new(0, videoio::CAP_ANY).context("Failed to open camera 0")?;
    let mut cam1 = VideoCapture::new(1, videoio::CAP_ANY).context("Failed to open camera 1")?;

    // 카메라 설정 (프레임 크기, FPS)
    cam0.set(videoio::CAP_PROP_FRAME_WIDTH, FRAME_WIDTH as f64)?;
    cam0.set(videoio::CAP_PROP_FRAME_HEIGHT, FRAME_HEIGHT as f64)?;
    cam0.set(videoio::CAP_PROP_FPS, REQUESTED_FPS)?;
    cam1.set(videoio::CAP_PROP_FRAME_WIDTH, FRAME_WIDTH as f64)?;
    cam1.set(videoio::CAP_PROP_FRAME_HEIGHT, FRAME_HEIGHT as f64)?;
    cam1.set(videoio::CAP_PROP_FPS, REQUESTED_FPS)?;

    if !cam0.is_opened()? || !cam1.is_opened()? {
        anyhow::bail!("Could not open one or both cameras.");
    }
    println!("Cameras opened successfully.");
    thread::sleep(Duration::from_secs(1)); // 카메라 안정화 시간

    // 비디오 라이터 설정
    let fourcc = VideoWriter::fourcc('X', 'V', 'I', 'D')?;
    let mut temp_writer = VideoWriter::new(
        temp_path.to_str().context("Invalid temporary path")?,
        fourcc,
        REQUESTED_FPS, // 초기에는 요청 FPS로 기록
        Size::new(combined_width, FRAME_HEIGHT),
        true, // 컬러 영상
    )
    .context("Failed to create temporary VideoWriter")?;

    if !temp_writer.is_opened()? {
        anyhow::bail!("Could not open temporary video writer.");
    }

    println!("Recording started. Waiting for stop signal or error.");

    let frame_interval = Duration::from_secs_f64(1.0 / REQUESTED_FPS);
    let mut frame_count = 0u64;
    let start_time = Instant::now();

    // 프레임 저장용 Mat 변수들
    let mut frame0 = Mat::default();
    let mut frame1 = Mat::default();
    let mut combined = Mat::default(); // 합쳐진 프레임 저장용
    let mut combined_frames_vec = Vector::<Mat>::new(); // hconcat 입력용 벡터

    // 비디오 라이터 안전 해제를 위한 Guard
    let writer_guard = VideoWriterGuard(&mut temp_writer);

    // --- 메인 레코딩 루프 ---
    while !stop_requested.load(Ordering::SeqCst) {
        let loop_start = Instant::now();

        // 카메라에서 프레임 읽기
        let read0_ok = cam0
            .read(&mut frame0)
            .context("Failed to read from camera 0")?;
        let read1_ok = cam1
            .read(&mut frame1)
            .context("Failed to read from camera 1")?;

        // 프레임 읽기 실패 또는 빈 프레임 처리
        if !read0_ok || !read1_ok || frame0.empty() || frame1.empty() {
            eprintln!("Frame drop detected or camera read failed. Skipping.");
            if stop_requested.load(Ordering::Relaxed) {
                break;
            }
            thread::sleep(Duration::from_millis(50)); // 잠시 대기 후 재시도
            continue;
        }

        // 프레임 크기 강제 조정 (필요 시) - 가급적이면 카메라 설정에서 맞추는 것이 좋음
        if frame0.cols() != FRAME_WIDTH
            || frame0.rows() != FRAME_HEIGHT
            || frame1.cols() != FRAME_WIDTH
            || frame1.rows() != FRAME_HEIGHT
        {
            eprintln!(
                "Warning: Captured frame dimensions differ (Cam0: {}x{}, Cam1: {}x{}). Resizing to {}x{}.",
                frame0.cols(),
                frame0.rows(),
                frame1.cols(),
                frame1.rows(),
                FRAME_WIDTH,
                FRAME_HEIGHT
            );
            let target_size = Size::new(FRAME_WIDTH, FRAME_HEIGHT);
            imgproc::resize(
                &frame0,
                &mut frame0.clone(),
                target_size,
                0.0,
                0.0,
                imgproc::INTER_LINEAR,
            )?;
            imgproc::resize(
                &frame1,
                &mut frame1.clone(),
                target_size,
                0.0,
                0.0,
                imgproc::INTER_LINEAR,
            )?;
        }

        frame_count += 1; // 유효 프레임 카운트 증가

        // 보정된 프레임들을 수평으로 연결 (Cam1 | Cam0 순서)
        combined_frames_vec.clear();
        combined_frames_vec.push(frame1.clone());
        combined_frames_vec.push(frame0.clone());
        core::hconcat(&combined_frames_vec, &mut combined)
            .context("Failed to horizontally concatenate frames")?;

        if stop_requested.load(Ordering::Relaxed) {
            break;
        } // 쓰기 전에 한 번 더 확인

        // 연결된 프레임을 임시 파일에 쓰기
        writer_guard
            .0
            .write(&combined)
            .context("Failed to write frame to temporary file")?;

        if stop_requested.load(Ordering::Relaxed) {
            break;
        } // 쓰기 후에도 확인

        // FPS 유지 위한 대기 시간 계산 및 적용
        let elapsed_loop = loop_start.elapsed();
        if elapsed_loop < frame_interval {
            if let Some(remaining) = frame_interval.checked_sub(elapsed_loop) {
                thread::sleep(remaining);
            }
        } else {
            // 루프 처리 시간이 프레임 간격보다 길 경우 경고 (디버깅 목적)
            // eprintln!("Warning: Loop processing time ({:?}) exceeded frame interval ({:?})", elapsed_loop, frame_interval);
        }
    }

    // --- 루프 종료 후 처리 ---
    if stop_requested.load(Ordering::SeqCst) {
        println!("Recording loop exited due to stop request.");
    } else {
        println!("Recording loop finished (possibly due to error or end of stream).");
    }

    println!("Finalizing video...");
    let total_elapsed = start_time.elapsed();
    let real_fps = if total_elapsed.as_secs_f64() > 0.0 && frame_count > 0 {
        frame_count as f64 / total_elapsed.as_secs_f64()
    } else {
        REQUESTED_FPS // 프레임이 없거나 시간이 0이면 요청 FPS 사용
    };
    println!(
        "Recorded {} frames in {:.1}s. Actual avg FPS: {:.2}",
        frame_count,
        total_elapsed.as_secs_f64(),
        real_fps
    );

    // VideoWriterGuard를 drop하여 임시 파일 닫기 및 해제
    drop(writer_guard);

    // 기록된 프레임이 있을 경우 실제 FPS로 리인코딩
    if frame_count > 0 {
        println!("Re-encoding video with actual FPS...");
        reencode_video(&temp_path, &final_path, real_fps) // 실제 FPS 전달
            .context("Failed during video re-encoding")?;
        fs::remove_file(&temp_path)
            .with_context(|| format!("Failed to remove temporary file: {:?}", temp_path))?;
        println!(
            "Recording complete. Final video saved to: {:?} ({:.2} FPS)",
            final_path, real_fps
        );
        Ok(final_path) // 최종 파일 경로 반환
    } else {
        // 기록된 프레임이 없을 경우 임시 파일 삭제 및 오류 반환
        println!("No frames were recorded. Cleaning up temporary file.");
        let _ = fs::remove_file(&temp_path); // 삭제 실패는 무시
        anyhow::bail!(
            "No frames recorded, possibly stopped too early or encountered immediate issue."
        )
    }
}

// --- reencode_video 함수 (동일) ---
fn reencode_video(input_path: &Path, output_path: &Path, fps: f64) -> Result<()> {
    // ... (이전 코드와 동일) ...
    let mut cap = VideoCapture::from_file(
        input_path.to_str().context("Invalid input path string")?,
        videoio::CAP_ANY,
    )
    .context("Failed to open temporary video file for re-encoding")?;
    if !cap.is_opened()? {
        anyhow::bail!("Could not open temporary video file: {:?}", input_path);
    }

    let width = cap.get(videoio::CAP_PROP_FRAME_WIDTH)? as i32;
    let height = cap.get(videoio::CAP_PROP_FRAME_HEIGHT)? as i32;
    let fourcc = VideoWriter::fourcc('X', 'V', 'I', 'D')?; // 코덱 확인 필요

    let mut writer = VideoWriter::new(
        output_path.to_str().context("Invalid output path string")?,
        fourcc,
        fps, // 전달받은 실제 FPS 사용
        Size::new(width, height),
        true,
    )
    .context("Failed to create final VideoWriter for re-encoding")?;
    if !writer.is_opened()? {
        anyhow::bail!("Could not open final video writer: {:?}", output_path);
    }

    let mut frame = Mat::default();
    let writer_guard = VideoWriterGuard(&mut writer); // 최종 writer guard
    loop {
        match cap.read(&mut frame) {
            Ok(true) => {
                if frame.empty() {
                    eprintln!("Warning: Read empty frame during re-encoding, skipping.");
                    continue; // Skip empty frames
                }
                writer_guard
                    .0
                    .write(&frame)
                    .context("Failed to write frame during re-encoding")?;
            }
            Ok(false) => {
                println!("End of video stream reached during re-encoding.");
                break; // 스트림 끝
            }
            Err(e) => return Err(e).context("Error reading frame during re-encoding"),
        }
    }
    drop(writer_guard); // 명시적 drop (선택사항, 스코프 벗어나면 자동 호출됨)
    println!("Re-encoding finished.");
    Ok(())
}

// --- VideoWriterGuard 구조체 및 Drop 구현 (동일) ---
struct VideoWriterGuard<'a>(&'a mut VideoWriter);
impl<'a> Drop for VideoWriterGuard<'a> {
    fn drop(&mut self) {
        if self.0.is_opened().unwrap_or(false) {
            println!("Releasing VideoWriter..."); // 해제 시 로그 추가
            if let Err(e) = self.0.release() {
                eprintln!("Error releasing VideoWriter: {}", e);
            } else {
                println!("VideoWriter released successfully.");
            }
        } else {
            println!("VideoWriter was already closed or not opened.");
        }
    }
}
