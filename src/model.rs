//! ピアノの弦とアクションの物理モデル。
//!
//! 各弦は速度波を運ぶデジタル導波路。打弦点でフェルトのハンマー（べき乗則
//! ばね、接触力は弦ごとに陰的に解く）と、ダンパー（粘性の点接触。離鍵で
//! 時間をかけて降り、ペダル踏下や打鍵で上がる）が弦に作用する。すべての弦は
//! 共通の集中アドミタンス Y_b を持つブリッジで終端され、互いに結合する。
//! 打鍵した音の 3 本の弦だけでなく、他の鍵の弦もブリッジにつながっている
//! ので、ペダルを踏んでダンパーが上がれば共鳴弦として鳴る。出力はブリッジに
//! 掛かる力。

use crate::design::LoopDesign;

struct Delay {
    buf: Vec<f64>,
    idx: usize,
}

impl Delay {
    fn new(len: usize) -> Delay {
        Delay { buf: vec![0.0; len.max(1)], idx: 0 }
    }
    fn read(&self) -> f64 {
        self.buf[self.idx]
    }
    fn write(&mut self, v: f64) {
        self.buf[self.idx] = v;
        self.idx += 1;
        if self.idx == self.buf.len() {
            self.idx = 0;
        }
    }
}

struct Allpass {
    a: f64,
    x1: f64,
    y1: f64,
}

impl Allpass {
    fn new(a: f64) -> Allpass {
        Allpass { a, x1: 0.0, y1: 0.0 }
    }
    fn process(&mut self, x: f64) -> f64 {
        let y = self.a * x + self.x1 - self.a * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}

struct StringWg {
    /// 弦は 3 区間: ナット〜打弦点 (a)、打弦点〜ダンパー (b)、ダンパー〜ブリッジ (c)。
    /// 各区間に右向き (_r) と左向き (_l) の遅延線。ループフィルタ（分散・損失・
    /// 分数遅延）はブリッジ側の端にまとめて置くので、ナット側は純粋な遅延で、
    /// 打弦点とダンパーの位置はナットからの物理的な距離で決められる。
    a_r: Delay,
    a_l: Delay,
    b_r: Delay,
    b_l: Delay,
    c_r: Delay,
    c_l: Delay,
    thiran: Allpass,
    dispersion: Vec<Allpass>,
    loss_g: f64,
    loss_c: f64,
    loss_y1: f64,
    /// 接触点での弦の変位。
    y_s: f64,
    /// ヒステリシスの記憶: フェルト圧縮量^p のローパス。
    memory: f64,
}

impl StringWg {
    fn new(d: &LoopDesign, strike_ratio: f64, damper_ratio: f64) -> StringWg {
        let n = d.n_total;
        // 弦の実効的な片道長（遅延線 + フィルタの群遅延の半分。フィルタは
        // 往復で 1 回通る）に対する位置。
        let half = n as f64 + 0.5 * d.filter_delay(d.fs, d.f0);
        let na = ((half * strike_ratio).round() as usize).clamp(1, n.saturating_sub(2).max(1));
        let nd = ((half * damper_ratio).round() as usize).clamp(na + 1, n.saturating_sub(1).max(na + 1));
        let nb = nd - na;
        let nc = n.saturating_sub(nd).max(1);
        StringWg {
            a_r: Delay::new(na),
            a_l: Delay::new(na),
            b_r: Delay::new(nb),
            b_l: Delay::new(nb),
            c_r: Delay::new(nc),
            c_l: Delay::new(nc),
            thiran: Allpass::new(d.thiran_a),
            dispersion: (0..d.ap_count).map(|_| Allpass::new(d.ap_a)).collect(),
            loss_g: d.loss_g,
            loss_c: d.loss_c,
            loss_y1: 0.0,
            y_s: 0.0,
            memory: 0.0,
        }
    }

