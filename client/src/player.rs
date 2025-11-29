use libloading::{Library, Symbol};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use std::{
    env,
    ffi::{c_char, c_float, c_int, c_uint, c_void, CStr, CString},
    mem,
    path::{Path, PathBuf},
    ptr,
    sync::Arc,
};

/// Wrapper around libVLC for video playback
pub struct VideoPlayer {
    instance: *mut libvlc_instance_t,
    media_player: *mut libvlc_media_player_t,
    current_file: Mutex<Option<String>>,
    frame_state: Arc<VideoFrameState>,
    callbacks_handle: *mut VideoFrameState,
}

unsafe impl Send for VideoPlayer {}
unsafe impl Sync for VideoPlayer {}

impl VideoPlayer {
    pub fn new(window_id: Option<i64>) -> Result<Self, String> {
        ensure_lib_loaded()?;
        let instance = unsafe { libvlc_new_instance()? };
        let media_player = unsafe { libvlc_media_player_new(instance)? };
        let frame_state = Arc::new(VideoFrameState::new());
        let callbacks_handle = Arc::into_raw(Arc::clone(&frame_state)) as *mut VideoFrameState;

        unsafe {
            if let Err(err) = install_video_callbacks(media_player, callbacks_handle) {
                drop(Arc::from_raw(callbacks_handle));
                libvlc_media_player_release(media_player);
                libvlc_release(instance);
                return Err(err);
            }
        }

        #[cfg(target_os = "windows")]
        if let Some(hwnd) = window_id {
            unsafe { libvlc_media_player_set_hwnd(media_player, hwnd as *mut c_void)? };
        }

        Ok(Self {
            instance,
            media_player,
            current_file: Mutex::new(None),
            callbacks_handle,
            frame_state,
        })
    }

    /// Load a video file
    pub fn load_file<P: AsRef<Path>>(&self, path: P) -> Result<(), String> {
        let path_str = path
            .as_ref()
            .to_str()
            .ok_or_else(|| "Invalid path encoding".to_string())?;
        let c_path =
            CString::new(path_str).map_err(|_| "Path contains embedded NUL".to_string())?;

        unsafe {
            let media = libvlc_media_new_path(self.instance, c_path.as_ptr())?;
            libvlc_media_player_set_media(self.media_player, media)?;
            libvlc_media_release(media);
        }

        *self.current_file.lock() = Some(path_str.to_string());
        Ok(())
    }

    /// Play the video
    pub fn play(&self) -> Result<(), String> {
        unsafe { libvlc_media_player_play(self.media_player) }
    }

    /// Pause the video
    pub fn pause(&self) -> Result<(), String> {
        unsafe { libvlc_media_player_set_pause(self.media_player, true) }
    }

    /// Stop playback
    pub fn stop(&self) -> Result<(), String> {
        unsafe { libvlc_media_player_stop(self.media_player) }
    }

    /// Seek to a specific timestamp (in seconds)
    pub fn seek(&self, timestamp: f64) -> Result<(), String> {
        unsafe { libvlc_media_player_set_time(self.media_player, (timestamp * 1000.0) as i64) }
    }

    /// Set playback speed
    pub fn set_speed(&self, speed: f64) -> Result<(), String> {
        unsafe { libvlc_media_player_set_rate(self.media_player, speed as c_float) }
    }

    /// Get current playback position (in seconds)
    pub fn get_position(&self) -> Result<f64, String> {
        unsafe {
            libvlc_media_player_get_time(self.media_player)
                .map(|ms| ms as f64 / 1000.0)
                .ok_or_else(|| "Position unavailable".to_string())
        }
    }

    /// Get video duration (in seconds)
    pub fn get_duration(&self) -> Result<f64, String> {
        unsafe {
            let len = libvlc_media_player_get_length(self.media_player);
            if len <= 0 {
                Err("Duration unavailable".to_string())
            } else {
                Ok(len as f64 / 1000.0)
            }
        }
    }

    /// Check if video is paused
    pub fn is_paused(&self) -> Result<bool, String> {
        unsafe { Ok(!libvlc_media_player_is_playing(self.media_player)) }
    }

