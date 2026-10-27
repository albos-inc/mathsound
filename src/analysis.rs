//! 録音または合成した 1 音の測定: 立ち上がり、基音、インハーモニシティ、
//! 部分音ごとの周波数と減衰率。

use crate::fft;
use std::f64::consts::PI;

#[derive(Clone, Debug)]
pub struct Partial {
    pub n: usize,
    pub freq: f64,
    pub peak_db: f64,
    /// 前半・後半の区間での減衰率（dB/s、負）。後半の区間で既にノイズ
    /// フロアに達している部分音では `late` は NaN。
    pub early_db_s: f64,
    pub late_db_s: f64,
}

impl Partial {
    /// 弦ループが再現すべき -60 dB までの減衰時間（秒）。測れる場合は後半
    /// （単弦相当）の減衰、そうでなければ前半の減衰から求める。
    pub fn t60_for_loop(&self) -> f64 {
        let rate = if self.late_db_s.is_nan() || self.late_db_s > -0.3 { self.early_db_s } else { self.late_db_s };
        (-60.0 / rate.min(-0.3)).clamp(0.05, 60.0)
    }
}

#[derive(Clone, Debug)]
pub struct Analysis {
    pub f0: f64,
    pub inharmonicity: f64,
    pub partials: Vec<Partial>,
}

pub fn onset(sig: &[f64], fs: f64) -> usize {
    let peak = sig.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let i = sig.iter().position(|v| v.abs() > 0.01 * peak).unwrap_or(0);
    i.saturating_sub((0.001 * fs) as usize)
}

/// `freq` での複素ヘテロダイン包絡。`window_s` 秒の三角窓（移動平均 2 回）
/// で平滑化する。サンプルごとの (re, im) を返す。
pub fn heterodyne(sig: &[f64], fs: f64, freq: f64, window_s: f64) -> (Vec<f64>, Vec<f64>) {
    let n = sig.len();
    let w = 2.0 * PI * freq / fs;
    let mut re: Vec<f64> = (0..n).map(|i| sig[i] * (w * i as f64).cos()).collect();
    let mut im: Vec<f64> = (0..n).map(|i| -sig[i] * (w * i as f64).sin()).collect();
    let box_len = ((window_s * fs / 2.0).round() as usize).max(1);
    for _ in 0..2 {
        box_filter(&mut re, box_len);
        box_filter(&mut im, box_len);
    }
    (re, im)
}

/// 中心揃えの移動平均（その場で更新）。
fn box_filter(x: &mut Vec<f64>, len: usize) {
    let n = x.len();
    let mut cum = vec![0.0; n + 1];
    for i in 0..n {
        cum[i + 1] = cum[i] + x[i];
    }
    let half = len / 2;
    for i in 0..n {
        let lo = i.saturating_sub(half);
        let hi = (i + len - half).min(n);
        x[i] = (cum[hi] - cum[lo]) / (hi - lo) as f64;
    }
}

fn slope_db_per_s(env_db: &[f64], fs: f64, t0: f64, t1: f64) -> f64 {
    let i0 = (t0 * fs) as usize;
    let i1 = ((t1 * fs) as usize).min(env_db.len());
    if i1 <= i0 + 10 {
        return f64::NAN;
    }
    let (mut sx, mut sy, mut sxx, mut sxy, mut n) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for i in (i0..i1).step_by(16) {
        let x = i as f64 / fs;
        let y = env_db[i];
        sx += x;
        sy += y;
        sxx += x * x;
        sxy += x * y;
        n += 1.0;
    }
    (n * sxy - sx * sy) / (n * sxx - sx * sx)
}

