#[derive(Default)]
pub struct RestartBudget {
    attempts: u8,
    last_attempt: u64,
    healthy_since: Option<u64>,
}
impl RestartBudget {
    pub fn tick(&mut self, now: u64, running: bool, desired: bool) -> bool {
        if !desired {
            self.attempts = 0;
            self.healthy_since = None;
            return false;
        }
        if running {
            let since = *self.healthy_since.get_or_insert(now);
            if now.saturating_sub(since) >= 60_000 {
                self.attempts = 0;
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
}
