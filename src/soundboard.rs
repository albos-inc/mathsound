//! 録音した 1 音から同定する響板・胴体の応答。
//!
//! 二つの部分からなる。録音の部分音レベルとモデル弦の部分音レベルの比を
//! 平滑化した振幅を持つ最小位相フィルタ（ブリッジ→響板→マイクの線形経路）
//! と、録音から部分音を除いた残差（ハンマーの打撃音、響板のノック音）を
//! 短い加算アタックとして保持したもの。

use crate::analysis::{heterodyne, Analysis};
use crate::fft;
use std::f64::consts::PI;

/// `sig`（先頭が立ち上がり）の最初の `len_s` 秒から `freqs` の部分音を引いた
/// 残差。`flat_s` 以降はフェードアウトする。
pub fn residual(sig: &[f64], fs: f64, freqs: &[f64], len_s: f64, flat_s: f64) -> Vec<f64> {
    let seg_len = ((len_s + 0.2) * fs) as usize;
    let seg = &sig[..seg_len.min(sig.len())];
    let mut res = seg.to_vec();
    for &f in freqs {
        let (re, im) = heterodyne(seg, fs, f, 0.02);
        let w = 2.0 * PI * f / fs;
        for i in 0..seg.len() {
            let (c, s) = ((w * i as f64).cos(), (w * i as f64).sin());
            res[i] -= 2.0 * (re[i] * c - im[i] * s);
        }
    }
    let n = (len_s * fs) as usize;
    let flat = (flat_s * fs) as usize;
    res.truncate(n);
    for i in flat..res.len() {
        let x = (i - flat) as f64 / (res.len() - flat) as f64;
        res[i] *= 0.5 + 0.5 * (PI * x).cos();
    }
    res
}

/// 部分音ごとのレベル差（録音 − モデル、dB）。`smooth_from_hz` より上では
/// 隣接 `smooth` 個の部分音で平滑化する（それより下では響板自身のモードの
/// せいで隣り合う部分音が 10 dB 以上違うので、測定値のまま使う）。
/// (周波数, dB) の組を返す。
pub fn level_ratio(recorded: &Analysis, model: &Analysis, smooth: usize, smooth_from_hz: f64) -> Vec<(f64, f64)> {
    let raw: Vec<(f64, f64)> = recorded
        .partials
        .iter()
        .filter_map(|p| model.partials.iter().find(|q| q.n == p.n).map(|q| (p.freq, p.peak_db - q.peak_db)))
        .collect();
    let half = smooth / 2;
    (0..raw.len())
        .map(|i| {
            let half = if raw[i].0 < smooth_from_hz { 0 } else { half };
            let lo = i.saturating_sub(half);
            let hi = (i + half + 1).min(raw.len());
            let mean = raw[lo..hi].iter().map(|r| r.1).sum::<f64>() / (hi - lo) as f64;
            (raw[i].0, mean)
        })
        .collect()
}

/// 振幅が `points`（周波数 Hz, dB）に従う最小位相インパルス応答（長さ
/// `taps`）。最初の点より下は平坦、最後の点より上は 6 dB/oct で下がる。
/// 平均レベルから `clamp_db` を超える偏差は制限する。
pub fn min_phase_eq(points: &[(f64, f64)], fs: f64, nfft: usize, taps: usize, clamp_db: f64) -> Vec<f64> {
    let mean = if points.is_empty() { 0.0 } else { points.iter().map(|p| p.1).sum::<f64>() / points.len() as f64 };
    let bins = nfft / 2 + 1;
    let mut log_mag = vec![0.0; nfft];
    for k in 0..bins {
        let f = k as f64 * fs / nfft as f64;
        let db = if points.is_empty() {
            0.0
        } else if f <= points[0].0 {
            points[0].1
        } else if f >= points[points.len() - 1].0 {
            let (fl, dl) = points[points.len() - 1];
            dl - 6.0 * (f / fl).log2()
        } else {
            let j = points.iter().position(|p| p.0 > f).unwrap();
            let (fa, da) = points[j - 1];
            let (fb, db) = points[j];
            let x = (f / fa).ln() / (fb / fa).ln();
            da + x * (db - da)
        };
        let db = mean + (db - mean).clamp(-clamp_db, clamp_db);
        log_mag[k] = db / 20.0 * std::f64::consts::LN_10;
    }
    for k in bins..nfft {
        log_mag[k] = log_mag[nfft - k];
    }
    let mut re = log_mag;
    let mut im = vec![0.0; nfft];
    fft::fft(&mut re, &mut im, true); // 実ケプストラム
    for k in 1..nfft / 2 {
        re[k] *= 2.0;
    }
    for k in nfft / 2 + 1..nfft {
        re[k] = 0.0;
    }
    im.iter_mut().for_each(|v| *v = 0.0);
    fft::fft(&mut re, &mut im, false);
    for k in 0..nfft {
        let mag = re[k].exp();
        let ph = im[k];
        re[k] = mag * ph.cos();
        im[k] = mag * ph.sin();
    }
    fft::fft(&mut re, &mut im, true);
    re.truncate(taps);
    re
}
