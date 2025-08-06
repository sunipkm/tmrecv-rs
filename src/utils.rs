use std::time::Instant;

pub struct DataRateCounter {
    count: usize,
    start: Instant,
}

impl Default for DataRateCounter {
    fn default() -> Self {
        Self {
            count: 0,
            start: Instant::now(),
        }
    }
}

impl DataRateCounter {
    pub fn update(&mut self, bytes: usize) {
        self.count += bytes;
    }

    pub fn reset(&mut self) -> Result<(Instant, f32, &'static str), Instant> {
        let now = Instant::now();
        let dur = now.duration_since(self.start).as_secs_f32();
        if dur > 1.0 && self.count > 0 {
            let mut drate = (self.count * 8) as f32 / dur;
            self.count = 0;
            self.start = now;
            let mut unit = "bps";
            if drate > 1024.0 {
                drate /= 1024.0;
                unit = "kbps";
            }
            if drate > 1024.0 {
                drate /= 1024.0;
                unit = "mbps";
            }
            Ok((now, drate, unit))
        } else {
            Err(now)
        }
    }
}
