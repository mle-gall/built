//! Wayland sends finger-axis frames and axis_stop, but no inertial tail.
//! Estimate velocity in logical pixels/ms from the last two finger frames.
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Kinetic {
    sample: Option<u32>,
    velocity: (f64, f64),
    tick: Option<Instant>,
}
impl Kinetic {
    pub fn frame(&mut self, time: u32, delta: (f64, f64), stopped: bool, now: Instant) {
        let elapsed = self.sample.map(|prev| time.wrapping_sub(prev));
        if delta != (0., 0.) {
            self.tick = None;
            self.velocity = match elapsed {
                Some(ms @ 1..=80) => (delta.0 / f64::from(ms), delta.1 / f64::from(ms)),
                _ => (0., 0.),
            };
            self.sample = Some(time);
        }
        if stopped {
            self.tick = (elapsed.is_some_and(|ms| ms <= 80) && self.speed() >= 0.08).then_some(now);
            self.sample = None;
        }
    }
    fn speed(&self) -> f64 {
        self.velocity.0.hypot(self.velocity.1)
    }
    pub fn next(&self) -> Option<Instant> {
        self.tick.map(|t| t + Duration::from_millis(16))
    }
    pub fn advance(&mut self, now: Instant) -> Option<(f64, f64)> {
        if now < self.next()? {
            return None;
        }
        let ms = now.duration_since(self.tick?).as_secs_f64() * 1000.;
        if ms > 100. {
            *self = Self::default();
            return None;
        }
        // Integrate exponential friction over the actual frame interval.
        let decay = (-ms / 120.).exp();
        let distance = 120. * (1. - decay);
        let delta = (self.velocity.0 * distance, self.velocity.1 * distance);
        self.velocity = (self.velocity.0 * decay, self.velocity.1 * decay);
        self.tick = (self.speed() >= 0.015).then_some(now);
        Some(delta)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inertia_decays_and_a_new_contact_cancels_it() {
        let mut k = Kinetic::default();
        let start = Instant::now();
        k.frame(100, (0., -8.), false, start);
        k.frame(116, (0., -8.), false, start);
        k.frame(119, (0., 0.), true, start);
        let first = k.advance(start + Duration::from_millis(16)).unwrap();
        let next = k.advance(start + Duration::from_millis(32)).unwrap();
        assert!(first.1 < next.1 && next.1 < 0.);
        k.frame(140, (1., 0.), false, start);
        assert!(k.next().is_none());
    }
}
