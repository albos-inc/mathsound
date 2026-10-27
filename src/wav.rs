//! 小さな WAV 読み込み（PCM 16/24/32、float 32）と 16 ビット書き出し。

use std::fs;
use std::io;
use std::path::Path;

pub struct Wav {
    pub sample_rate: u32,
    pub channels: Vec<Vec<f64>>,
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

pub fn read(path: impl AsRef<Path>) -> io::Result<Wav> {
    let b = fs::read(path)?;
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err(bad("not a RIFF/WAVE file"));
    }
    let mut pos = 12;
    let (mut fmt_tag, mut nch, mut rate, mut bits) = (0u16, 0usize, 0u32, 0u16);
    let mut data: Option<&[u8]> = None;
    while pos + 8 <= b.len() {
        let id = &b[pos..pos + 4];
        let size = u32_at(&b, pos + 4) as usize;
        let body_end = (pos + 8 + size).min(b.len());
        let body = &b[pos + 8..body_end];
        if id == b"fmt " {
            fmt_tag = u16_at(body, 0);
            nch = u16_at(body, 2) as usize;
            rate = u32_at(body, 4);
            bits = u16_at(body, 14);
            if fmt_tag == 0xFFFE && body.len() >= 26 {
                fmt_tag = u16_at(body, 24);
            }
        } else if id == b"data" {
            data = Some(body);
        }
        pos = pos + 8 + size + (size & 1);
    }
    let data = data.ok_or_else(|| bad("no data chunk"))?;
    if nch == 0 {
        return Err(bad("no fmt chunk"));
    }
    let bytes = (bits / 8) as usize;
    let frames = data.len() / (bytes * nch);
    let mut channels = vec![Vec::with_capacity(frames); nch];
    for f in 0..frames {
        for c in 0..nch {
            let i = (f * nch + c) * bytes;
            let v = match (fmt_tag, bits) {
                (1, 16) => i16::from_le_bytes([data[i], data[i + 1]]) as f64 / 32768.0,
                (1, 24) => {
                    let v = i32::from_le_bytes([0, data[i], data[i + 1], data[i + 2]]) >> 8;
                    v as f64 / 8388608.0
                }
                (1, 32) => u32_at(data, i) as i32 as f64 / 2147483648.0,
                (3, 32) => f32::from_bits(u32_at(data, i)) as f64,
                _ => return Err(bad("unsupported WAV sample format")),
            };
            channels[c].push(v);
        }
    }
    Ok(Wav { sample_rate: rate, channels })
}

pub fn write16(path: impl AsRef<Path>, sample_rate: u32, channels: &[&[f64]]) -> io::Result<()> {
    let nch = channels.len();
    let frames = channels.iter().map(|c| c.len()).min().unwrap_or(0);
    let data_len = frames * nch * 2;
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(nch as u16).to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * nch as u32 * 2).to_le_bytes());
    out.extend_from_slice(&((nch * 2) as u16).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for f in 0..frames {
        for ch in channels {
            let v = (ch[f].clamp(-1.0, 1.0) * 32767.0).round() as i16;
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    fs::write(path, out)
}
