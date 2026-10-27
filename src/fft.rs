//! 最小限の基数 2 FFT と FFT 畳み込み。

use std::f64::consts::PI;

pub fn next_pow2(n: usize) -> usize {
    let mut p = 1;
    while p < n {
        p <<= 1;
    }
    p
}

/// その場で行う複素 FFT（`inverse` のとき 1/n を掛ける）。
pub fn fft(re: &mut [f64], im: &mut [f64], inverse: bool) {
    let n = re.len();
    assert!(n.is_power_of_two() && im.len() == n);
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = 2.0 * PI / len as f64 * if inverse { 1.0 } else { -1.0 };
        let (wr, wi) = (ang.cos(), ang.sin());
        let half = len / 2;
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f64, 0.0f64);
            for k in 0..half {
                let (ar, ai) = (re[start + k], im[start + k]);
                let (br, bi) = (re[start + k + half], im[start + k + half]);
                let tr = br * cr - bi * ci;
                let ti = br * ci + bi * cr;
                re[start + k] = ar + tr;
                im[start + k] = ai + ti;
                re[start + k + half] = ar - tr;
                im[start + k + half] = ai - ti;
                let ncr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = ncr;
            }
        }
        len <<= 1;
    }
    if inverse {
        let s = 1.0 / n as f64;
        for i in 0..n {
            re[i] *= s;
            im[i] *= s;
        }
    }
}

/// `x` と `h` の線形畳み込み。長さは `x.len()` に切り詰める。
pub fn convolve(x: &[f64], h: &[f64]) -> Vec<f64> {
    let n = next_pow2(x.len() + h.len());
    let mut xr = vec![0.0; n];
    let mut xi = vec![0.0; n];
    let mut hr = vec![0.0; n];
    let mut hi = vec![0.0; n];
    xr[..x.len()].copy_from_slice(x);
    hr[..h.len()].copy_from_slice(h);
    fft(&mut xr, &mut xi, false);
    fft(&mut hr, &mut hi, false);
    for i in 0..n {
        let r = xr[i] * hr[i] - xi[i] * hi[i];
        let im = xr[i] * hi[i] + xi[i] * hr[i];
        xr[i] = r;
        xi[i] = im;
    }
    fft(&mut xr, &mut xi, true);
    xr.truncate(x.len());
    xr
}

/// Hann 窓を掛けた区間の振幅スペクトル（dB）。`nfft` までゼロ詰めする。
pub fn spectrum_db(seg: &[f64], nfft: usize) -> Vec<f64> {
    let mut re = vec![0.0; nfft];
    let mut im = vec![0.0; nfft];
    let n = seg.len().min(nfft);
    for i in 0..n {
        let w = 0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos();
        re[i] = seg[i] * w;
    }
    fft(&mut re, &mut im, false);
    (0..nfft / 2)
        .map(|i| 10.0 * (re[i] * re[i] + im[i] * im[i] + 1e-30).log10())
        .collect()
}
