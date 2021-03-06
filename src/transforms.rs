use std::f32::consts::PI;
use std::f32;
use std::collections::VecDeque;

use cgmath::{Vector2, vec2, InnerSpace, MetricSpace};

pub struct LowPassFilter {
    first_time: bool,
    pub hat_x_prev: f32,
}

impl LowPassFilter {
    pub fn new() -> LowPassFilter {
        LowPassFilter {
            first_time: true,
            hat_x_prev: 0.0,
        }
    }

    pub fn filter(&mut self, x: f32, alpha: f32) -> f32 {
        if self.first_time {
            self.first_time = false;
            self.hat_x_prev = x;
        }
        let hatx = alpha * x + (1.0 - alpha) * self.hat_x_prev;
        self.hat_x_prev = hatx;
        hatx
    }
}

pub struct OneEuroFilter {
    first_time: bool,
    mincutoff: f32,
    beta: f32,
    dcutoff: f32,
    xfilt: LowPassFilter,
    dxfilt: LowPassFilter,
}

impl OneEuroFilter {
    pub fn new(mincutoff: f32, beta: f32, dcutoff: f32) -> Self {
        OneEuroFilter {
            first_time: true,
            mincutoff,
            beta,
            dcutoff,
            xfilt: LowPassFilter::new(),
            dxfilt: LowPassFilter::new(),
        }
    }

    pub fn filter(&mut self, x: f32, dt: f32) -> f32 {
        let rate = 1.0 / dt;
        let dx = if self.first_time {
            self.first_time = false;
            0.0
        } else {
            (x - self.xfilt.hat_x_prev) * rate
        };

        let edx = self.dxfilt.filter(dx, Self::alpha(rate, self.dcutoff));
        let cutoff = self.mincutoff + self.beta * edx.abs();
        self.xfilt.filter(x, Self::alpha(rate, cutoff))
    }

    fn alpha(rate: f32, cutoff: f32) -> f32 {
        let tau = 1.0 / (2.0 * PI * cutoff);
        let te = 1.0 / rate;
        1.0 / (1.0 + (tau / te))
    }
}

pub struct VecOneEuroFilter {
    xf: OneEuroFilter,
    yf: OneEuroFilter,
}

impl VecOneEuroFilter {
    pub fn new(mincutoff: f32, beta: f32, dcutoff: f32) -> Self {
        VecOneEuroFilter {
            xf: OneEuroFilter::new(mincutoff, beta, dcutoff),
            yf: OneEuroFilter::new(mincutoff, beta, dcutoff),
        }
    }

    pub fn filter(&mut self, x: Vector2<f32>, dt: f32) -> Vector2<f32> {
        vec2(self.xf.filter(x.x, dt), self.yf.filter(x.y, dt))
    }
}

/// Based on page 16 of Mathieu Nancel's "Mid-Air Pointing on Ultra-Walls" paper
/// See the paper for how to set the constants.
pub struct Acceleration {
    pub cd_min: f32,
    pub cd_max: f32,
    pub v_min: f32,
    pub v_max: f32,
    pub lambda: f32,
    pub ratio: f32,
}

impl Acceleration {
    pub fn transform(&self, diff: f32, dt: f32) -> f32 {
        let v_inf = self.ratio * (self.v_max - self.v_min) + self.v_min;
        let raw_vel = diff * dt;
        let exponent = -self.lambda * (raw_vel.abs() - v_inf);
        let cd = ((self.cd_max - self.cd_min) / (1.0 + f32::exp(exponent))) + self.cd_min;
        diff * cd
    }
}

pub struct AccumulatingRounder {
    accum: f32,
}

impl AccumulatingRounder {
    pub fn new() -> Self {
        AccumulatingRounder { accum: 0.0 }
    }

    pub fn round(&mut self, x: f32) -> i32 {
        let mut res = x.trunc();
        self.accum += x.fract();
        if self.accum.abs() >= 1.0 {
            let nudge = self.accum.signum();
            res += nudge;
            self.accum -= nudge;
        }
        res as i32
    }
}

pub struct FixationFilter {
    buffer: VecDeque<Vector2<f32>>,
    pub min_fixation_s: f32,
    pub max_velocity: f32,
    pub cur: Vector2<f32>,
}

impl FixationFilter {
    const MAX_BUFFER: usize = 128;

    pub fn new(min_fixation_s: f32, max_velocity: f32) -> Self {
        FixationFilter {
            min_fixation_s,
            max_velocity,
            buffer: VecDeque::with_capacity(Self::MAX_BUFFER),
            cur: vec2(0.0, 0.0),
        }
    }

