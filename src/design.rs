//! 弦ループのフィルタ設計: 調律（整数遅延 + Thiran 分数遅延）、周波数依存の
//! 損失（1 次 IIR）、分散（同一係数の 1 次オールパスの縦続。ループの共振が
//! 非調和列 f_n = n f0 sqrt(1 + B n^2) に乗るように数値的に合わせ込む）。

use std::f64::consts::PI;

#[derive(Clone, Debug)]
pub struct LoopDesign {
    /// 弦の片道遅延（サンプル数、ナットからブリッジまで）。ループフィルタの
    /// 遅延は含まない。
    pub n_total: usize,
    /// 設計時の基音（Hz）。
    pub f0: f64,
    /// 設計時のサンプリング周波数。
    pub fs: f64,
    /// 1 次 Thiran オールパスで実現する分数遅延。
    pub frac: f64,
    pub thiran_a: f64,
    pub ap_count: usize,
    pub ap_a: f64,
    pub loss_g: f64,
    pub loss_c: f64,
    /// 合わせ込んだ部分音の RMS 誤差と最大誤差（セント）。
    pub rms_cents: f64,
    pub max_cents: f64,
    pub fitted_partials: usize,
}

/// H(z) = (a + z^-1) / (1 + a z^-1) の位相遅れ（ラジアン、0 以上）。
pub fn allpass_lag(a: f64, w: f64) -> f64 {
    let (c, s) = (w.cos(), -w.sin());
    let (nr, ni) = (a + c, s);
    let (dr, di) = (1.0 + a * c, a * s);
    // H = num / den
    let hr = nr * dr + ni * di;
    let hi = ni * dr - nr * di;
    -hi.atan2(hr)
}

/// 1 次ローパス g (1 - c) / (1 - c z^-1) の位相遅れ。
pub fn lowpass_lag(c: f64, w: f64) -> f64 {
    (c * w.sin()).atan2(1.0 - c * w.cos())
}

pub fn lowpass_mag(g: f64, c: f64, w: f64) -> f64 {
    g * (1.0 - c) / (1.0 - 2.0 * c * w.cos() + c * c).sqrt()
}

pub fn thiran_coef(d: f64) -> f64 {
    (1.0 - d) / (1.0 + d)
}

impl LoopDesign {
    /// 角周波数 `w` でのループ 1 周の位相遅れの合計。
    pub fn lag(&self, w: f64) -> f64 {
        w * (2 * self.n_total) as f64
            + allpass_lag(self.thiran_a, w)
            + self.ap_count as f64 * allpass_lag(self.ap_a, w)
            + lowpass_lag(self.loss_c, w)
    }

    /// ループフィルタ（Thiran、分散、損失）が基音で持つ群遅延の合計（サンプル）。
    /// 遅延線上の位置を弦の物理的な位置に換算するときに使う。
    pub fn filter_delay(&self, fs: f64, f0: f64) -> f64 {
        let w0 = 2.0 * PI * f0 / fs;
        (allpass_lag(self.thiran_a, w0) + self.ap_count as f64 * allpass_lag(self.ap_a, w0) + lowpass_lag(self.loss_c, w0)) / w0
    }

