#[derive(Default)]
pub struct RestartBudget {
    attempts: u8,
    last_attempt: u64,
    healthy_since: Option<u64>,
    exhaustion_reported: bool,
}
impl RestartBudget {
    pub fn tick(&mut self, now: u64, running: bool, desired: bool) -> bool {
        if !desired {
            self.attempts = 0;
            self.healthy_since = None;
            self.exhaustion_reported = false;
            return false;
        }
        if running {
            let since = *self.healthy_since.get_or_insert(now);
            if now.saturating_sub(since) >= 60_000 {
                self.attempts = 0;
                self.exhaustion_reported = false;
            }
            return false;
        }
        self.healthy_since = None;
        if self.attempts >= 3 || now.saturating_sub(self.last_attempt) < 5_000 {
            return false;
        }
        self.attempts += 1;
        self.last_attempt = now;
        true
    }

    pub fn take_exhausted_notice(&mut self, running: bool, desired: bool) -> bool {
        if running || !desired || self.attempts < 3 || self.exhaustion_reported {
            return false;
        }
        self.exhaustion_reported = true;
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limits_crash_restarts_and_resets_after_stable_operation() {
        let mut b = RestartBudget::default();
        assert!(b.tick(5000, false, true));
        assert!(!b.tick(6000, false, true));
        assert!(b.tick(10000, false, true));
        assert!(b.tick(15000, false, true));
        assert!(!b.tick(20000, false, true));
        assert!(!b.tick(21000, true, true));
        assert!(!b.tick(81000, true, true));
        assert!(b.tick(82000, false, true));
        assert!(!b.tick(90000, false, false));
        assert!(b.tick(96000, false, true));
    }

    #[test]
    fn exhaustion_is_reported_once_and_resets_after_an_explicit_stop() {
        let mut b = RestartBudget::default();
        for now in [5000, 10000, 15000] {
            assert!(b.tick(now, false, true));
        }
        assert!(!b.take_exhausted_notice(true, true));
        assert!(b.take_exhausted_notice(false, true));
        assert!(!b.take_exhausted_notice(false, true));
        assert!(!b.tick(20000, false, false));
        assert!(!b.take_exhausted_notice(false, true));
        for now in [25000, 30000, 35000] {
            assert!(b.tick(now, false, true));
        }
        assert!(b.take_exhausted_notice(false, true));
    }
}
