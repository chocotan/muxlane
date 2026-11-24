//! Sound notifications and desktop alerts for agent state changes.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, OnceLock};

static SOUND_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);
static SOUND_DONE: &[u8] = include_bytes!("../assets/sounds/done.mp3");
static SOUND_REQUEST: &[u8] = include_bytes!("../assets/sounds/request.mp3");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundKind {
    Done,
    Request,
}

#[derive(Debug)]
enum NotificationRequest {
    Play(SoundKind),
    Desktop { title: String, body: String },
}

fn notification_sender() -> &'static mpsc::Sender<NotificationRequest> {
    static SENDER: OnceLock<mpsc::Sender<NotificationRequest>> = OnceLock::new();
    SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(request) = rx.recv() {
                match request {
                    NotificationRequest::Play(sound) => play_sound_now(sound),
                    NotificationRequest::Desktop { title, body } => {
                        let _ = notify_rust::Notification::new()
                            .appname("Muxlane")
                            .summary(&title)
                            .body(&body)
                            .show();
                    }
                }
            }
        });
        tx
    })
}

pub fn play_sound(sound: SoundKind) {
    if cfg!(test)
        || std::env::var_os("MUXLANE_DISABLE_SOUND").is_some()
        || std::env::var_os("NEXTEST").is_some()
    {
        return;
    }
    let _ = notification_sender().send(NotificationRequest::Play(sound));
}

fn play_sound_now(sound: SoundKind) {
    let data = match sound {
        SoundKind::Done => SOUND_DONE,
        SoundKind::Request => SOUND_REQUEST,
    };

    let tmp = temp_sound_path();
    if let Ok(mut file) = std::fs::File::create(&tmp) {
        if file.write_all(data).is_ok() {
            drop(file);
            let _ = run_player(&tmp);
        }
        let _ = std::fs::remove_file(&tmp);
    }
}

fn temp_sound_path() -> PathBuf {
    let count = SOUND_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    std::env::temp_dir().join(format!("muxlane-sound-{pid}-{count}.mp3"))
}

fn run_player(path: &Path) -> Result<Output, String> {
    #[cfg(target_os = "macos")]
    {
        return Command::new("afplay")
            .arg(path)
            .output()
            .map_err(|e| e.to_string());
    }

    #[cfg(not(target_os = "macos"))]
    {
        // Try common linux players in order
        let players = [
            ("paplay", vec![]),
            ("pw-play", vec![]),
            ("aplay", vec![]),
            ("mpv", vec!["--no-terminal", "--really-quiet"]),
            ("ffplay", vec!["-nodisp", "-autoexit", "-loglevel", "quiet"]),
        ];

        for (player, extra_args) in players {
            let mut cmd = Command::new(player);
            for arg in extra_args {
                cmd.arg(arg);
            }
            cmd.arg(path);
            if let Ok(output) = cmd.output() {
                if output.status.success() {
                    return Ok(output);
                }
            }
        }
        Err("No audio player found".to_string())
    }
}

pub fn send_desktop_notification(title: &str, body: &str) {
    if std::env::var_os("MUXLANE_DISABLE_NOTIFY").is_some() || std::env::var_os("NEXTEST").is_some()
    {
        return;
    }

    let _ = notification_sender().send(NotificationRequest::Desktop {
        title: title.to_string(),
        body: body.to_string(),
    });
}
