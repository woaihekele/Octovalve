use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub(crate) struct ActivityTracker {
    state: Mutex<ActivityState>,
}

#[derive(Debug)]
struct ActivityState {
    control_connections: usize,
    data_connections: usize,
    idle_since: Option<Instant>,
}

#[derive(Debug, Clone, Copy)]
enum ConnectionKind {
    Control,
    Data,
}

pub(crate) struct ActivityGuard {
    tracker: Arc<ActivityTracker>,
    kind: ConnectionKind,
}

impl ActivityTracker {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(ActivityState {
                control_connections: 0,
                data_connections: 0,
                idle_since: Some(Instant::now()),
            }),
        }
    }

    pub(crate) fn track_control(self: &Arc<Self>) -> ActivityGuard {
        self.note_open(ConnectionKind::Control);
        ActivityGuard {
            tracker: Arc::clone(self),
            kind: ConnectionKind::Control,
        }
    }

    pub(crate) fn track_data(self: &Arc<Self>) -> ActivityGuard {
        self.note_open(ConnectionKind::Data);
        ActivityGuard {
            tracker: Arc::clone(self),
            kind: ConnectionKind::Data,
        }
    }

    pub(crate) fn idle_for(&self) -> Option<Duration> {
        let state = self.state.lock().expect("activity lock");
        if state.control_connections == 0 && state.data_connections == 0 {
            state.idle_since.map(|since| since.elapsed())
        } else {
            None
        }
    }

    fn note_open(&self, kind: ConnectionKind) {
        let mut state = self.state.lock().expect("activity lock");
        match kind {
            ConnectionKind::Control => state.control_connections += 1,
            ConnectionKind::Data => state.data_connections += 1,
        }
        state.idle_since = None;
    }

    fn note_close(&self, kind: ConnectionKind) {
        let mut state = self.state.lock().expect("activity lock");
        match kind {
            ConnectionKind::Control => {
                if state.control_connections > 0 {
                    state.control_connections -= 1;
                }
            }
            ConnectionKind::Data => {
                if state.data_connections > 0 {
                    state.data_connections -= 1;
                }
            }
        }
        if state.control_connections == 0 && state.data_connections == 0 {
            state.idle_since = Some(Instant::now());
        }
    }
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        self.tracker.note_close(self.kind);
    }
}

pub(crate) fn spawn_idle_shutdown(activity: Arc<ActivityTracker>, ttl: Duration) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            if let Some(idle_for) = activity.idle_for() {
                if idle_for >= ttl {
                    tracing::info!(
                        idle_for_secs = idle_for.as_secs(),
                        "no clients detected, shutting down"
                    );
                    std::process::exit(0);
                }
            }
        }
    });
}
