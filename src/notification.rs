use serde::Deserialize;

#[derive(Deserialize)]
pub struct Presence { pub id: String, pub focused: bool, pub rendered_at: u64 }
struct Client { presence: Presence, seen_at: u64 }

#[derive(Default)]
pub struct Notification {
    clients: Vec<Client>,
    signals: Vec<String>,
    pending: Option<(Vec<String>, u64, u64)>,
}

impl Notification {
    pub const fn new(signals: Vec<String>) -> Self { Self { signals, clients: Vec::new(), pending: None } }

    pub fn presence(&mut self, presence: Presence, time: u64) {
        self.clients.retain(|c| time.saturating_sub(c.seen_at) < 5000 && c.presence.id != presence.id);
        self.clients.push(Client { presence, seen_at: time });
    }

    pub fn poll(&mut self, signals: Vec<String>, revision: u64, time: u64) -> bool {
        self.clients.retain(|c| time.saturating_sub(c.seen_at) < 5000);
        let added: Vec<_> = signals.iter().filter(|s| !self.signals.contains(s)).cloned().collect();
        if let Some((pending, _, _)) = &mut self.pending { pending.retain(|s| signals.contains(s)); }
        if !added.is_empty() {
            let mut pending = self.pending.take().map(|p| p.0).unwrap_or_default();
            pending.extend(added);
            self.pending = Some((pending, revision, time));
        }
        self.signals = signals;
        if self.pending.as_ref().is_some_and(|p| p.0.is_empty()) { self.pending = None; }
        let Some((_, revision, since)) = &self.pending else { return false; };
        if self.clients.iter().any(|c| c.presence.focused) { self.pending = None; return false; }
        // Let polling boards apply the task state before alerting. Sleeping/closed tabs expire.
        if time.saturating_sub(*since) < 1500 || self.clients.iter().any(|c| c.presence.rendered_at < *revision) { return false; }
        self.pending = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(id: &str, focused: bool, rendered_at: u64) -> Presence {
        Presence { id: id.into(), focused, rendered_at }
    }

    #[test]
    fn waits_for_board_then_alerts_once() {
        let mut n = Notification::new(vec![]);
        n.presence(client("tab", false, 1), 1000);
        assert!(!n.poll(vec!["finished".into()], 2, 1000));
        assert!(!n.poll(vec!["finished".into()], 2, 2600));
        n.presence(client("tab", false, 2), 2700);
        assert!(n.poll(vec!["finished".into()], 2, 2700));
        assert!(!n.poll(vec!["finished".into()], 2, 5000));
    }

    #[test]
    fn any_focused_tab_suppresses_without_a_late_alert() {
        let mut n = Notification::new(vec![]);
        n.presence(client("background", false, 2), 1000);
        n.presence(client("foreground", true, 2), 1000);
        assert!(!n.poll(vec!["finished".into()], 2, 1000));
        n.presence(client("foreground", false, 2), 1100);
        assert!(!n.poll(vec!["finished".into()], 2, 2700));
    }

    #[test]
    fn closed_tabs_expire_and_no_browser_is_required() {
        let mut n = Notification::new(vec![]);
        n.presence(client("closed", false, 1), 1000);
        assert!(!n.poll(vec!["finished".into()], 2, 1000));
        assert!(!n.poll(vec!["finished".into()], 2, 2600));
        assert!(n.poll(vec!["finished".into()], 2, 6000));
        assert!(!n.poll(vec![], 3, 7000));
        assert!(!n.poll(vec!["finished".into()], 4, 8000));
        assert!(n.poll(vec!["finished".into()], 4, 9600));
    }

    #[test]
    fn startup_is_silent_and_new_work_cancels_pending_alert() {
        let mut n = Notification::new(vec!["finished".into()]);
        assert!(!n.poll(vec!["finished".into()], 1, 1000));
        assert!(!n.poll(vec![], 2, 2000));
        assert!(!n.poll(vec!["finished".into()], 3, 3000));
        assert!(!n.poll(vec![], 4, 4000));
        assert!(!n.poll(vec![], 4, 6000));
    }

    #[test]
    fn each_new_attention_event_alerts_while_existing_issues_stay_silent() {
        let issue = vec!["task-a:1".into()];
        let mut n = Notification::new(issue.clone());
        assert!(!n.poll(issue.clone(), 1, 1000));
        let both = vec!["task-a:1".into(), "task-b:1".into()];
        assert!(!n.poll(both.clone(), 2, 2000));
        assert!(n.poll(both.clone(), 2, 3600));
        assert!(!n.poll(both, 2, 6000));
        let follow_up = vec!["task-a:2".into(), "task-b:1".into()];
        assert!(!n.poll(follow_up.clone(), 3, 7000));
        assert!(n.poll(follow_up, 3, 8600));
        assert!(!n.poll(vec![], 4, 9000));
        assert!(!n.poll(issue.clone(), 5, 10000));
        assert!(n.poll(issue, 5, 11600));
    }

    #[test]
    fn resolving_new_issue_cancels_alert_even_if_an_old_issue_remains() {
        let old = vec!["old".into()];
        let mut n = Notification::new(old.clone());
        assert!(!n.poll(vec!["old".into(), "new".into()], 1, 1000));
        assert!(!n.poll(old.clone(), 2, 2000));
        assert!(!n.poll(old, 2, 3000));
    }
}
