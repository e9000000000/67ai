use serde::Deserialize;

#[derive(Deserialize)]
pub struct Presence { pub id: String, pub focused: bool, pub rendered_at: u64, #[serde(default)] pub sequence: u64 }
struct Client { presence: Presence, seen_at: u64 }

#[derive(Default)]
pub struct Notification {
    clients: Vec<Client>,
    ready: bool,
    pending: Option<(u64, u64)>,
}

impl Notification {
    pub const fn new(ready: bool) -> Self { Self { ready, clients: Vec::new(), pending: None } }

    pub fn presence(&mut self, presence: Presence, time: u64) {
        if self.clients.iter().any(|c| c.presence.id == presence.id && c.presence.sequence > presence.sequence) { return; }
        if presence.focused { self.pending = None; }
        self.clients.retain(|c| time.saturating_sub(c.seen_at) < 5000 && c.presence.id != presence.id);
        self.clients.push(Client { presence, seen_at: time });
    }

    pub fn poll(&mut self, ready: bool, revision: u64, time: u64) -> bool {
        self.clients.retain(|c| time.saturating_sub(c.seen_at) < 5000);
        if !ready { self.pending = None; }
        else if !self.ready { self.pending = Some((revision, time)); }
        self.ready = ready;
        let Some((revision, since)) = self.pending else { return false; };
        if self.clients.iter().any(|c| c.presence.focused) { self.pending = None; return false; }
        // Let polling boards apply Done before alerting. Sleeping/closed tabs expire.
        if time.saturating_sub(since) < 1500 || self.clients.iter().any(|c| c.presence.rendered_at < revision) { return false; }
        self.pending = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(id: &str, focused: bool, rendered_at: u64) -> Presence {
        Presence { id: id.into(), focused, rendered_at, sequence: 0 }
    }

    #[test]
    fn waits_for_board_then_alerts_once() {
        let mut n = Notification::new(false);
        n.presence(client("tab", false, 1), 1000);
        assert!(!n.poll(true, 2, 1000));
        assert!(!n.poll(true, 2, 2600));
        n.presence(client("tab", false, 2), 2700);
        assert!(n.poll(true, 2, 2700));
        assert!(!n.poll(true, 2, 5000));
    }

    #[test]
    fn any_focused_tab_suppresses_without_a_late_alert() {
        let mut n = Notification::new(false);
        n.presence(client("background", false, 2), 1000);
        n.presence(client("foreground", true, 2), 1000);
        assert!(!n.poll(true, 2, 1000));
        n.presence(client("foreground", false, 2), 1100);
        assert!(!n.poll(true, 2, 2700));
    }

    #[test]
    fn refocusing_between_polls_cancels_pending_alert() {
        let mut n = Notification::new(false);
        n.presence(client("tab", false, 2), 1000);
        assert!(!n.poll(true, 2, 1000));
        n.presence(client("tab", true, 2), 1100);
        n.presence(client("tab", false, 2), 1200);
        assert!(!n.poll(true, 2, 2700));
        assert!(!n.poll(true, 2, 7000));
    }

    #[test]
    fn delayed_reports_cannot_overwrite_newer_focus_state() {
        let mut n = Notification::new(false);
        n.presence(Presence { sequence: 2, ..client("tab", true, 2) }, 1000);
        n.presence(Presence { sequence: 1, ..client("tab", false, 1) }, 1100);
        assert!(!n.poll(true, 2, 1200));
        assert!(!n.poll(true, 2, 7000));

        assert!(!n.poll(false, 3, 8000));
        n.presence(Presence { sequence: 4, ..client("tab", false, 4) }, 9000);
        assert!(!n.poll(true, 4, 9000));
        n.presence(Presence { sequence: 3, ..client("tab", true, 3) }, 9100);
        assert!(n.poll(true, 4, 10600));
    }

    #[test]
    fn closed_tabs_expire_and_no_browser_is_required() {
        let mut n = Notification::new(false);
        n.presence(client("closed", false, 1), 1000);
        assert!(!n.poll(true, 2, 1000));
        assert!(!n.poll(true, 2, 2600));
        assert!(n.poll(true, 2, 6000));
        assert!(!n.poll(false, 3, 7000));
        assert!(!n.poll(true, 4, 8000));
        assert!(n.poll(true, 4, 9600));
    }

    #[test]
    fn startup_is_silent_and_new_work_cancels_pending_alert() {
        let mut n = Notification::new(true);
        assert!(!n.poll(true, 1, 1000));
        assert!(!n.poll(false, 2, 2000));
        assert!(!n.poll(true, 3, 3000));
        assert!(!n.poll(false, 4, 4000));
        assert!(!n.poll(false, 4, 6000));
    }
}
