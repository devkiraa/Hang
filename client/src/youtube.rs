use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

// Windows constant to hide console window
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Video quality options
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoQuality {
    Best,
    Quality4K,
    Quality1440p,
    Quality1080p,
    Quality720p,
    Quality480p,
    Quality360p,
    AudioOnly,
}

impl VideoQuality {
    pub fn as_str(&self) -> &'static str {
        match self {
            VideoQuality::Best => "Best Available",
            VideoQuality::Quality4K => "4K (2160p)",
            VideoQuality::Quality1440p => "1440p (2K)",
            VideoQuality::Quality1080p => "1080p (Full HD)",
            VideoQuality::Quality720p => "720p (HD)",
            VideoQuality::Quality480p => "480p",
            VideoQuality::Quality360p => "360p",
            VideoQuality::AudioOnly => "Audio Only",
        }
    }

    pub fn all() -> &'static [VideoQuality] {
        &[
            VideoQuality::Best,
            VideoQuality::Quality4K,
            VideoQuality::Quality1440p,
            VideoQuality::Quality1080p,
            VideoQuality::Quality720p,
            VideoQuality::Quality480p,
            VideoQuality::Quality360p,
            VideoQuality::AudioOnly,
        ]
    }

    fn to_format_string(&self) -> &'static str {
        // Use formats that have both video AND audio in a single stream
        // b = best format with both video and audio
        // The fallback formats ensure we get combined streams that VLC can play
        match self {
            VideoQuality::Best => "b",
            VideoQuality::Quality4K => "b[height<=2160]",
            VideoQuality::Quality1440p => "b[height<=1440]",
            VideoQuality::Quality1080p => "b[height<=1080]",
            VideoQuality::Quality720p => "b[height<=720]",
            VideoQuality::Quality480p => "b[height<=480]",
            VideoQuality::Quality360p => "b[height<=360]",
            VideoQuality::AudioOnly => "ba/b",
        }
    }
}

impl Default for VideoQuality {
    fn default() -> Self {
        VideoQuality::Best
    }
}

/// Information about a YouTube video
#[derive(Debug, Clone)]
pub struct YouTubeVideo {
    pub title: String,
    pub stream_url: String,
    pub quality: VideoQuality,
}

/// Result of async YouTube loading
#[derive(Debug)]
pub enum YouTubeLoadResult {
    Success(YouTubeVideo),
    Error(String),
    Downloading, // yt-dlp is being downloaded
}

/// Check if a URL is a YouTube URL
pub fn is_youtube_url(url: &str) -> bool {
    url.contains("youtube.com/watch")
        || url.contains("youtu.be/")
        || url.contains("youtube.com/shorts/")
        || url.contains("youtube.com/live/")
}

/// Extract video ID from YouTube URL
pub fn extract_video_id(url: &str) -> Option<String> {
    // Handle youtu.be/VIDEO_ID
    if url.contains("youtu.be/") {
        return url
            .split("youtu.be/")
            .nth(1)
            .map(|s| s.split(&['?', '&', '/'][..]).next().unwrap_or(s).to_string());
    }

    // Handle youtube.com/watch?v=VIDEO_ID
    if url.contains("youtube.com/watch") {
        if let Some(query) = url.split('?').nth(1) {
            for param in query.split('&') {
                if param.starts_with("v=") {
                    return Some(param[2..].to_string());
                }
            }
        }
    }

    // Handle youtube.com/shorts/VIDEO_ID or youtube.com/live/VIDEO_ID
    if url.contains("/shorts/") || url.contains("/live/") {
        return url
            .split(&['/'][..])
            .last()
            .map(|s| s.split(&['?', '&'][..]).next().unwrap_or(s).to_string());
    }

    None
}

/// Get the path where yt-dlp should be stored (next to the executable)
fn get_ytdlp_path() -> PathBuf {
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            return exe_dir.join("yt-dlp.exe");
        }
    }
    PathBuf::from("yt-dlp.exe")
}

/// Check if yt-dlp is available
pub fn is_ytdlp_available() -> bool {
    get_ytdlp_path().exists() || Command::new("yt-dlp").arg("--version").output().is_ok()
}