    /// ループの n 番目の共振周波数（Hz）を二分法で求める。
    pub fn partial(&self, n: usize, fs: f64, f0: f64) -> Option<f64> {
        let target = 2.0 * PI * n as f64;
        let mut lo = 2.0 * PI * n as f64 * f0 * 0.7 / fs;
        let mut hi = (2.0 * PI * n as f64 * f0 * 1.8 / fs).min(PI * 0.999);
        if self.lag(lo) > target || self.lag(hi) < target {
            return None;
        }
        for _ in 0..60 {
            let mid = 0.5 * (lo + hi);
            if self.lag(mid) < target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Some(0.5 * (lo + hi) * fs / (2.0 * PI))
    }

    /// 第 1 共振が `f0` になるように整数遅延と分数遅延を決める。
    fn tune(&mut self, fs: f64, f0: f64) {
        let w0 = 2.0 * PI * f0 / fs;
        let filt = (self.ap_count as f64 * allpass_lag(self.ap_a, w0) + lowpass_lag(self.loss_c, w0)) / w0;
        let mut req = fs / f0 - filt; // 1 周の遅延から固定フィルタ分を引いたもの（サンプル）
        for _ in 0..4 {
            let n = ((req - 0.3) / 2.0).floor().max(2.0);
            self.n_total = n as usize;
            self.frac = req - 2.0 * n;
            self.thiran_a = thiran_coef(self.frac);
            match self.partial(1, fs, f0) {
                Some(f1) => req += fs / f0 - fs / f1,
                None => break,
            }
        }
    }

    fn error_cents(&self, fs: f64, f0: f64, b: f64, max_partials: usize, f_max: f64) -> (f64, f64, usize) {
        let mut sum = 0.0;
        let mut max = 0.0f64;
        let mut count = 0;
        for n in 2..=max_partials {
            let nf = n as f64;
            let target = nf * f0 * (1.0 + b * nf * nf).sqrt();
            if target > f_max {
                break;
            }
            let got = match self.partial(n, fs, f0) {
                Some(f) => f,
                None => return (1e9, 1e9, count),
            };
            let cents = 1200.0 * (got / target).log2();
            sum += cents * cents;
            max = max.max(cents.abs());
            count += 1;
        }
        if count == 0 {
            return (0.0, 0.0, 0);
        }
        ((sum / count as f64).sqrt(), max, count)
    }
}

/// 部分音ごとの減衰時間に（dB で最小二乗）合わせた損失フィルタ。`targets`
/// は (周波数, T60) の組。1 周はおよそ 1/f0 秒なので、部分音でのループ利得
/// は 10^(-3 / (f0 T60)) でなければならない。`bridge_reflection` は 1 本の弦
/// から見たブリッジ反射の大きさで、ループフィルタは残りの損失だけを受け持つ。
pub fn design_loss(fs: f64, f0: f64, targets: &[(f64, f64)], bridge_reflection: f64) -> (f64, f64) {
    let want: Vec<(f64, f64)> = targets
        .iter()
        .map(|&(f, t60)| (2.0 * PI * f / fs, 20.0 * (10f64.powf(-3.0 / (f0 * t60)) / bridge_reflection).min(0.99999).log10()))
        .collect();
    let mut best = (1.0, 0.0, f64::INFINITY);
    for ci in 0..990 {
        let c = ci as f64 * 0.001;
        // g を除いた形状（dB）
        let shape: Vec<f64> = want.iter().map(|&(w, _)| 20.0 * ((1.0 - c) / (1.0 - 2.0 * c * w.cos() + c * c).sqrt()).log10()).collect();
        let offset = want.iter().zip(&shape).map(|(&(_, d), s)| d - s).sum::<f64>() / want.len() as f64;
        let err: f64 = want.iter().zip(&shape).map(|(&(_, d), s)| (d - s - offset).powi(2)).sum();
        if err < best.2 {
            best = (10f64.powf(offset / 20.0), c, err);
        }
    }
    (best.0, best.1)
}

/// 打鍵した `n_unison` 本の弦の同相振動が `rate_db_s`（負）で減衰するような
/// ブリッジアドミタンス（弦インピーダンス倍）。`n_total` はブリッジにつながる
/// 弦の実効本数（重みの二乗和）。ループフィルタの利得 `loop_gain_db`
/// （負、1 周あたり）が既に取り除く分は差し引く。
pub fn bridge_yz_for_rate(f0: f64, n_unison: f64, n_total: f64, rate_db_s: f64, loop_gain_db: f64) -> f64 {
    let per_trip_db = (rate_db_s / f0 - loop_gain_db).min(-1e-4);
    let r = 10f64.powf(per_trip_db / 20.0); // 求める同相反射の大きさ
    // r = 1 - 2 n_u yz / (1 + n_t yz)  =>  yz = (1 - r) / (2 n_u - n_t (1 - r))
    let q = 1.0 - r;
    (q / (2.0 * n_unison - n_total * q)).clamp(0.0, 0.2)
}

pub struct DesignTargets {
    pub fs: f64,
    pub f0: f64,
    pub inharmonicity: f64,
    /// 損失フィルタ用の部分音ごとの (周波数, T60)。`loss` があれば使わない。
    pub decay: Vec<(f64, f64)>,
    /// 既に設計済みの損失フィルタ (g, c) を流用する場合。
    pub loss: Option<(f64, f64)>,
    pub bridge_reflection: f64,
    pub max_partials: usize,
    pub f_max: f64,
}

/// 設計の全体: 分散オールパスの段数と係数を探索（粗い格子の後、最良点の
/// 周りを細かい格子で）し、調律する。
pub fn design(t: &DesignTargets) -> LoopDesign {
    let (loss_g, loss_c) = t.loss.unwrap_or_else(|| design_loss(t.fs, t.f0, &t.decay, t.bridge_reflection));
    let mut best: Option<LoopDesign> = None;
    let mut candidates: Vec<(usize, f64)> = vec![(0, 0.0)];
    for ap_count in (2..=64usize).step_by(2) {
        for ai in 0..=48 {
            candidates.push((ap_count, -0.98 + ai as f64 * 0.02));
        }
    }
    for pass in 0..2 {
        if pass == 1 {
            let b = best.as_ref().unwrap();
            candidates = Vec::new();
            for ap_count in b.ap_count.saturating_sub(2)..=b.ap_count + 2 {
                for ai in -40..=40 {
                    let a = b.ap_a + ai as f64 * 0.0005;
                    if ap_count >= 1 && a > -0.999 && a <= 0.0 {
                        candidates.push((ap_count, a));
                    }
                }
            }
        }
        for &(ap_count, ap_a) in &candidates {
            let mut d = LoopDesign {
                f0: t.f0,
                fs: t.fs,
                n_total: 0,
                frac: 0.0,
                thiran_a: 0.0,
                ap_count,
                ap_a,
                loss_g,
                loss_c,
                rms_cents: 0.0,
                max_cents: 0.0,
                fitted_partials: 0,
            };
            d.tune(t.fs, t.f0);
            if d.n_total < 4 {
                continue;
            }
            let (rms, max, count) = d.error_cents(t.fs, t.f0, t.inharmonicity, t.max_partials, t.f_max);
            d.rms_cents = rms;
            d.max_cents = max;
            d.fitted_partials = count;
            if best.as_ref().map_or(true, |b| rms < b.rms_cents) {
                best = Some(d);
            }
        }
    }
    best.expect("design search produced no candidate")
}

/// 既存の設計（同じフィルタ）を別の基音に調律し直す。
pub fn retune(base: &LoopDesign, fs: f64, f0: f64) -> LoopDesign {
    let mut d = base.clone();
    d.f0 = f0;
    d.fs = fs;
    d.tune(fs, f0);
    d
}