    /// Get current playback speed
    pub fn get_speed(&self) -> Result<f64, String> {
        unsafe { Ok(libvlc_media_player_get_rate(self.media_player) as f64) }
    }

    /// Set volume (0-100)
    pub fn set_volume(&self, volume: f64) -> Result<(), String> {
        let clamped = volume.clamp(0.0, 100.0) as c_int;
        unsafe { libvlc_audio_set_volume(self.media_player, clamped) }
    }

    /// Get volume (0-100)
    pub fn get_volume(&self) -> Result<f64, String> {
        unsafe { libvlc_audio_get_volume(self.media_player).map(|v| v as f64) }
    }

    /// Get available audio tracks
    pub fn get_audio_tracks(&self) -> Result<Vec<AudioTrack>, String> {
        unsafe { enumerate_tracks(libvlc_audio_get_track_description, self.media_player) }
    }

    /// Set current audio track
    pub fn set_audio_track(&self, track_id: i64) -> Result<(), String> {
        unsafe { libvlc_audio_set_track(self.media_player, track_id as c_int) }
    }

    /// Get available subtitle tracks
    pub fn get_subtitle_tracks(&self) -> Result<Vec<SubtitleTrack>, String> {
        unsafe { enumerate_tracks(libvlc_video_get_spu_description, self.media_player) }.map(
            |tracks| {
                tracks
                    .into_iter()
                    .map(|t| SubtitleTrack {
                        id: t.id,
                        title: t.title.clone(),
                        lang: t.lang,
                    })
                    .collect()
            },
        )
    }

    /// Set current subtitle track (use -1 to disable)
    pub fn set_subtitle_track(&self, track_id: i64) -> Result<(), String> {
        unsafe { libvlc_video_set_spu(self.media_player, track_id as c_int) }
    }

    /// Frame step forward
    pub fn frame_step_forward(&self) -> Result<(), String> {
        unsafe { libvlc_media_player_next_frame(self.media_player) }
    }

    /// Frame step backward (approximate using a short reverse seek)
    pub fn frame_step_backward(&self) -> Result<(), String> {
        let current = unsafe { libvlc_media_player_get_time(self.media_player).unwrap_or(0) };
        let target = (current - 40).max(0);
        unsafe { libvlc_media_player_set_time(self.media_player, target) }
    }