/// Download yt-dlp if not present - returns the path to the executable
fn ensure_ytdlp() -> Result<PathBuf> {
    let ytdlp_path = get_ytdlp_path();

    // Check if already exists next to exe
    if ytdlp_path.exists() {
        return Ok(ytdlp_path);
    }

    // Check if in PATH
    if Command::new("yt-dlp").arg("--version").output().is_ok() {
        return Ok(PathBuf::from("yt-dlp"));
    }

    // Download yt-dlp automatically using PowerShell (works on Windows)
    let url = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";
    let dest = ytdlp_path.to_string_lossy();

    // Use PowerShell to download (hidden window)
    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-WindowStyle", "Hidden",
        "-Command",
        &format!(
            "Invoke-WebRequest -Uri '{}' -OutFile '{}' -UseBasicParsing",
            url, dest
        ),
    ]);
    
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    
    let status = cmd.status()
        .context("Failed to run PowerShell for download")?;

    if !status.success() {
        anyhow::bail!("Failed to download yt-dlp");
    }

    if !ytdlp_path.exists() {
        anyhow::bail!("Download completed but yt-dlp.exe not found");
    }

    Ok(ytdlp_path)
}

/// Get stream URL using yt-dlp (blocking - call from background thread)
pub fn get_stream_url(youtube_url: &str, quality: VideoQuality) -> Result<YouTubeVideo> {
    let ytdlp_path = ensure_ytdlp()?;

    // Get the stream URL with specified quality (hidden window)
    let mut cmd = Command::new(&ytdlp_path);
    cmd.args([
        "--no-warnings",
        "--no-playlist",
        "-f",
        quality.to_format_string(),
        "--get-url",
        "--get-title",
        youtube_url,
    ]);
    
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    
    let output = cmd.output()
        .context("Failed to execute yt-dlp")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("yt-dlp failed: {}", stderr.trim());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.trim().lines().collect();

    if lines.is_empty() {
        anyhow::bail!("No output from yt-dlp");
    }

    let title = lines[0].to_string();
    // yt-dlp may output multiple URLs for video+audio, we take the first one
    let stream_url = if lines.len() > 1 {
        lines[1].to_string()
    } else {
        anyhow::bail!("No stream URL returned");
    };

    Ok(YouTubeVideo {
        title,
        stream_url,
        quality,
    })
}

/// Async YouTube loader - spawns a thread and returns a receiver
pub struct YouTubeLoader {
    receiver: mpsc::Receiver<YouTubeLoadResult>,
}

impl YouTubeLoader {
    /// Start loading a YouTube video in the background
    pub fn start(url: String, quality: VideoQuality) -> Self {
        let (sender, receiver) = mpsc::channel();

        std::thread::spawn(move || {
            // Check if we need to download yt-dlp first
            if !is_ytdlp_available() {
                let _ = sender.send(YouTubeLoadResult::Downloading);
            }

            match get_stream_url(&url, quality) {
                Ok(video) => {
                    let _ = sender.send(YouTubeLoadResult::Success(video));
                }
                Err(e) => {
                    let _ = sender.send(YouTubeLoadResult::Error(e.to_string()));
                }
            }
        });

        Self { receiver }
    }

    /// Check if result is ready (non-blocking)
    pub fn try_recv(&self) -> Option<YouTubeLoadResult> {
        self.receiver.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_youtube_url() {
        assert!(is_youtube_url("https://www.youtube.com/watch?v=dQw4w9WgXcQ"));
        assert!(is_youtube_url("https://youtu.be/dQw4w9WgXcQ"));
        assert!(is_youtube_url("https://youtube.com/shorts/abc123"));
        assert!(!is_youtube_url("https://example.com/video.mp4"));
    }

    #[test]
    fn test_extract_video_id() {
        assert_eq!(
            extract_video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".to_string())
        );
        assert_eq!(
            extract_video_id("https://youtu.be/dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".to_string())
        );
        assert_eq!(
            extract_video_id("https://youtube.com/shorts/abc123"),
            Some("abc123".to_string())
        );
    }
}