/// `sig`（先頭が立ち上がりに揃っていること）を解析する。
pub fn analyze(sig: &[f64], fs: f64, f0_guess: f64, max_partials: usize, f_max: f64) -> Analysis {
    let start = (0.02 * fs) as usize;
    let len = ((1.0 * fs) as usize).min(sig.len().saturating_sub(start));
    let nfft = fft::next_pow2(len).max(1 << 18);
    let spec = fft::spectrum_db(&sig[start..start + len], nfft);
    let bin_hz = fs / nfft as f64;
    let peak_near = |center: f64, halfwidth: f64| -> Option<(f64, f64)> {
        let lo = ((center - halfwidth) / bin_hz).max(1.0) as usize;
        let hi = (((center + halfwidth) / bin_hz) as usize).min(spec.len() - 2);
        if hi <= lo {
            return None;
        }
        let mut best = lo;
        for i in lo..=hi {
            if spec[i] > spec[best] {
                best = i;
            }
        }
        // 放物線補間
        let (a, b, c) = (spec[best - 1], spec[best], spec[best + 1]);
        let denom = a - 2.0 * b + c;
        let delta = if denom.abs() > 1e-12 { 0.5 * (a - c) / denom } else { 0.0 };
        Some(((best as f64 + delta) * bin_hz, b - 0.25 * (a - c) * delta))
    };
    let (f0, p0_db) = peak_near(f0_guess, 0.06 * f0_guess).unwrap_or((f0_guess, 0.0));
    let mut freqs = vec![(1usize, f0, p0_db)];
    let mut b_est = 0.0;
    let mut num = 0.0;
    let mut den = 0.0;
    for n in 2..=max_partials {
        let nf = n as f64;
        let pred = nf * f0 * (1.0 + b_est * nf * nf).sqrt();
        if pred > f_max {
            break;
        }
        match peak_near(pred, 0.3 * f0) {
            Some((f, db)) => {
                freqs.push((n, f, db));
                let y = (f / (nf * f0)).powi(2) - 1.0;
                num += nf * nf * y;
                den += nf.powi(4);
                if den > 0.0 {
                    b_est = (num / den).max(0.0);
                }
            }
            None => break,
        }
    }
    let partials = freqs
        .iter()
        .map(|&(n, f, db)| {
            let (re, im) = heterodyne(sig, fs, f, 0.05);
            let env_db: Vec<f64> = re.iter().zip(&im).map(|(r, i)| 10.0 * (r * r + i * i + 1e-30).log10()).collect();
            let tail = env_db.len().saturating_sub((0.3 * fs) as usize);
            let floor = env_db[tail..].iter().sum::<f64>() / (env_db.len() - tail).max(1) as f64;
            let at = |t: f64| env_db[((t * fs) as usize).min(env_db.len() - 1)];
            let late_ok = env_db.len() > (4.0 * fs) as usize
                && at(1.5) > floor + 15.0
                && (at(4.0) - floor) > 0.3 * (at(1.5) - floor);
            Partial {
                n,
                freq: f,
                peak_db: db,
                early_db_s: slope_db_per_s(&env_db, fs, 0.1, 0.8),
                late_db_s: if late_ok { slope_db_per_s(&env_db, fs, 1.5, 4.0) } else { f64::NAN },
            }
        })
        .collect();
    Analysis { f0, inharmonicity: b_est, partials }
}

/// `freq` の部分音の包絡（dB）を `step_s` 秒ごとに返す。
pub fn envelope_trace(sig: &[f64], fs: f64, freq: f64, step_s: f64, until_s: f64) -> Vec<f64> {
    let (re, im) = heterodyne(sig, fs, freq, 0.05);
    let step = (step_s * fs) as usize;
    (0..((until_s * fs) as usize).min(sig.len()))
        .step_by(step.max(1))
        .map(|i| 10.0 * (re[i] * re[i] + im[i] * im[i] + 1e-30).log10())
        .collect()
}

/// 時間範囲の RMS。
pub fn rms(sig: &[f64], fs: f64, t0: f64, t1: f64) -> f64 {
    let i0 = ((t0 * fs) as usize).min(sig.len());
    let i1 = ((t1 * fs) as usize).min(sig.len());
    if i1 <= i0 {
        return 0.0;
    }
    (sig[i0..i1].iter().map(|v| v * v).sum::<f64>() / (i1 - i0) as f64).sqrt()
}