    /// Retrieve latest RGB frame if available
    pub fn latest_frame(&self) -> Option<VideoFrame> {
        self.frame_state.grab_frame()
    }
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        unsafe {
            let _ = libvlc_media_player_stop(self.media_player);
            libvlc_media_player_release(self.media_player);
            libvlc_release(self.instance);
            uninstall_video_callbacks(self.media_player);
            if !self.callbacks_handle.is_null() {
                drop(Arc::from_raw(self.callbacks_handle));
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct AudioTrack {
    pub id: i64,
    pub title: String,
    pub lang: String,
}

#[derive(Debug, Clone)]
pub struct SubtitleTrack {
    pub id: i64,
    pub title: String,
    pub lang: String,
}

#[derive(Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub buffer: Vec<u8>,
}

struct VideoFrameState {
    buffers: Mutex<FrameBuffers>,
}

#[derive(Default)]
struct FrameBuffers {
    front: Vec<u8>,
    back: Vec<u8>,
    width: u32,
    height: u32,
    stride: usize,
    has_new_frame: bool,
}

impl VideoFrameState {
    fn new() -> Self {
        Self {
            buffers: Mutex::new(FrameBuffers::default()),
        }
    }

    fn configure(&self, width: u32, height: u32, stride: usize) {
        let mut buffers = self.buffers.lock();
        buffers.width = width;
        buffers.height = height;
        buffers.stride = stride;
        let required = buffers.required_len();
        if required == 0 {
            buffers.front.clear();
            buffers.back.clear();
            buffers.has_new_frame = false;
            return;
        }
        buffers.front.resize(required, 0);
        buffers.back.resize(required, 0);
        buffers.has_new_frame = false;
    }

    fn lock_plane(&self) -> *mut u8 {
        let mut buffers = self.buffers.lock();
        let required = buffers.required_len();
        if required == 0 {
            return ptr::null_mut();
        }
        if buffers.back.len() != required {
            buffers.back.resize(required, 0);
        }
        buffers.back.as_mut_ptr()
    }

    fn present(&self) {
        let mut buffers = self.buffers.lock();
        if buffers.width == 0 || buffers.height == 0 || buffers.back.is_empty() {
            return;
        }
        let new_front = mem::take(&mut buffers.back);
        let old_front = mem::replace(&mut buffers.front, new_front);
        buffers.back = old_front;
        buffers.has_new_frame = true;
    }

    fn grab_frame(&self) -> Option<VideoFrame> {
        let mut buffers = self.buffers.lock();
        if !buffers.has_new_frame || buffers.width == 0 || buffers.height == 0 {
            return None;
        }
        let data = buffers.front.clone();
        buffers.has_new_frame = false;
        Some(VideoFrame {
            width: buffers.width,
            height: buffers.height,
            buffer: data,
        })
    }
}

impl FrameBuffers {
    fn required_len(&self) -> usize {
        self.stride.saturating_mul(self.height as usize)
    }
}

unsafe fn install_video_callbacks(
    player: *mut libvlc_media_player_t,
    state_ptr: *mut VideoFrameState,
) -> Result<(), String> {
    libvlc_video_set_callbacks(
        player,
        Some(video_lock),
        Some(video_unlock),
        Some(video_display),
        state_ptr as *mut c_void,
    )?;
    libvlc_video_set_format_callbacks(
        player,
        Some(video_format_setup),
        Some(video_format_cleanup),
    )?;
    Ok(())
}

unsafe fn uninstall_video_callbacks(player: *mut libvlc_media_player_t) {
    let _ = libvlc_video_set_callbacks(player, None, None, None, ptr::null_mut());
    let _ = libvlc_video_set_format_callbacks(player, None, None);
}

unsafe extern "C" fn video_lock(opaque: *mut c_void, planes: *mut *mut c_void) -> *mut c_void {
    if opaque.is_null() || planes.is_null() {
        return ptr::null_mut();
    }
    let state = (opaque as *mut VideoFrameState).as_ref();
    let Some(state) = state else {
        return ptr::null_mut();
    };
    let plane_ptr = state.lock_plane();
    if plane_ptr.is_null() {
        return ptr::null_mut();
    }
    *planes = plane_ptr as *mut c_void;
    ptr::null_mut()
}

unsafe extern "C" fn video_unlock(
    _opaque: *mut c_void,
    _picture: *mut c_void,
    _planes: *mut *mut c_void,
) {
    // No-op: we swap buffers during the display callback.
}

unsafe extern "C" fn video_display(opaque: *mut c_void, _picture: *mut c_void) {
    if opaque.is_null() {
        return;
    }
    if let Some(state) = (opaque as *mut VideoFrameState).as_ref() {
        state.present();
    }
}

unsafe extern "C" fn video_format_setup(
    opaque: *mut *mut c_void,
    chroma: *mut c_char,
    width: *mut c_uint,
    height: *mut c_uint,
    pitches: *mut c_uint,
    lines: *mut c_uint,
) -> c_uint {
    if opaque.is_null()
        || width.is_null()
        || height.is_null()
        || pitches.is_null()
        || lines.is_null()
    {
        return 0;
    }

    let state_ptr = *opaque as *mut VideoFrameState;
    let Some(state) = state_ptr.as_ref() else {
        return 0;
    };

    unsafe {
        *opaque = state_ptr as *mut c_void;
    }

    let w = unsafe { *width } as u32;
    let h = unsafe { *height } as u32;
    if w == 0 || h == 0 {
        return 0;
    }

    let stride = (w as usize).saturating_mul(4);
    unsafe {
        *pitches = stride as c_uint;
        *lines = h as c_uint;
        if !chroma.is_null() {
            let chroma_bytes = b"RV32";
            ptr::copy_nonoverlapping(
                chroma_bytes.as_ptr() as *const c_char,
                chroma,
                chroma_bytes.len(),
            );
        }
    }

    state.configure(w, h, stride);
    1
}

unsafe extern "C" fn video_format_cleanup(_opaque: *mut c_void) {}

// --- libVLC dynamic bindings -------------------------------------------------

type TrackListFn = unsafe fn(*mut libvlc_media_player_t) -> *mut libvlc_track_description_t;

unsafe fn enumerate_tracks(
    getter: TrackListFn,
    player: *mut libvlc_media_player_t,
) -> Result<Vec<AudioTrack>, String> {
    let mut tracks = Vec::new();
    let list = getter(player);
    if list.is_null() {
        return Ok(tracks);
    }

    let mut node = list;
    while !node.is_null() {
        let name = cstr_to_string((*node).psz_name)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("Track {}", tracks.len() + 1));
        tracks.push(AudioTrack {
            id: (*node).i_id as i64,
            title: name.clone(),
            lang: name,
        });
        node = (*node).p_next;
    }

    libvlc_track_description_list_release(list);
    Ok(tracks)
}

fn cstr_to_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        None
    } else {
        unsafe { Some(CStr::from_ptr(ptr).to_string_lossy().into_owned()) }
    }
}