    pub fn transform(&mut self, pt: Vector2<f32>, dt: f32) -> Vector2<f32> {
        if self.buffer.len() >= Self::MAX_BUFFER {
            self.buffer.pop_front();
        }
        self.buffer.push_back(pt);
        let len = self.buffer.len();

        if dt == 0.0 {
            return pt;
        }
        let mut to_sample = (self.min_fixation_s / dt).round() as usize;
        if to_sample > len {
            // println!("Warning: need {:?} fixation samples but only have {}", to_sample, len);
            to_sample = len;
        }

        // compute dispersion for to_sample by the method from the I-DT algorithm
        let mut min = pt;
        let mut max = pt;
        for i in (len - to_sample)..len {
            let el = self.buffer.get(i).unwrap();
            if el.x < min.x {
                min.x = el.x;
            }
            if el.y < min.y {
                min.y = el.y;
            }
            if el.x >= max.x {
                max.x = el.x;
            }
            if el.y >= max.y {
                max.y = el.y;
            }
        }
        let diffs = max - min;
        let dispersion = diffs.x + diffs.y;

        let max_dispersion = self.max_velocity * self.min_fixation_s;
        if dispersion < max_dispersion {
            self.cur = pt;
        }
        self.cur
    }
}

#[derive(Clone)]
pub struct PolyMouseParams {
    pub min_jump: f32,
    pub speed_expand_factor: f32,
    pub head_smoothing_factor: f32,
    pub throw_thresh_speed: f32,
    pub throw_speed: f32,
    pub small_jump_factor: f32,
}

pub struct PolyMouseTransform {
    params: PolyMouseParams,
    throwing: bool,
    smoothed_head_speed: f32,
    pub last_jump_destination: Vector2<f32>,
    x_round: AccumulatingRounder,
    y_round: AccumulatingRounder,
}

impl PolyMouseTransform {
    pub fn new(params: PolyMouseParams) -> Self {
        PolyMouseTransform {
            params,
            throwing: false,
            smoothed_head_speed: 0.0,
            last_jump_destination: vec2(0.0, 0.0),
            x_round: AccumulatingRounder::new(),
            y_round: AccumulatingRounder::new(),
        }
    }

    pub fn transform(&mut self,
                     gaze_pt: Vector2<f32>,
                     mouse_pt: Vector2<i32>,
                     head_delta: Vector2<f32>,
                     dt: f32)
                     -> Vector2<i32> {
        let mouse_pt_f = vec2(mouse_pt.x as f32, mouse_pt.y as f32);

        // TODO this is accelerated speed, should the acceleration be after?
        let head_speed = head_delta.magnitude() / dt;
        // TODO the amount of smoothing isn't independent of dt
        self.smoothed_head_speed = self.smoothed_head_speed *
                                   (1.0 - self.params.head_smoothing_factor) +
                                   head_speed * self.params.head_smoothing_factor;

        // println!("{:?}", self.smoothed_head_speed);
        if self.looking_far_away(gaze_pt, mouse_pt_f) &&
           self.smoothed_head_speed > self.params.throw_thresh_speed {
            self.throwing = true;
        }

        if self.throwing {
            let throw_dist = self.params.throw_speed * dt;
            let dirn = (gaze_pt - mouse_pt_f).normalize();

            // check we're not jumping past the circle
            let dest_f = if mouse_pt_f.distance(gaze_pt) > throw_dist + self.params.min_jump {
                mouse_pt_f + dirn * throw_dist
            } else {
                self.last_jump_destination = gaze_pt;
                self.throwing = false;
                gaze_pt + dirn * (-self.params.min_jump)
            };

            vec2(dest_f.x as i32, dest_f.y as i32) // TODO round?
        } else {
            let rounded_move = vec2(self.x_round.round(head_delta.x),
                                    self.y_round.round(head_delta.y));
            mouse_pt + rounded_move
        }
    }

    fn looking_far_away(&self, gaze_pt: Vector2<f32>, mouse_pt: Vector2<f32>) -> bool {
        let jump_radius = self.params.min_jump +
                          self.smoothed_head_speed * self.params.speed_expand_factor;
        let small_jump = jump_radius * self.params.small_jump_factor;
        mouse_pt.distance(gaze_pt) > jump_radius &&
        self.last_jump_destination.distance(gaze_pt) > small_jump
    }
}
