//! Orbit Island: standalone notification-engine for the Orbit Island pill.
//!
//! This crate is the reference implementation of the Island's *behavior* — a pure
//! state machine over notifications, priorities, a focus timer and expand/collapse
//! state. It contains no UI and no platform code; the Orbit host renders the pill.
//!
//! The embedded copy inside Orbit (`orbit-core::island`) intentionally mirrors this
//! engine until the module-IPC milestone lands; both are covered by equivalent tests.
//!
//! Also defines `IslandMessage`, the wire envelope of the Orbit Module Protocol v1
//! (see SPEC.md). Kept here so the protocol has a compilable, tested definition.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;
use thiserror::Error;

pub const MAX_QUEUE: usize = 16;
pub const MAX_TOAST_CHARS: usize = 140;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum IslandError {
    #[error("toast text must be 1..={MAX_TOAST_CHARS} characters, got {0}")]
    BadToastLength(usize),
    #[error("timer duration must be between 1 second and 8 hours")]
    BadTimerDuration,
    #[error("notification id {0} not found")]
    NotificationNotFound(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    Normal,
    High,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IslandContent {
    Toast { text: String },
    ClipboardStored { preview: String },
    ScreenshotCaptured { name: String },
    FileTransfer { label: String, bytes_done: u64, bytes_total: u64 },
    WorkspaceSwitched { name: String },
    SystemLoad { cpu_percent: f64, ram_percent: f64 },
    TimerDone { label: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IslandNotification {
    pub id: u64,
    pub priority: Priority,
    pub content: IslandContent,
    pub created_at_ms: u64,
    /// Milliseconds after creation when the notification may auto-dismiss (None = sticky).
    pub auto_dismiss_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FocusTimer {
    pub label: String,
    pub duration_ms: u64,
    pub started_at_ms: u64,
    /// Remaining ms when paused; None while running.
    pub paused_remaining_ms: Option<u64>,
}

impl FocusTimer {
    fn remaining_ms(&self, now_ms: u64) -> u64 {
        match self.paused_remaining_ms {
            Some(r) => r,
            None => self.duration_ms.saturating_sub(now_ms.saturating_sub(self.started_at_ms)),
        }
    }

    pub fn is_finished(&self, now_ms: u64) -> bool {
        self.paused_remaining_ms.is_none() && self.remaining_ms(now_ms) == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IslandView {
    Collapsed,
    Expanded,
}

/// Currently-playing media reported by the host (e.g. MPRIS over D-Bus on KDE/Linux).
/// The island never invents it; the host pushes and clears it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NowPlaying {
    pub player: String,
    pub title: String,
    pub artist: String,
    pub position_secs: u64,
    pub length_secs: Option<u64>,
    pub playing: bool,
    /// Media volume 0.0–1.0 when the player exposes it (MPRIS `Volume`); `None` otherwise — never guessed.
    pub volume: Option<f32>,
}

/// Coarse machine load for the island. Sampled by the host from local sources
/// (procfs on Linux); never leaves the device.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SystemStats {
    pub cpu_percent: f32,
    pub ram_percent: f32,
    pub ram_used_mb: u64,
    pub ram_total_mb: u64,
}

/// One in-flight or finished download/transfer shown on the island.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DownloadTask {
    /// Stable host-chosen key.
    pub id: String,
    pub label: String,
    pub received_bytes: u64,
    /// `None` when the total is unknowable — never guessed.
    pub total_bytes: Option<u64>,
    pub done: bool,
}

/// A file the user dropped onto the island. Listing only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DroppedFile {
    pub path: String,
    pub bytes: u64,
}

/// How many completed download tasks to keep for the "recent" list.
pub const MAX_KEEP_DONE_DOWNLOADS: usize = 4;
/// Upper bound on tracked in-flight downloads.
pub const MAX_TRACK_DOWNLOADS: usize = 8;
/// Upper bound on remembered dropped files.
pub const MAX_DROPPED_FILES: usize = 16;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IslandSnapshot {
    pub active: Option<IslandNotification>,
    pub queued: usize,
    pub view: IslandView,
    pub timer: Option<FocusTimer>,
    pub now_playing: Option<NowPlaying>,
    pub downloads: Vec<DownloadTask>,
    pub downloads_active: usize,
    pub dropped_files: Vec<DroppedFile>,
    /// Coarse local CPU/RAM load; `None` when the sampler is off or unsupported on this platform.
    pub system: Option<SystemStats>,
}

#[derive(Debug, Clone)]
pub struct IslandManager {
    active: Option<IslandNotification>,
    queue: VecDeque<IslandNotification>,
    next_id: u64,
    view: IslandView,
    timer: Option<FocusTimer>,
    now_playing: Option<NowPlaying>,
    downloads: Vec<DownloadTask>,
    dropped_files: Vec<DroppedFile>,
    system: Option<SystemStats>,
}

impl Default for IslandManager {
    fn default() -> Self {
        Self::new()
    }
}

impl IslandManager {
    pub fn new() -> Self {
        Self {
            active: None,
            queue: VecDeque::new(),
            next_id: 1,
            view: IslandView::Collapsed,
            timer: None,
            now_playing: None,
            downloads: Vec::new(),
            dropped_files: Vec::new(),
            system: None,
        }
    }

    /// Build a toast notification (validates length); other content types pass through.
    pub fn toast(&mut self, text: impl Into<String>, priority: Priority, now_ms: u64) -> Result<IslandNotification, IslandError> {
        let text = text.into();
        let chars = text.chars().count();
        if chars == 0 || chars > MAX_TOAST_CHARS {
            return Err(IslandError::BadToastLength(chars));
        }
        Ok(self.push_content(IslandContent::Toast { text }, priority, now_ms, Some(6_000)))
    }

    /// Push a clipboard-stored notification (priority Normal, auto-dismiss 6s).
    /// The preview is validated like a toast (1..=MAX_TOAST_CHARS). Returns the id.
    pub fn notify_clipboard(&mut self, preview: String, at_ms: u64) -> Result<u64, IslandError> {
        Ok(self.push_clipboard_helper(preview, Priority::Normal, at_ms)?.id)
    }

    /// Push a file-transfer progress notification (priority Normal, auto-dismiss 6s).
    /// The label is validated like a toast (1..=MAX_TOAST_CHARS). Returns the id.
    pub fn notify_file_transfer(
        &mut self,
        label: String,
        bytes_done: u64,
        bytes_total: u64,
        at_ms: u64,
    ) -> Result<u64, IslandError> {
        let chars = label.chars().count();
        if chars == 0 || chars > MAX_TOAST_CHARS {
            return Err(IslandError::BadToastLength(chars));
        }
        Ok(self.push_content(IslandContent::FileTransfer { label, bytes_done, bytes_total }, Priority::Normal, at_ms, Some(6_000)).id)
    }

    /// Shared push path for clipboard notifications (kept private; clipboard previews
    /// carry no user-typed text, only an opaque preview the host computed).
    fn push_clipboard_helper(&mut self, preview: String, priority: Priority, now_ms: u64) -> Result<IslandNotification, IslandError> {
        let chars = preview.chars().count();
        if chars == 0 || chars > MAX_TOAST_CHARS {
            return Err(IslandError::BadToastLength(chars));
        }
        Ok(self.push_content(IslandContent::ClipboardStored { preview }, priority, now_ms, Some(6_000)))
    }

    /// Dismiss the currently active notification. Returns the number dismissed (0 or 1) —
    /// i.e. it behaves like [`Self::dismiss_active`] and reports whether anything popped.
    pub fn drain_queued(&mut self) -> usize {
        let had_active = self.active.is_some() as usize;
        self.dismiss_active();
        had_active
    }

    pub fn push_content(
        &mut self,
        content: IslandContent,
        priority: Priority,
        now_ms: u64,
        auto_dismiss_ms: Option<u64>,
    ) -> IslandNotification {
        let id = self.next_id;
        self.next_id += 1;
        let note = IslandNotification { id, priority, content, created_at_ms: now_ms, auto_dismiss_ms };
        self.enqueue(note.clone());
        note
    }

    fn enqueue(&mut self, note: IslandNotification) {
        // High priority preempts whatever is active and jumps the queue.
        if note.priority == Priority::High {
            if let Some(current) = self.active.take() {
                // The preempted notification returns to the front of the queue.
                self.queue.push_front(current);
            }
            self.active = Some(note);
        } else {
            self.queue.push_back(note);
            self.promote_if_idle();
        }
        while self.queue.len() > MAX_QUEUE {
            self.queue.pop_front();
        }
    }

    fn promote_if_idle(&mut self) {
        if self.active.is_none() {
            self.active = self.queue.pop_front();
        }
    }

    /// Dismiss a specific notification (active or queued).
    pub fn dismiss(&mut self, id: u64) -> Result<(), IslandError> {
        if self.active.as_ref().map(|n| n.id) == Some(id) {
            self.active = None;
            self.promote_if_idle();
            return Ok(());
        }
        let before = self.queue.len();
        self.queue.retain(|n| n.id != id);
        if self.queue.len() != before {
            return Ok(());
        }
        Err(IslandError::NotificationNotFound(id))
    }

    /// Dismiss the currently shown notification.
    pub fn dismiss_active(&mut self) {
        self.active = None;
        self.promote_if_idle();
    }

    /// Tick the world forward: expire auto-dismissing notifications and fire TimerDone.
    pub fn tick(&mut self, now_ms: u64) {
        // Expire active notification if due.
        let expired = self
            .active
            .as_ref()
            .and_then(|n| n.auto_dismiss_ms.map(|d| now_ms.saturating_sub(n.created_at_ms) >= d))
            .unwrap_or(false);
        if expired {
            self.active = None;
            self.promote_if_idle();
        }
        // Expire queued notifications that can never be shown in time.
        self.queue.retain(|n| {
            n.auto_dismiss_ms
                .map(|d| now_ms.saturating_sub(n.created_at_ms) < d)
                .unwrap_or(true)
        });
        // Fire the timer exactly once when it elapses.
        if let Some(timer) = &self.timer {
            if timer.is_finished(now_ms) {
                let label = timer.label.clone();
                self.timer = None;
                self.push_content(IslandContent::TimerDone { label }, Priority::High, now_ms, Some(10_000));
            }
        }
    }

    pub fn start_timer(&mut self, label: impl Into<String>, duration_ms: u64, now_ms: u64) -> Result<FocusTimer, IslandError> {
        if !(1_000..=8 * 3_600_000).contains(&duration_ms) {
            return Err(IslandError::BadTimerDuration);
        }
        let timer = FocusTimer {
            label: label.into(),
            duration_ms,
            started_at_ms: now_ms,
            paused_remaining_ms: None,
        };
        self.timer = Some(timer.clone());
        Ok(timer)
    }

    pub fn cancel_timer(&mut self) {
        self.timer = None;
    }

    pub fn toggle_timer_pause(&mut self, now_ms: u64) {
        if let Some(timer) = &mut self.timer {
            match timer.paused_remaining_ms {
                Some(_) => {
                    // Resume: rebase start so remaining is preserved.
                    let remaining = timer.paused_remaining_ms.take().unwrap_or(0);
                    timer.started_at_ms = now_ms.saturating_sub(timer.duration_ms.saturating_sub(remaining));
                }
                None => {
                    let remaining = timer.remaining_ms(now_ms);
                    timer.paused_remaining_ms = Some(remaining);
                }
            }
        }
    }

    pub fn expand(&mut self) {
        self.view = IslandView::Expanded;
    }

    pub fn collapse(&mut self) {
        self.view = IslandView::Collapsed;
    }

    pub fn toggle_expand(&mut self) {
        self.view = match self.view {
            IslandView::Collapsed => IslandView::Expanded,
            IslandView::Expanded => IslandView::Collapsed,
        };
    }

    pub fn set_expanded(&mut self, expanded: bool) {
        self.view = if expanded { IslandView::Expanded } else { IslandView::Collapsed };
    }

    /// Host pushes (or clears with `None`) the currently-playing media item.
    pub fn set_now_playing(&mut self, now_playing: Option<NowPlaying>) {
        self.now_playing = now_playing;
    }

    /// Host pushes (or clears with `None`) the coarse local system load sample.
    pub fn set_system(&mut self, system: Option<SystemStats>) {
        self.system = system;
    }

    /// Insert or update one in-flight/finished download. Label is trimmed to 80 chars.
    /// When the in-flight table is full, the oldest *done* entry is evicted; if none
    /// are done, the task is refused rather than evicting a live transfer.
    pub fn upsert_download(&mut self, task: DownloadTask) {
        let label = task.label.trim();
        if label.is_empty() {
            return;
        }
        let clamped_label: String = label.chars().take(80).collect();
        let mut task = task;
        task.label = clamped_label;
        if let Some(slot) = self.downloads.iter_mut().find(|d| d.id == task.id) {
            *slot = task;
            return;
        }
        let inflight = self.downloads.iter().filter(|d| !d.done).count();
        if inflight >= MAX_TRACK_DOWNLOADS {
            match self.downloads.iter().position(|d| d.done) {
                Some(pos) => {
                    self.downloads.remove(pos);
                }
                None => return, // refuse rather than evict a live transfer
            }
        }
        self.downloads.push(task);
        self.trim_downloads();
    }

    pub fn mark_download_done(&mut self, id: &str) {
        if let Some(d) = self.downloads.iter_mut().find(|d| d.id == id) {
            d.done = true;
        }
        self.trim_downloads();
    }

    pub fn remove_download(&mut self, id: &str) {
        self.downloads.retain(|d| d.id != id);
    }

    fn trim_downloads(&mut self) {
        let mut done_kept = 0;
        self.downloads.retain(|d| {
            if d.done {
                done_kept += 1;
                done_kept <= MAX_KEEP_DONE_DOWNLOADS
            } else {
                true
            }
        });
    }

    /// Remember a file dropped on the island (listing only, nothing is copied/moved).
    pub fn add_dropped_file(&mut self, file: DroppedFile) {
        if file.path.trim().is_empty() {
            return;
        }
        self.dropped_files.push(file);
        if self.dropped_files.len() > MAX_DROPPED_FILES {
            let excess = self.dropped_files.len() - MAX_DROPPED_FILES;
            self.dropped_files.drain(0..excess);
        }
    }

    pub fn clear_dropped_files(&mut self) {
        self.dropped_files.clear();
    }

    pub fn snapshot(&self) -> IslandSnapshot {
        IslandSnapshot {
            active: self.active.clone(),
            queued: self.queue.len(),
            view: self.view,
            timer: self.timer.clone(),
            now_playing: self.now_playing.clone(),
            downloads: self.downloads.clone(),
            downloads_active: self.downloads.iter().filter(|d| !d.done).count(),
            dropped_files: self.dropped_files.clone(),
            system: self.system,
        }
    }
}

/// Orbit Module Protocol v1 wire envelope. One JSON object per line over stdio.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IslandMessage {
    /// Protocol version; always 1.
    pub v: u32,
    /// Per-direction message id; replies echo the request id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    pub kind: IslandMessageKind,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IslandMessageKind {
    Request,
    Response,
    Event,
}

impl IslandMessage {
    pub fn request(id: u64, method: impl Into<String>, params: Option<Value>) -> Self {
        Self { v: 1, id: Some(id), kind: IslandMessageKind::Request, method: method.into(), params, result: None, error: None }
    }

    pub fn response(id: u64, method: impl Into<String>, result: Value) -> Self {
        Self { v: 1, id: Some(id), kind: IslandMessageKind::Response, method: method.into(), params: None, result: Some(result), error: None }
    }

    pub fn error_response(id: u64, method: impl Into<String>, error: impl Into<String>) -> Self {
        Self { v: 1, id: Some(id), kind: IslandMessageKind::Response, method: method.into(), params: None, result: None, error: Some(error.into()) }
    }

    pub fn event(method: impl Into<String>, params: Value) -> Self {
        Self { v: 1, id: None, kind: IslandMessageKind::Event, method: method.into(), params: Some(params), result: None, error: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_length_validated() {
        let mut m = IslandManager::new();
        assert!(matches!(m.toast("", Priority::Normal, 0), Err(IslandError::BadToastLength(0))));
        assert!(matches!(m.toast("x".repeat(141), Priority::Normal, 0), Err(IslandError::BadToastLength(141))));
        assert!(m.toast("ok", Priority::Normal, 0).is_ok());
    }

    #[test]
    fn high_priority_preempts_active() {
        let mut m = IslandManager::new();
        m.toast("first", Priority::Low, 0).unwrap();
        m.toast("important", Priority::High, 100).unwrap();
        assert_eq!(m.snapshot().active.unwrap().priority, Priority::High);
    }

    #[test]
    fn queue_capped_drop_oldest() {
        let mut m = IslandManager::new();
        m.dismiss_active(); // nothing to dismiss, but ensure idle path
        m.toast("active", Priority::Normal, 0).unwrap();
        for i in 0..(MAX_QUEUE + 5) {
            m.toast(format!("n{i}"), Priority::Normal, (i + 1) as u64).unwrap();
        }
        assert!(m.snapshot().queued <= MAX_QUEUE);
    }

    #[test]
    fn auto_dismiss_expiry() {
        let mut m = IslandManager::new();
        m.toast("temp", Priority::Normal, 0).unwrap(); // 6000ms ttl
        assert!(m.snapshot().active.is_some());
        m.tick(6_001);
        assert!(m.snapshot().active.is_none());
    }

    #[test]
    fn timer_fires_once_as_high_priority() {
        let mut m = IslandManager::new();
        m.start_timer("focus", 1_000, 0).unwrap();
        m.tick(1_001);
        let snap = m.snapshot();
        match snap.active.as_ref().map(|n| &n.content) {
            Some(IslandContent::TimerDone { label }) => assert_eq!(label, "focus"),
            other => panic!("expected TimerDone, got {other:?}"),
        }
        m.tick(1_002); // must not double-fire
        let snap2 = m.snapshot();
        assert!(matches!(snap2.active.as_ref().map(|n| &n.content), Some(IslandContent::TimerDone { .. })));
    }

    #[test]
    fn timer_pause_and_resume() {
        let mut m = IslandManager::new();
        m.start_timer("t", 10_000, 0).unwrap();
        m.toggle_timer_pause(2_000); // pause with 8000 remaining
        m.toggle_timer_pause(9_000); // resume: start rebased
        m.tick(9_999); // 999ms elapsed since resume -> not done yet
        assert!(!m.timer.as_ref().map(|t| t.is_finished(9_999)).unwrap_or(false));
        m.tick(17_001); // 8001ms elapsed since resume -> done
        assert!(matches!(m.snapshot().active.as_ref().map(|n| &n.content), Some(IslandContent::TimerDone { .. })));
    }

    #[test]
    fn bad_timer_duration_rejected() {
        let mut m = IslandManager::new();
        assert_eq!(m.start_timer("x", 500, 0).unwrap_err(), IslandError::BadTimerDuration);
        assert_eq!(m.start_timer("x", 9 * 3_600_000, 0).unwrap_err(), IslandError::BadTimerDuration);
    }

    #[test]
    fn notify_clipboard_pushes_normal_auto_dismiss() {
        let mut m = IslandManager::new();
        let id = m.notify_clipboard("cargo test --locked".to_string(), 100).unwrap();
        let snap = m.snapshot();
        let active = snap.active.as_ref().expect("clipboard notification should be active");
        assert_eq!(active.id, id);
        assert_eq!(active.priority, Priority::Normal);
        assert_eq!(active.auto_dismiss_ms, Some(6_000));
        assert!(matches!(&active.content, IslandContent::ClipboardStored { preview } if preview == "cargo test --locked"));
        // Validation: empty / too-long previews are rejected
        assert!(matches!(m.notify_clipboard(String::new(), 200), Err(IslandError::BadToastLength(0))));
        assert!(matches!(m.notify_clipboard("x".repeat(141), 200), Err(IslandError::BadToastLength(141))));
        assert_eq!(m.snapshot().queued, 0); // only the valid one remains queued/active
    }

    #[test]
    fn notify_file_transfer_pushes_progress_payload() {
        let mut m = IslandManager::new();
        let id = m.notify_file_transfer("iso.img".to_string(), 512, 1024, 300).unwrap();
        let snap = m.snapshot();
        let active = snap.active.as_ref().expect("transfer notification should be active");
        assert_eq!(active.id, id);
        assert_eq!(active.priority, Priority::Normal);
        assert!(matches!(
            &active.content,
            IslandContent::FileTransfer { label, bytes_done, bytes_total }
                if label == "iso.img" && *bytes_done == 512 && *bytes_total == 1024
        ));
        assert!(matches!(m.notify_file_transfer(String::new(), 0, 10, 400), Err(IslandError::BadToastLength(0))));
        assert!(matches!(m.notify_file_transfer("y".repeat(141), 0, 10, 400), Err(IslandError::BadToastLength(141))));
    }

    #[test]
    fn drain_queued_pops_active_and_counts() {
        let mut m = IslandManager::new();
        assert_eq!(m.drain_queued(), 0); // nothing active → 0 dismissed
        m.notify_clipboard("first".into(), 0).unwrap();
        m.notify_file_transfer("second".into(), 1, 2, 10).unwrap(); // queued behind first
        assert_eq!(m.drain_queued(), 1); // pops active ("first"); promotes next
        assert_eq!(m.snapshot().queued, 0);
        assert!(matches!(m.snapshot().active.as_ref().map(|n| &n.content), Some(IslandContent::FileTransfer { .. })));
        assert_eq!(m.drain_queued(), 1); // pops the promoted transfer
        assert_eq!(m.snapshot().active.as_ref(), None);
        assert_eq!(m.drain_queued(), 0);
    }

    #[test]
    fn message_envelope_roundtrip() {
        let req = IslandMessage::request(7, "island.start_timer", Some(serde_json::json!({"label":"focus","duration_ms":60000})));
        let json = serde_json::to_string(&req).unwrap();
        let back: IslandMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, req);
        assert_eq!(back.v, 1);

        let evt = IslandMessage::event("island.changed", serde_json::json!({"expanded":true}));
        let json = serde_json::to_string(&evt).unwrap();
        assert!(json.contains("\"kind\":\"event\""));
        assert!(!json.contains("\"id\"") || !json.contains("\"id\":"));
    }

    #[test]
    fn ambient_now_playing_downloads_and_drops() {
        let mut m = IslandManager::new();
        assert!(m.snapshot().now_playing.is_none());

        m.set_now_playing(Some(NowPlaying {
            player: "spotify_client".into(),
            title: "A Change Is Gonna Come".into(),
            artist: "Sam Cooke".into(),
            position_secs: 12,
            length_secs: Some(232),
            playing: true,
            volume: None,
        }));
        assert_eq!(m.snapshot().now_playing.as_ref().unwrap().title, "A Change Is Gonna Come");
        m.set_now_playing(None);

        m.upsert_download(DownloadTask {
            id: "dl-1".into(),
            label: "fedora.iso".into(),
            received_bytes: 10,
            total_bytes: Some(100),
            done: false,
        });
        assert_eq!(m.snapshot().downloads_active, 1);
        m.mark_download_done("dl-1");
        assert_eq!(m.snapshot().downloads_active, 0);
        assert_eq!(m.snapshot().downloads.len(), 1);

        m.add_dropped_file(DroppedFile { path: "/tmp/x.png".into(), bytes: 5 });
        m.add_dropped_file(DroppedFile { path: "".into(), bytes: 5 });
        assert_eq!(m.snapshot().dropped_files.len(), 1);
        m.clear_dropped_files();
        assert!(m.snapshot().dropped_files.is_empty());
    }

    #[test]
    fn downloads_refuse_nine_in_flight() {
        let mut m = IslandManager::new();
        for i in 0..MAX_TRACK_DOWNLOADS {
            m.upsert_download(DownloadTask {
                id: format!("live-{i}"),
                label: format!("live-{i}.bin"),
                received_bytes: 0,
                total_bytes: None,
                done: false,
            });
        }
        m.upsert_download(DownloadTask {
            id: "live-overflow".into(),
            label: "overflow.bin".into(),
            received_bytes: 0,
            total_bytes: None,
            done: false,
        });
        assert!(m.snapshot().downloads.iter().all(|d| d.id != "live-overflow"));
    }
}