static LIBVLC: OnceCell<&'static Library> = OnceCell::new();

fn ensure_lib_loaded() -> Result<(), String> {
    libvlc_library().map(|_| ())
}

fn libvlc_library() -> Result<&'static Library, String> {
    LIBVLC
        .get_or_try_init(|| {
            let lib = unsafe { load_library()? };
            Ok(Box::leak(Box::new(lib)))
        })
        .map(|lib| *lib)
}

unsafe fn load_library() -> Result<Library, String> {
    if let Ok(path) = env::var("LIBVLC_PATH") {
        return Library::new(&path)
            .map_err(|e| format!("Failed to load libVLC from {}: {e}", path));
    }

    let mut errors = Vec::new();
    for candidate in default_candidates() {
        match Library::new(&candidate) {
            Ok(lib) => return Ok(lib),
            Err(err) => errors.push(format!("{}: {err}", candidate.display())),
        }
    }

    Err(format!(
        "Unable to locate libVLC. Set LIBVLC_PATH or install VLC. Tried:\n{}",
        errors.join("\n")
    ))
}

fn default_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    #[cfg(target_os = "windows")]
    {
        paths.push(PathBuf::from("libvlc.dll"));
        paths.push(PathBuf::from("vlc\\libvlc.dll"));
        if let Some(pf) = env::var_os("ProgramFiles") {
            paths.push(PathBuf::from(pf).join("VideoLAN\\VLC\\libvlc.dll"));
        }
        if let Some(pf86) = env::var_os("ProgramFiles(x86)") {
            paths.push(PathBuf::from(pf86).join("VideoLAN\\VLC\\libvlc.dll"));
        }
    }

    #[cfg(target_os = "linux")]
    {
        paths.push(PathBuf::from("libvlc.so"));
        paths.push(PathBuf::from("libvlc.so.5"));
    }

    #[cfg(target_os = "macos")]
    {
        paths.push(PathBuf::from("libvlc.dylib"));
        paths.push(PathBuf::from(
            "/Applications/VLC.app/Contents/MacOS/lib/libvlc.dylib",
        ));
    }

    paths
}

fn symbol_name(bytes: &[u8]) -> &str {
    std::str::from_utf8(&bytes[..bytes.len() - 1]).unwrap_or("<invalid>")
}