    fn loop_filter(&mut self, x: f64) -> f64 {
        let mut v = self.thiran.process(x);
        for ap in &mut self.dispersion {
            v = ap.process(v);
        }
        let y = self.loss_g * (1.0 - self.loss_c) * v + self.loss_c * self.loss_y1;
        self.loss_y1 = y;
        y
    }
}

/// フェルトハンマー。フェルトはべき乗則のばねに Stulov の遺伝形ヒステリシス
/// を加えたもの: F = K (x^p - eps * r)。r は x^p を時定数 `tau`（秒）で
/// ローパスしたもの。`eps` = 0 なら単純なべき乗則。
#[derive(Clone, Debug)]
pub struct Hammer {
    pub mass: f64,
    pub stiffness: f64,
    pub exponent: f64,
    pub eps: f64,
    pub tau: f64,
}

/// ダンパー: 接触点に置く粘性抵抗。`rho` は基準鍵（C4）での R / (2 Z)
/// （1 で弦の波動インピーダンスに整合し、1 回通過あたりの吸収が最大になる）。
/// 低い鍵ほど波が接触点を通る回数が少ないので、鍵ごとの抵抗は基音に反比例
/// させる（`rho_scale` が鍵ごとの倍率）。`ramp_s` は離鍵からダンパーが完全に
/// 降りるまでの時間。
#[derive(Clone, Debug)]
pub struct Damper {
    pub rho: f64,
    pub ramp_s: f64,
}

/// 1 鍵分の弦。打鍵する鍵は 3 本（ユニゾン）、共鳴用の鍵は 1 本でもよい。
pub struct KeyStrings {
    pub key: u8,
    /// ブリッジ上の結合の重み（打鍵点から遠い弦や別ブリッジの弦は小さい）。
    pub weight: f64,
    pub has_damper: bool,
    /// この鍵のダンパー抵抗の倍率（基準鍵に対して）。
    pub damper_scale: f64,
    strings: Vec<StringWg>,
    held: bool,
    hand_damped: bool,
    damper_r: f64,
}

impl KeyStrings {
    pub fn new(
        key: u8,
        weight: f64,
        has_damper: bool,
        damper_scale: f64,
        designs: &[LoopDesign],
        strike_ratio: f64,
        damper_ratio: f64,
    ) -> KeyStrings {
        KeyStrings {
            key,
            weight,
            has_damper,
            damper_scale,
            strings: designs.iter().map(|d| StringWg::new(d, strike_ratio, damper_ratio)).collect(),
            held: false,
            hand_damped: false,
            damper_r: 0.0,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Event {
    /// 打鍵。`velocity` はハンマー速度（m/s）。
    NoteOn { key: u8, velocity: f64 },
    NoteOff { key: u8 },
    Pedal(bool),
    /// ペダルを踏んだまま、その鍵の弦だけを手で止める（共鳴サンプルの収録条件）。
    HandDamp { key: u8 },
}

struct HammerState {
    key_index: usize,
    y_h: f64,
    v_h: f64,
}

pub struct Piano {
    fs: f64,
    /// 弦 1 本の特性インピーダンス（kg/s）。
    z: f64,
    /// ブリッジアドミタンス × 弦インピーダンス（無次元）。
    yz: f64,
    keys: Vec<KeyStrings>,
    hammer: Hammer,
    damper: Damper,
    pedal: bool,
    hammer_state: Option<HammerState>,
    /// 接触の統計: 接触していたサンプル数、合計力のピーク（N）、力の時系列。
    pub contact_samples: usize,
    pub peak_force: f64,
    pub force_trace: Vec<f64>,
}

impl Piano {
    pub fn new(fs: f64, z: f64, yz: f64, hammer: Hammer, damper: Damper, keys: Vec<KeyStrings>) -> Piano {
        Piano {
            fs,
            z,
            yz,
            keys,
            hammer,
            damper,
            pedal: false,
            hammer_state: None,
            contact_samples: 0,
            peak_force: 0.0,
            force_trace: Vec::new(),
        }
    }


    /// 鍵ごとの、遅延線に残っているエネルギーの目安（速度波の二乗和）。
    pub fn key_energies(&self) -> Vec<(u8, f64)> {
        self.keys
            .iter()
            .map(|k| {
                let e: f64 = k
                    .strings
                    .iter()
                    .map(|s| [&s.a_r, &s.a_l, &s.b_r, &s.b_l, &s.c_r, &s.c_l].iter().map(|d| d.buf.iter().map(|v| v * v).sum::<f64>()).sum::<f64>())
                    .sum();
                (k.key, e)
            })
            .collect()
    }

    fn apply(&mut self, ev: &Event) {
        match ev {
            Event::NoteOn { key, velocity } => {
                if let Some(i) = self.keys.iter().position(|k| k.key == *key) {
                    self.keys[i].held = true;
                    self.keys[i].hand_damped = false;
                    self.hammer_state = Some(HammerState { key_index: i, y_h: 0.0, v_h: *velocity });
                    for s in &mut self.keys[i].strings {
                        s.y_s = 0.0;
                        s.memory = 0.0;
                    }
                }
            }
            Event::NoteOff { key } => {
                if let Some(k) = self.keys.iter_mut().find(|k| k.key == *key) {
                    k.held = false;
                }
            }
            Event::Pedal(down) => self.pedal = *down,
            Event::HandDamp { key } => {
                if let Some(k) = self.keys.iter_mut().find(|k| k.key == *key) {
                    k.hand_damped = true;
                }
            }
        }
    }

    /// F = K ((c0 - c1 F)^p - eps * r) を F >= 0（c0 > 0, c1 > 0）について
    /// 二分法で解く。右辺は F について単調減少なので根は一つ。
    fn contact_force(k: f64, p: f64, eps_r: f64, c0: f64, c1: f64) -> f64 {
        if c0 <= 0.0 || k * (c0.powf(p) - eps_r) <= 0.0 {
            return 0.0;
        }
        let (mut lo, mut hi) = (0.0, c0 / c1);
        for _ in 0..50 {
            let mid = 0.5 * (lo + hi);
            let g = mid - k * ((c0 - c1 * mid).max(0.0).powf(p) - eps_r);
            if g < 0.0 {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        0.5 * (lo + hi)
    }

    /// `n` サンプル描画する。`events` は (サンプル位置, イベント) を時間順に。
    pub fn render(&mut self, n: usize, events: &[(usize, Event)]) -> Vec<f64> {
        let dt = 1.0 / self.fs;
        let z = self.z;
        let r_base = self.damper.rho * 2.0 * z;
        let ramp = (self.damper.ramp_s * self.fs).max(1.0);
        let mut out = Vec::with_capacity(n);
        let total_strings: usize = self.keys.iter().map(|k| k.strings.len()).sum();
        let mut a = vec![0.0; total_strings];
        let mut b = vec![0.0; total_strings];
        let mut c = vec![0.0; total_strings];
        let mut d = vec![0.0; total_strings];
        let mut x_nut = vec![0.0; total_strings];
        let mut v_in = vec![0.0; total_strings];
        let mut next_event = 0;
        let mut key_sum = vec![0.0; self.keys.len()];
        let key_of_offset: Vec<usize> = {
            let mut v = Vec::new();
            for (ki, k) in self.keys.iter().enumerate() {
                for _ in &k.strings {
                    v.push(ki);
                }
            }
            v
        };
        let kidx_of = |offset: usize| key_of_offset[offset];
        let (k_h, p_h, eps, tau) = (self.hammer.stiffness, self.hammer.exponent, self.hammer.eps, self.hammer.tau);
        for t in 0..n {
            while next_event < events.len() && events[next_event].0 <= t {
                let ev = events[next_event].1.clone();
                self.apply(&ev);
                next_event += 1;
            }
            // ダンパーの目標位置と移動。
            let pedal = self.pedal;
            for k in &mut self.keys {
                let down = k.has_damper && (k.hand_damped || !(k.held || pedal));
                let r_max = (r_base * k.damper_scale).min(2.0 * z);
                let target = if down { r_max } else { 0.0 };
                let diff = target - k.damper_r;
                let step = r_max / ramp;
                k.damper_r += diff.clamp(-step, step);
            }
            // 書き込む前にすべての遅延線を読む（ブリッジに届く波にはループフィルタを掛ける）。
            let mut idx = 0;
            for k in &mut self.keys {
                for s in &mut k.strings {
                    a[idx] = s.a_r.read();
                    b[idx] = s.b_l.read();
                    c[idx] = s.b_r.read();
                    d[idx] = s.c_l.read();
                    x_nut[idx] = s.a_l.read();
                    v_in[idx] = s.loop_filter(s.c_r.read());
                    idx += 1;
                }
            }
            // 打弦点: ハンマー力。ダンパー点: 粘性力。
            let hs = self.hammer_state.as_ref().map(|h| (h.key_index, h.y_h, h.v_h));
            let mut force_sum = 0.0;
            let mut any_contact = false;
            let mut idx = 0;
            for (ki, k) in self.keys.iter_mut().enumerate() {
                let rho = k.damper_r / (2.0 * z);
                for s in &mut k.strings {
                    let f_h = match hs {
                        Some((hk, y_h, v_h)) if hk == ki => {
                            let c0 = (y_h - s.y_s) + dt * (v_h - a[idx] - b[idx]);
                            let c1 = dt / (2.0 * z);
                            let f = Piano::contact_force(k_h, p_h, eps * s.memory, c0, c1);
                            if f > 0.0 {
                                any_contact = true;
                            }
                            let xp = (c0 - c1 * f).max(0.0).powf(p_h);
                            if tau > 0.0 {
                                s.memory += (dt / tau).min(1.0) * (xp - s.memory);
                            }
                            f
                        }
                        _ => 0.0,
                    };
                    force_sum += f_h;
                    let inject_h = f_h / (2.0 * z);
                    s.y_s += dt * (a[idx] + b[idx] + inject_h);
                    s.b_r.write(a[idx] + inject_h);
                    s.a_l.write(b[idx] + inject_h);
                    // ダンパー点: v_d = (c + d) / (1 + rho), F_d = -R v_d
                    let v_d = (c[idx] + d[idx]) / (1.0 + rho);
                    let inject_d = -k.damper_r * v_d / (2.0 * z);
                    s.c_r.write(c[idx] + inject_d);
                    s.b_l.write(d[idx] + inject_d);
                    idx += 1;
                }
            }
            if let Some(h) = &mut self.hammer_state {
                if any_contact {
                    self.contact_samples += 1;
                    self.peak_force = self.peak_force.max(force_sum);
                }
                self.force_trace.push(force_sum);
                h.v_h -= dt * force_sum / self.hammer.mass;
                h.y_h += dt * h.v_h;
                if !any_contact && h.v_h < 0.0 {
                    self.hammer_state = None;
                }
            }
            // ナット: 剛な終端（反転反射）。
            let mut idx = 0;
            for k in &mut self.keys {
                for s in &mut k.strings {
                    s.a_r.write(-x_nut[idx]);
                    idx += 1;
                }
            }
            // ブリッジ: 各弦は自分の取り付け点のアドミタンス（yz）を見る。同じ鍵の
            // ユニゾン弦は同じ点にあり（結合 1）、他の鍵の弦の運動は重み w_i w_j で
            // 伝わる。v_b,i = 2 yz (S_i + w_i Σ_{j≠i} w_j S_j) / (1 + yz n_i)。
            let mut idx = 0;
            let mut weighted_total = 0.0;
            for k in &self.keys {
                let n = k.strings.len();
                let s_key: f64 = v_in[idx..idx + n].iter().sum();
                key_sum[kidx_of(idx)] = s_key;
                weighted_total += k.weight * s_key;
                idx += n;
            }
            let mut f_bridge = 0.0;
            let mut idx = 0;
            for (ki, k) in self.keys.iter_mut().enumerate() {
                let n = k.strings.len() as f64;
                let s_key = key_sum[ki];
                let others = weighted_total - k.weight * s_key;
                let v_b = 2.0 * self.yz * (s_key + k.weight * others) / (1.0 + self.yz * n);
                for s in &mut k.strings {
                    s.c_l.write(v_b - v_in[idx]);
                    f_bridge += z * (2.0 * v_in[idx] - v_b);
                    idx += 1;
                }
            }
            out.push(f_bridge);
        }
        out
    }
}

/// 窓付き sinc ローパスで整数倍に間引く。
pub fn decimate(x: &[f64], factor: usize) -> Vec<f64> {
    if factor <= 1 {
        return x.to_vec();
    }
    let taps = 127usize;
    let half = (taps / 2) as isize;
    let cutoff = 0.45 / factor as f64; // 入力サンプルあたりの周期数
    let h: Vec<f64> = (0..taps)
        .map(|i| {
            let m = i as f64 - half as f64;
            let sinc = if m == 0.0 { 2.0 * cutoff } else { (2.0 * std::f64::consts::PI * cutoff * m).sin() / (std::f64::consts::PI * m) };
            let w = 0.42 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (taps - 1) as f64).cos()
                + 0.08 * (4.0 * std::f64::consts::PI * i as f64 / (taps - 1) as f64).cos();
            sinc * w
        })
        .collect();
    let mut out = Vec::with_capacity(x.len() / factor + 1);
    let mut i = 0usize;
    while i < x.len() {
        let mut acc = 0.0;
        for (k, hk) in h.iter().enumerate() {
            let j = i as isize + k as isize - half;
            if j >= 0 && (j as usize) < x.len() {
                acc += hk * x[j as usize];
            }
        }
        out.push(acc);
        i += factor;
    }
    out
}