unsafe fn get_symbol<T>(name: &'static [u8]) -> Result<Symbol<'static, T>, String> {
    libvlc_library()?
        .get(name)
        .map_err(|e| format!("Failed to load symbol {}: {e}", symbol_name(name)))
}

unsafe fn libvlc_new_instance() -> Result<*mut libvlc_instance_t, String> {
    let sym: Symbol<unsafe extern "C" fn(c_int, *const *const c_char) -> *mut libvlc_instance_t> =
        get_symbol(b"libvlc_new\0")?;
    let ptr = sym(0, ptr::null());
    if ptr.is_null() {
        Err(format_error("libvlc_new"))
    } else {
        Ok(ptr)
    }
}

unsafe fn libvlc_release(instance: *mut libvlc_instance_t) {
    if let Ok(sym) = get_symbol::<unsafe extern "C" fn(*mut libvlc_instance_t)>(b"libvlc_release\0")
    {
        sym(instance);
    }
}

unsafe fn libvlc_media_player_new(
    instance: *mut libvlc_instance_t,
) -> Result<*mut libvlc_media_player_t, String> {
    let sym: Symbol<unsafe extern "C" fn(*mut libvlc_instance_t) -> *mut libvlc_media_player_t> =
        get_symbol(b"libvlc_media_player_new\0")?;
    let ptr = sym(instance);
    if ptr.is_null() {
        Err(format_error("libvlc_media_player_new"))
    } else {
        Ok(ptr)
    }
}

unsafe fn libvlc_media_player_release(player: *mut libvlc_media_player_t) {
    if let Ok(sym) = get_symbol::<unsafe extern "C" fn(*mut libvlc_media_player_t)>(
        b"libvlc_media_player_release\0",
    ) {
        sym(player);
    }
}

unsafe fn libvlc_media_new_path(
    instance: *mut libvlc_instance_t,
    path: *const c_char,
) -> Result<*mut libvlc_media_t, String> {
    let sym: Symbol<
        unsafe extern "C" fn(*mut libvlc_instance_t, *const c_char) -> *mut libvlc_media_t,
    > = get_symbol(b"libvlc_media_new_path\0")?;
    let media = sym(instance, path);
    if media.is_null() {
        Err(format_error("libvlc_media_new_path"))
    } else {
        Ok(media)
    }
}

unsafe fn libvlc_media_release(media: *mut libvlc_media_t) {
    if let Ok(sym) =
        get_symbol::<unsafe extern "C" fn(*mut libvlc_media_t)>(b"libvlc_media_release\0")
    {
        sym(media);
    }
}

unsafe fn libvlc_media_player_set_media(
    player: *mut libvlc_media_player_t,
    media: *mut libvlc_media_t,
) -> Result<(), String> {
    let sym: Symbol<unsafe extern "C" fn(*mut libvlc_media_player_t, *mut libvlc_media_t)> =
        get_symbol(b"libvlc_media_player_set_media\0")?;
    sym(player, media);
    Ok(())
}

unsafe fn libvlc_media_player_play(player: *mut libvlc_media_player_t) -> Result<(), String> {
    let sym: Symbol<unsafe extern "C" fn(*mut libvlc_media_player_t) -> c_int> =
        get_symbol(b"libvlc_media_player_play\0")?;
    if sym(player) == 0 {
        Ok(())
    } else {
        Err(format_error("Failed to start playback"))
    }
}

unsafe fn libvlc_media_player_set_pause(
    player: *mut libvlc_media_player_t,
    paused: bool,
) -> Result<(), String> {
    let sym: Symbol<unsafe extern "C" fn(*mut libvlc_media_player_t, c_int)> =
        get_symbol(b"libvlc_media_player_set_pause\0")?;
    sym(player, if paused { 1 } else { 0 });
    Ok(())
}

unsafe fn libvlc_media_player_stop(player: *mut libvlc_media_player_t) -> Result<(), String> {
    let sym: Symbol<unsafe extern "C" fn(*mut libvlc_media_player_t)> =
        get_symbol(b"libvlc_media_player_stop\0")?;
    sym(player);
    Ok(())
}

unsafe fn libvlc_media_player_set_time(
    player: *mut libvlc_media_player_t,
    time_ms: i64,
) -> Result<(), String> {
    let sym: Symbol<unsafe extern "C" fn(*mut libvlc_media_player_t, i64)> =
        get_symbol(b"libvlc_media_player_set_time\0")?;
    sym(player, time_ms);
    Ok(())
}

unsafe fn libvlc_media_player_get_time(player: *mut libvlc_media_player_t) -> Option<i64> {
    let sym: Symbol<unsafe extern "C" fn(*mut libvlc_media_player_t) -> i64> =
        get_symbol(b"libvlc_media_player_get_time\0").ok()?;
    let value = sym(player);
    if value < 0 {
        None
    } else {
        Some(value)
    }
}

unsafe fn libvlc_media_player_get_length(player: *mut libvlc_media_player_t) -> i64 {
    let sym: Symbol<unsafe extern "C" fn(*mut libvlc_media_player_t) -> i64> =
        get_symbol(b"libvlc_media_player_get_length\0").unwrap();
    sym(player)
}

unsafe fn libvlc_media_player_is_playing(player: *mut libvlc_media_player_t) -> bool {
    let sym: Symbol<unsafe extern "C" fn(*mut libvlc_media_player_t) -> c_int> =
        get_symbol(b"libvlc_media_player_is_playing\0").unwrap();
    sym(player) != 0
}

unsafe fn libvlc_media_player_get_rate(player: *mut libvlc_media_player_t) -> c_float {
    let sym: Symbol<unsafe extern "C" fn(*mut libvlc_media_player_t) -> c_float> =
        get_symbol(b"libvlc_media_player_get_rate\0").unwrap();
    sym(player)
}

unsafe fn libvlc_media_player_set_rate(
    player: *mut libvlc_media_player_t,
    rate: c_float,
) -> Result<(), String> {
    let sym: Symbol<unsafe extern "C" fn(*mut libvlc_media_player_t, c_float) -> c_int> =
        get_symbol(b"libvlc_media_player_set_rate\0")?;
    if sym(player, rate) == 0 {
        Ok(())
    } else {
        Err(format_error("Failed to set playback rate"))
    }
}

unsafe fn libvlc_audio_get_volume(player: *mut libvlc_media_player_t) -> Result<c_int, String> {
    let sym: Symbol<unsafe extern "C" fn(*mut libvlc_media_player_t) -> c_int> =
        get_symbol(b"libvlc_audio_get_volume\0")?;
    let volume = sym(player);
    if volume < 0 {
        Err(format_error("Failed to read volume"))
    } else {
        Ok(volume)
    }
}

unsafe fn libvlc_audio_set_volume(
    player: *mut libvlc_media_player_t,
    volume: c_int,
) -> Result<(), String> {
    let sym: Symbol<unsafe extern "C" fn(*mut libvlc_media_player_t, c_int) -> c_int> =
        get_symbol(b"libvlc_audio_set_volume\0")?;
    if sym(player, volume) == 0 {
        Ok(())
    } else {
        Err(format_error("Failed to set volume"))
    }
}

unsafe fn libvlc_audio_get_track_description(
    player: *mut libvlc_media_player_t,
) -> *mut libvlc_track_description_t {
    let sym: Symbol<
        unsafe extern "C" fn(*mut libvlc_media_player_t) -> *mut libvlc_track_description_t,
    > = get_symbol(b"libvlc_audio_get_track_description\0").unwrap();
    sym(player)
}

unsafe fn libvlc_audio_set_track(
    player: *mut libvlc_media_player_t,
    id: c_int,
) -> Result<(), String> {
    let sym: Symbol<unsafe extern "C" fn(*mut libvlc_media_player_t, c_int) -> c_int> =
        get_symbol(b"libvlc_audio_set_track\0")?;
    if sym(player, id) == 0 {
        Ok(())
    } else {
        Err(format_error("Failed to set audio track"))
    }
}

unsafe fn libvlc_video_get_spu_description(
    player: *mut libvlc_media_player_t,
) -> *mut libvlc_track_description_t {
    let sym: Symbol<
        unsafe extern "C" fn(*mut libvlc_media_player_t) -> *mut libvlc_track_description_t,
    > = get_symbol(b"libvlc_video_get_spu_description\0").unwrap();
    sym(player)
}

unsafe fn libvlc_video_set_spu(
    player: *mut libvlc_media_player_t,
    id: c_int,
) -> Result<(), String> {
    let sym: Symbol<unsafe extern "C" fn(*mut libvlc_media_player_t, c_int) -> c_int> =
        get_symbol(b"libvlc_video_set_spu\0")?;
    if sym(player, id) == 0 {
        Ok(())
    } else {
        Err(format_error("Failed to set subtitle track"))
    }
}

unsafe fn libvlc_track_description_list_release(list: *mut libvlc_track_description_t) {
    if let Ok(sym) = get_symbol::<unsafe extern "C" fn(*mut libvlc_track_description_t)>(
        b"libvlc_track_description_list_release\0",
    ) {
        sym(list);
    }
}

unsafe fn libvlc_media_player_next_frame(player: *mut libvlc_media_player_t) -> Result<(), String> {
    let sym: Symbol<unsafe extern "C" fn(*mut libvlc_media_player_t)> =
        get_symbol(b"libvlc_media_player_next_frame\0")?;
    sym(player);
    Ok(())
}

#[cfg(target_os = "windows")]
unsafe fn libvlc_media_player_set_hwnd(
    player: *mut libvlc_media_player_t,
    hwnd: *mut c_void,
) -> Result<(), String> {
    let sym: Symbol<unsafe extern "C" fn(*mut libvlc_media_player_t, *mut c_void)> =
        get_symbol(b"libvlc_media_player_set_hwnd\0")?;
    sym(player, hwnd);
    Ok(())
}

type VideoLockCallback = unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> *mut c_void;
type VideoUnlockCallback = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut *mut c_void);
type VideoDisplayCallback = unsafe extern "C" fn(*mut c_void, *mut c_void);
type VideoFormatCallback = unsafe extern "C" fn(
    *mut *mut c_void,
    *mut c_char,
    *mut c_uint,
    *mut c_uint,
    *mut c_uint,
    *mut c_uint,
) -> c_uint;
type VideoCleanupCallback = unsafe extern "C" fn(*mut c_void);

unsafe fn libvlc_video_set_callbacks(
    player: *mut libvlc_media_player_t,
    lock_cb: Option<VideoLockCallback>,
    unlock_cb: Option<VideoUnlockCallback>,
    display_cb: Option<VideoDisplayCallback>,
    opaque: *mut c_void,
) -> Result<(), String> {
    let sym: Symbol<
        unsafe extern "C" fn(
            *mut libvlc_media_player_t,
            Option<VideoLockCallback>,
            Option<VideoUnlockCallback>,
            Option<VideoDisplayCallback>,
            *mut c_void,
        ),
    > = get_symbol(b"libvlc_video_set_callbacks\0")?;
    sym(player, lock_cb, unlock_cb, display_cb, opaque);
    Ok(())
}

unsafe fn libvlc_video_set_format_callbacks(
    player: *mut libvlc_media_player_t,
    setup_cb: Option<VideoFormatCallback>,
    cleanup_cb: Option<VideoCleanupCallback>,
) -> Result<(), String> {
    let sym: Symbol<
        unsafe extern "C" fn(
            *mut libvlc_media_player_t,
            Option<VideoFormatCallback>,
            Option<VideoCleanupCallback>,
        ),
    > = get_symbol(b"libvlc_video_set_format_callbacks\0")?;
    sym(player, setup_cb, cleanup_cb);
    Ok(())
}

fn format_error(action: &str) -> String {
    unsafe {
        if let Ok(sym) = get_symbol::<unsafe extern "C" fn() -> *const c_char>(b"libvlc_errmsg\0") {
            let ptr = sym();
            if !ptr.is_null() {
                let msg = CStr::from_ptr(ptr).to_string_lossy().into_owned();
                if !msg.is_empty() {
                    return format!("{action}: {msg}");
                }
            }
        }
    }
    action.to_string()
}

#[repr(C)]
struct libvlc_instance_t {
    _private: [u8; 0],
}

#[repr(C)]
struct libvlc_media_t {
    _private: [u8; 0],
}

#[repr(C)]
struct libvlc_media_player_t {
    _private: [u8; 0],
}

#[repr(C)]
struct libvlc_track_description_t {
    i_id: c_int,
    psz_name: *mut c_char,
    p_next: *mut libvlc_track_description_t,
}
