//! 物理モデルで作ったピアノ 1 音（C4）を複数のハンマー速度で描画し、
//! 同じ音の Salamander Grand Piano 録音と並べて、聴き比べと測定による比較
//! （部分音の周波数・レベル・減衰率）ができるようにする。打弦に加えて、
//! 離鍵（ダンパー）とペダル踏下時の共鳴も V3 の付属サンプルと比較する。

mod analysis;
mod design;
mod fft;
mod model;
mod soundboard;
mod wav;

use model::{Event, KeyStrings, Piano};
use std::path::PathBuf;

struct Args {
    sal_dir: PathBuf,
    out_dir: PathBuf,
    /// (Salamander のベロシティ層, 代表 MIDI ベロシティ)
    layers: Vec<(u32, u32)>,
    ref_layer: u32,
    seconds: f64,
    oversample: usize,
    detune: [f64; 3],
    yz: Option<f64>,
    z: f64,
    strike: f64,
    hammer_mass: f64,
    hammer_k: f64,
    hammer_p: f64,
    hammer_eps: f64,
    hammer_tau: f64,
    vel_scale: f64,
    vel_exp: f64,
    use_eq: bool,
    use_thump: bool,
    thump_gain: f64,
    thump_len: f64,
    inharmonicity: Option<f64>,
    eq_smooth: usize,
    eq_smooth_from_hz: f64,
    trace: Vec<usize>,
    summary_only: bool,
    /// 共鳴弦（他の 87 鍵）をブリッジにつなぐか。
    sympathetic: bool,
    bass_weight: f64,
    symp_weight: f64,
    symp_strings: usize,
    damper_rho: f64,
    damper_ms: f64,
    damper_pos: f64,
    release_at: f64,
    release_noise: bool,
}

fn parse_args() -> Args {
    let mut a = Args {
        sal_dir: "salamander".into(),
        out_dir: "out".into(),
        layers: vec![(4, 40), (8, 60), (14, 108)],
        ref_layer: 8,
        seconds: 6.0,
        oversample: 2,
        detune: [0.0, 0.3, 1.2],
        yz: None,
        z: 2.0,
        strike: 0.12,
        hammer_mass: 6.0e-3,
        hammer_k: 2.0e9,
        hammer_p: 2.5,
        hammer_eps: 0.0,
        hammer_tau: 100e-6,
        vel_scale: 0.0064,
        vel_exp: 1.35,
        use_eq: true,
        use_thump: true,
        thump_gain: 1.0,
        thump_len: 0.3,
        inharmonicity: None,
        eq_smooth: 3,
        eq_smooth_from_hz: 1500.0,
        trace: Vec::new(),
        summary_only: false,
        sympathetic: true,
        bass_weight: 0.3,
        symp_weight: 0.1,
        symp_strings: 3,
        damper_rho: 0.06,
        damper_ms: 25.0,
        damper_pos: 0.22,
        release_at: 1.0,
        release_noise: false,
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    let next = |i: &mut usize| -> String {
        *i += 1;
        argv.get(*i).cloned().unwrap_or_else(|| panic!("missing value for {}", argv[*i - 1]))
    };
    while i < argv.len() {
        match argv[i].as_str() {
            "--sal-dir" => a.sal_dir = next(&mut i).into(),
            "--out" => a.out_dir = next(&mut i).into(),
            "--seconds" => a.seconds = next(&mut i).parse().unwrap(),
            "--oversample" => a.oversample = next(&mut i).parse().unwrap(),
            "--detune" => {
                let v: Vec<f64> = next(&mut i).split(',').map(|s| s.parse().unwrap()).collect();
                a.detune = [v[0], v[1], v[2]];
            }
            "--yz" => a.yz = Some(next(&mut i).parse().unwrap()),
            "--z" => a.z = next(&mut i).parse().unwrap(),
            "--strike" => a.strike = next(&mut i).parse().unwrap(),
            "--hammer-mass" => a.hammer_mass = next(&mut i).parse().unwrap(),
            "--hammer-k" => a.hammer_k = next(&mut i).parse().unwrap(),
            "--hammer-p" => a.hammer_p = next(&mut i).parse().unwrap(),
            "--hammer-eps" => a.hammer_eps = next(&mut i).parse().unwrap(),
            "--hammer-tau-us" => a.hammer_tau = next(&mut i).parse::<f64>().unwrap() * 1e-6,
            "--vel-scale" => a.vel_scale = next(&mut i).parse().unwrap(),
            "--vel-exp" => a.vel_exp = next(&mut i).parse().unwrap(),
            "--no-eq" => a.use_eq = false,
            "--no-thump" => a.use_thump = false,
            "--thump-gain" => a.thump_gain = next(&mut i).parse().unwrap(),
            "--thump-len" => a.thump_len = next(&mut i).parse().unwrap(),
            "--ref-layer" => a.ref_layer = next(&mut i).parse().unwrap(),
            "--b" => a.inharmonicity = Some(next(&mut i).parse().unwrap()),
            "--eq-smooth" => a.eq_smooth = next(&mut i).parse().unwrap(),
            "--eq-smooth-from" => a.eq_smooth_from_hz = next(&mut i).parse().unwrap(),
            "--summary" => a.summary_only = true,
            "--trace" => a.trace = next(&mut i).split(',').map(|s| s.parse().unwrap()).collect(),
            "--no-symp" => a.sympathetic = false,
            "--bass-weight" => a.bass_weight = next(&mut i).parse().unwrap(),
            "--symp-weight" => a.symp_weight = next(&mut i).parse().unwrap(),
            "--symp-strings" => a.symp_strings = next(&mut i).parse().unwrap(),
            "--damper-rho" => a.damper_rho = next(&mut i).parse().unwrap(),
            "--damper-ms" => a.damper_ms = next(&mut i).parse().unwrap(),
            "--damper-pos" => a.damper_pos = next(&mut i).parse().unwrap(),
            "--release-at" => a.release_at = next(&mut i).parse().unwrap(),
            "--release-noise" => a.release_noise = true,
            "--layers" => {
                a.layers = next(&mut i)
                    .split(',')
                    .map(|s| {
                        let (l, v) = s.split_once(':').expect("layer:velocity");
                        (l.parse().unwrap(), v.parse().unwrap())
                    })
                    .collect();
            }
            other => panic!("unknown argument {other}"),
        }
        i += 1;
    }
    a
}

fn load_mono(path: &PathBuf) -> (f64, Vec<f64>) {
    let w = wav::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let n = w.channels[0].len();
    let nch = w.channels.len() as f64;
    let mono = (0..n).map(|i| w.channels.iter().map(|c| c[i]).sum::<f64>() / nch).collect();
    (w.sample_rate as f64, mono)
}

fn fmt_rate(v: f64) -> String {
    if v.is_nan() { "    n/a".to_string() } else { format!("{v:7.1}") }
}

fn print_analysis(label: &str, an: &analysis::Analysis, reference: Option<&analysis::Analysis>) {
    println!("{label}: f0 = {:.2} Hz, B = {:.2e}, partials = {}", an.f0, an.inharmonicity, an.partials.len());
    println!("   n     freq(Hz)  level(dB)  early(dB/s)  late(dB/s)   vs Salamander: cents  level  early  late");
    for p in &an.partials {
        let cmp = reference
            .and_then(|r| r.partials.iter().find(|q| q.n == p.n))
            .map(|q| {
                format!(
                    "{:+6.1}c {:+6.1} {:+6.1} {:+6.1}",
                    1200.0 * (p.freq / q.freq).log2(),
                    p.peak_db - q.peak_db,
                    p.early_db_s - q.early_db_s,
                    p.late_db_s - q.late_db_s
                )
            })
            .unwrap_or_default();
        if p.n <= 12 || p.n % 4 == 0 {
            println!(
                "  {:2}  {:10.2}  {:8.1}  {}  {}   {cmp}",
                p.n,
                p.freq,
                p.peak_db,
                fmt_rate(p.early_db_s),
                fmt_rate(p.late_db_s)
            );
        }
    }
}

fn envelope_line(sig: &[f64], fs: f64, freq: f64, step: f64, until: f64) -> String {
    analysis::envelope_trace(sig, fs, freq, step, until).iter().map(|d| format!("{d:4.0}")).collect::<Vec<_>>().join(" ")
}

/// `--analyze path [f0]`: 任意の WAV を解析して部分音表と包絡を表示する。
fn analyze_file(path: &str, f0_guess: f64) {
    let (fs, sig) = load_mono(&PathBuf::from(path));
    let on = analysis::onset(&sig, fs);
    let note = &sig[on..];
    let an = analysis::analyze(note, fs, f0_guess, 24, 8000.0);
    println!("{path}: onset at {:.3} s, {:.2} s long", on as f64 / fs, note.len() as f64 / fs);
    print_analysis("Analysis", &an, None);
    for p in an.partials.iter().take(4) {
        println!("partial {} ({:.0} Hz) dB every 50 ms: {}", p.n, p.freq, envelope_line(note, fs, p.freq, 0.05, 2.0));
    }
}

/// 鍵ごとの弦の設計と重み。
struct KeyDesign {
    key: u8,
    weight: f64,
    has_damper: bool,
    designs: Vec<design::LoopDesign>,
}

struct Scene {
    args_vel_scale: f64,
    fs: f64,
    fs_model: f64,
    oversample: usize,
    z: f64,
    yz: f64,
    strike: f64,
    damper_pos: f64,
    hammer: model::Hammer,
    damper: model::Damper,
    keys: Vec<KeyDesign>,
}

struct Rendered {
    dry: Vec<f64>,
    peak_force: f64,
    contact_ms: f64,
    force_trace: Vec<f64>,
    /// 描画終了時に鍵ごとに残っているエネルギー。
    energies: Vec<(u8, f64)>,
}

impl Scene {
    fn velocity(&self, midi: u32, vel_exp: f64) -> f64 {
        self.args_vel_scale * (midi as f64).powf(vel_exp)
    }

    /// 新しいピアノを組み立てて `events`（秒単位）を描画し、48 kHz のブリッジ力を返す。
    fn render(&self, seconds: f64, events: &[(f64, Event)], pre: usize) -> Rendered {
        let keys: Vec<KeyStrings> = self
            .keys
            .iter()
            .map(|k| {
                KeyStrings::new(k.key, k.weight, k.has_damper, 2f64.powf((60.0 - k.key as f64) / 12.0), &k.designs, self.strike, self.damper_pos)
            })
            .collect();
        let mut piano = Piano::new(self.fs_model, self.z, self.yz, self.hammer.clone(), self.damper.clone(), keys);
        let ev: Vec<(usize, Event)> = events.iter().map(|(t, e)| ((t * self.fs_model) as usize, e.clone())).collect();
        let bridge = piano.render((seconds * self.fs_model) as usize, &ev);
        let total = (seconds * self.fs) as usize;
        let mut dry = vec![0.0; pre];
        dry.extend(model::decimate(&bridge, self.oversample));
        dry.truncate(total);
        dry.resize(total, 0.0);
        let energies = piano.key_energies();
        Rendered {
            dry,
            peak_force: piano.peak_force,
            contact_ms: 1000.0 * piano.contact_samples as f64 / self.fs_model,
            force_trace: piano.force_trace,
            energies,
        }
    }
}

/// 響板 EQ と打撃音を掛けて録音の単位に揃える。
fn post(dry: &[f64], eq: Option<&Vec<f64>>, thump: Option<&Vec<f64>>, thump_gain: f64, pre: usize) -> Vec<f64> {
    let mut wet = match eq {
        Some(eq) => fft::convolve(dry, eq),
        None => dry.to_vec(),
    };
    if let Some(t) = thump {
        for (i, v) in t.iter().enumerate() {
            if pre + i < wet.len() {
                wet[pre + i] += thump_gain * v;
            }
        }
    }
    wet
}

fn write_ab(path: PathBuf, fs: f64, first: &[f64], second: &[f64], clip_s: f64) {
    let clip = (clip_s * fs) as usize;
    let gap = vec![0.0; (0.5 * fs) as usize];
    let mut ab = first[..clip.min(first.len())].to_vec();
    ab.extend_from_slice(&gap);
    ab.extend_from_slice(&second[..clip.min(second.len())]);
    let peak = ab.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    if peak > 0.95 {
        ab.iter_mut().for_each(|v| *v *= 0.95 / peak);
    }
    wav::write16(path, fs as u32, &[&ab]).unwrap();
}

fn write_norm(path: PathBuf, fs: f64, sig: &[f64]) {
    let peak = sig.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let g = if peak > 0.95 { 0.95 / peak } else { 1.0 };
    let v: Vec<f64> = sig.iter().map(|x| x * g).collect();
    wav::write16(path, fs as u32, &[&v]).unwrap();
}

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    if argv.get(1).map(|s| s.as_str()) == Some("--analyze") {
        let f0 = argv.get(3).and_then(|s| s.parse().ok()).unwrap_or(261.63);
        analyze_file(&argv[2], f0);
        return;
    }
    let args = parse_args();
    std::fs::create_dir_all(&args.out_dir).unwrap();
    let f_max = 10_000.0;
    let max_partials = 48;
    let pre_s = 0.01; // 出力ファイル先頭の無音
    let played_key = 60u8;

    // --- 基準録音から弦パラメータを同定 --------------------------------
    let (fs, ref_sig) = load_mono(&args.sal_dir.join(format!("C4v{}.wav", args.ref_layer)));
    let onset = analysis::onset(&ref_sig, fs);
    let ref_note = &ref_sig[onset..];
    let ref_an = analysis::analyze(ref_note, fs, 261.63, max_partials, f_max);
    print_analysis(&format!("Salamander C4v{}", args.ref_layer), &ref_an, None);

    let f0 = ref_an.f0;
    let b = args.inharmonicity.unwrap_or(ref_an.inharmonicity);
    let decay: Vec<(f64, f64)> = ref_an.partials.iter().map(|p| (p.freq, p.t60_for_loop())).collect();
    let early_rate = 0.5 * (ref_an.partials[0].early_db_s + ref_an.partials[1].early_db_s);
    let fs_model = fs * args.oversample as f64;
    let w0 = 2.0 * std::f64::consts::PI * f0 / fs_model;
    let pre = (pre_s * fs) as usize;

    // ブリッジにつながる弦: 打鍵する鍵は 3 本、他の鍵は 1 本ずつ。
    let key_plan: Vec<(u8, f64, bool, usize)> = (21u8..=108)
        .filter(|&k| k == played_key || args.sympathetic)
        .map(|k| {
            let weight = if k == played_key { 1.0 } else if k < 42 { args.symp_weight * args.bass_weight } else { args.symp_weight };
            let strings = if k == played_key { 3 } else { args.symp_strings };
            (k, weight, k < 89, strings)
        })
        .collect();
    let n_total = 3.0; // ユニゾン弦は自分の取り付け点のアドミタンスだけを見る

    // ブリッジ結合とループ損失は互いに依存するので反復で決める。
    let mut yz = args.yz.unwrap_or(0.002);
    let mut loss = (1.0, 0.0);
    for _ in 0..4 {
        let bridge_reflection = 1.0 - 2.0 * 3.0 * yz / (1.0 + n_total * yz);
        loss = design::design_loss(fs_model, f0, &decay, bridge_reflection);
        if args.yz.is_none() {
            let loop_db = 20.0 * design::lowpass_mag(loss.0, loss.1, w0).log10();
            yz = design::bridge_yz_for_rate(f0, 3.0, n_total, early_rate, loop_db);
        }
    }
    let bridge_reflection = 1.0 - 2.0 * 3.0 * yz / (1.0 + n_total * yz);
    println!(
        "\nString targets: f0 {:.2} Hz, B {:.2e}; loop T60 {:.1} s (partial 1) .. {:.2} s (partial {}); in-phase early decay {:.1} dB/s -> Y_b Z = {:.4}; {} keys with {} strings each coupled at weight {}",
        f0,
        b,
        decay[0].1,
        decay[decay.len() - 1].1,
        ref_an.partials.len(),
        early_rate,
        yz,
        key_plan.len() - 1,
        args.symp_strings,
        args.symp_weight
    );
    let base = design::design(&design::DesignTargets {
        fs: fs_model,
        f0,
        inharmonicity: b,
        decay: decay.clone(),
        loss: None,
        bridge_reflection,
        max_partials,
        f_max,
    });
    println!(
        "Loop at {} Hz: one-way {} + {:.3} samples, {} allpass (a = {:.4}), loss g = {:.5} c = {:.4}; {} partials fitted, rms {:.2} cents, max {:.2} cents",
        fs_model, base.n_total, base.frac, base.ap_count, base.ap_a, loss.0, loss.1, base.fitted_partials, base.rms_cents, base.max_cents
    );

    // 鍵ごとの設計。打鍵する鍵はユニゾン 3 本、他は平均律 + インハーモニシティの推定値。
    let t_design = std::time::Instant::now();
    let mut worst = (0u8, 0.0f64);
    let keys: Vec<KeyDesign> = key_plan
        .iter()
        .map(|&(k, weight, has_damper, _)| {
            let designs = if k == played_key {
                args.detune.iter().map(|c| design::retune(&base, fs_model, f0 * 2f64.powf(c / 1200.0))).collect()
            } else {
                let fk = f0 * 2f64.powf((k as f64 - played_key as f64) / 12.0);
                let bk = (b * 2f64.powf((k as f64 - played_key as f64) / 12.0 * 1.5)).max(1e-4);
                let d = design::design(&design::DesignTargets {
                    fs: fs_model,
                    f0: fk,
                    inharmonicity: bk,
                    decay: Vec::new(),
                    loss: Some(loss),
                    bridge_reflection,
                    max_partials: 24,
                    f_max: 8000.0,
                });
                if d.rms_cents > worst.1 {
                    worst = (k, d.rms_cents);
                }
                args.detune.iter().take(args.symp_strings).map(|c| design::retune(&d, fs_model, fk * 2f64.powf(c / 1200.0))).collect()
            };
            KeyDesign { key: k, weight, has_damper, designs }
        })
        .collect();
    if args.sympathetic {
        println!(
            "Sympathetic strings: {} keys designed in {:.1} s; worst dispersion fit key {} ({:.2} cents rms)",
            keys.len() - 1,
            t_design.elapsed().as_secs_f64(),
            worst.0,
            worst.1
        );
    }
    let scene = Scene {
        args_vel_scale: args.vel_scale,
        fs,
        fs_model,
        oversample: args.oversample,
        z: args.z,
        yz,
        strike: args.strike,
        damper_pos: args.damper_pos,
        hammer: model::Hammer {
            mass: args.hammer_mass,
            stiffness: args.hammer_k,
            exponent: args.hammer_p,
            eps: args.hammer_eps,
            tau: args.hammer_tau,
        },
        damper: model::Damper { rho: args.damper_rho, ramp_s: args.damper_ms * 1e-3 },
        keys,
    };
    let note_on = |midi: u32| Event::NoteOn { key: played_key, velocity: scene.velocity(midi, args.vel_exp) };

    // --- 打弦: 各層のドライ出力 -----------------------------------------
    let total = (args.seconds * fs) as usize;
    let mut strikes: Vec<(u32, u32, Vec<f64>, Rendered)> = Vec::new();
    for &(layer, vel) in &args.layers {
        let (fs_l, sal) = load_mono(&args.sal_dir.join(format!("C4v{layer}.wav")));
        assert_eq!(fs_l, fs);
        let on = analysis::onset(&sal, fs);
        let mut sal_cut = vec![0.0; pre];
        sal_cut.extend_from_slice(&sal[on..(on + total - pre).min(sal.len())]);
        sal_cut.resize(total, 0.0);
        let r = scene.render(args.seconds, &[(0.0, note_on(vel))], pre);
        strikes.push((layer, vel, sal_cut, r));
    }

    // --- 響板: 基準層から EQ を、その残差から打撃音を得る ---------------
    let ref_idx = strikes.iter().position(|s| s.0 == args.ref_layer).expect("reference layer must be rendered");
    let ref_dry_an = analysis::analyze(&strikes[ref_idx].3.dry[pre..], fs, f0, max_partials, f_max);
    let ratio = soundboard::level_ratio(&ref_an, &ref_dry_an, 1, 0.0);
    println!("\nRecording minus dry model, per partial (dB; what the soundboard EQ has to supply, before smoothing):");
    let line: Vec<String> = ratio.iter().map(|(f, d)| format!("{:.0}:{:+.0}", f, d)).collect();
    println!("  {}", line.join("  "));
    let eq = if args.use_eq {
        let smooth = soundboard::level_ratio(&ref_an, &ref_dry_an, args.eq_smooth, args.eq_smooth_from_hz);
        Some(soundboard::min_phase_eq(&smooth, fs, 8192, 4096, 40.0))
    } else {
        None
    };
    let thump = if args.use_thump {
        let freqs: Vec<f64> = ref_an.partials.iter().map(|p| p.freq).collect();
        let t = soundboard::residual(ref_note, fs, &freqs, args.thump_len, 0.12);
        wav::write16(args.out_dir.join("thump_residual.wav"), fs as u32, &[&t]).unwrap();
        let e_res = analysis::rms(&t, fs, 0.0, 0.1);
        let e_sig = analysis::rms(ref_note, fs, 0.0, 0.1);
        println!("Thump residual: {:.1} dB below the recording over the first 100 ms", 20.0 * (e_sig / e_res.max(1e-12)).log10());
        Some(t)
    } else {
        None
    };
    let ref_force = strikes[ref_idx].3.peak_force;
    let thump_gain = |peak_force: f64| args.thump_gain * peak_force / ref_force;

    // --- 打弦の出力ファイルと比較 ----------------------------------------
    for (layer, vel, sal_cut, r) in &strikes {
        let mut wet = post(&r.dry, eq.as_ref(), thump.as_ref(), thump_gain(r.peak_force), pre);
        let sal_rms = analysis::rms(sal_cut, fs, 0.0, 1.0);
        let wet_rms = analysis::rms(&wet, fs, 0.0, 1.0).max(1e-12);
        let dry_rms = analysis::rms(&r.dry, fs, 0.0, 1.0).max(1e-12);
        let dry_scaled: Vec<f64> = r.dry.iter().map(|v| v * sal_rms / dry_rms).collect();
        if eq.is_none() {
            wet = wet.iter().map(|v| v * sal_rms / wet_rms).collect();
        }
        write_norm(args.out_dir.join(format!("salamander_C4_v{layer}.wav")), fs, sal_cut);
        write_norm(args.out_dir.join(format!("model_C4_v{layer}.wav")), fs, &wet);
        write_norm(args.out_dir.join(format!("model_C4_v{layer}_dry.wav")), fs, &dry_scaled);
        write_ab(args.out_dir.join(format!("ab_C4_v{layer}.wav")), fs, sal_cut, &wet, 4.0);

        let sal_an = analysis::analyze(&sal_cut[pre..], fs, f0, max_partials, f_max);
        let model_an = analysis::analyze(&wet[pre..], fs, f0, max_partials, f_max);
        println!(
            "\n=== layer v{layer}: MIDI velocity {vel}, hammer {:.2} m/s, contact {:.2} ms, peak force {:.1} N, level vs recording {:+.1} dB (first second) ===",
            scene.velocity(*vel, args.vel_exp),
            r.contact_ms,
            r.peak_force,
            20.0 * (wet_rms / sal_rms).log10()
        );
        let band = |lo: usize, hi: usize| -> f64 {
            let diffs: Vec<f64> = model_an
                .partials
                .iter()
                .filter(|p| p.n >= lo && p.n <= hi)
                .filter_map(|p| sal_an.partials.iter().find(|q| q.n == p.n).map(|q| p.peak_db - q.peak_db))
                .collect();
            if diffs.is_empty() { f64::NAN } else { diffs.iter().sum::<f64>() / diffs.len() as f64 }
        };
        println!(
            "Level vs recording (dB, mean over partials): 1-6 {:+.1}, 7-15 {:+.1}, 16-32 {:+.1}; early decay of partial 1: model {:.1} vs {:.1} dB/s",
            band(1, 6),
            band(7, 15),
            band(16, 32),
            model_an.partials[0].early_db_s,
            sal_an.partials[0].early_db_s
        );
        if args.summary_only {
            continue;
        }
        print_analysis("Model", &model_an, Some(&sal_an));
        let step = (fs_model / 20000.0).max(1.0) as usize; // 0.05 ms ごと
        let line: Vec<String> = r.force_trace.iter().step_by(step).take(120).map(|f| format!("{f:.0}")).collect();
        println!("Hammer force (N) every {:.2} ms: {}", 1000.0 * step as f64 / fs_model, line.join(" "));
        for &n in &args.trace {
            if let Some(p) = sal_an.partials.iter().find(|p| p.n == n) {
                println!("Envelope of partial {n} ({:.0} Hz), dB every 0.1 s:", p.freq);
                println!("  recording: {}", envelope_line(&sal_cut[pre..], fs, p.freq, 0.1, 4.0));
                println!("  model:     {}", envelope_line(&wet[pre..], fs, p.freq, 0.1, 4.0));
            }
        }
    }
    if args.summary_only {
        return;
    }

    // --- 離鍵: v8 を release_at 秒で離す ---------------------------------
    let (ref_layer, ref_vel, ref_cut, _) = &strikes[ref_idx];
    let t_off = args.release_at;
    let rel_seconds = t_off + 2.0;
    let r = scene.render(rel_seconds, &[(0.0, note_on(*ref_vel)), (t_off, Event::NoteOff { key: played_key })], pre);
    let mut model_rel = post(&r.dry, eq.as_ref(), thump.as_ref(), thump_gain(r.peak_force), pre);
    let rel_path = args.sal_dir.join("C4_release.wav");
    let release_sample = if rel_path.exists() { Some(load_mono(&rel_path).1) } else { None };
    // SFZ が鳴らすもの: 打弦サンプルに ampeg_release=1 s の減衰、離鍵サンプルに
    // volume=-4 dB、rt_decay=6 dB/s × 保持時間、amp_veltrack=94 のベロシティ利得。
    let sfz_rel_gain = {
        let vel = *ref_vel as f64 / 127.0;
        let veltrack = 0.06 + 0.94 * vel * vel;
        10f64.powf(-4.0 / 20.0) * 10f64.powf(-6.0 * t_off / 20.0) * veltrack
    };
    let rel_len = (rel_seconds * fs) as usize;
    let mut sfz_rel: Vec<f64> = ref_cut.iter().take(rel_len).copied().collect();
    sfz_rel.resize(rel_len, 0.0);
    let off_idx = pre + (t_off * fs) as usize;
    for i in off_idx..rel_len {
        sfz_rel[i] *= 10f64.powf(-96.0 * (i - off_idx) as f64 / fs / 20.0);
    }
    if let Some(rel) = &release_sample {
        for (i, v) in rel.iter().enumerate() {
            if off_idx + i < rel_len {
                sfz_rel[off_idx + i] += sfz_rel_gain * v;
                if args.release_noise {
                    model_rel[off_idx + i] += sfz_rel_gain * v;
                }
            }
        }
    }
    write_norm(args.out_dir.join(format!("model_C4_v{ref_layer}_release.wav")), fs, &model_rel);
    write_norm(args.out_dir.join("sfz_C4_release.wav"), fs, &sfz_rel);
    write_ab(args.out_dir.join("ab_C4_release.wav"), fs, &sfz_rel, &model_rel, rel_seconds);
    println!(
        "\n=== release: note v{ref_layer} held {t_off:.2} s, then key up (damper rho {:.2} at {:.2} L, {:.0} ms); SFZ reference = sample with 1 s release + rel40 at {:.1} dB ===",
        args.damper_rho,
        args.damper_pos,
        args.damper_ms,
        20.0 * sfz_rel_gain.log10()
    );
    for n in 1..=3 {
        let p = &ref_an.partials[n - 1];
        println!("Partial {n} ({:.0} Hz) dB every 50 ms from key up:", p.freq);
        println!("  model:          {}", envelope_line(&model_rel[off_idx..], fs, p.freq, 0.05, 0.8));
        println!("  SFZ reference:  {}", envelope_line(&sfz_rel[off_idx..], fs, p.freq, 0.05, 0.8));
    }
    {
        let mut e = r.energies.clone();
        e.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let total: f64 = e.iter().map(|x| x.1).sum::<f64>().max(1e-30);
        let top: Vec<String> = e.iter().take(6).map(|(k, v)| format!("key {k}: {:.0} dB", 10.0 * (v / total).log10())).collect();
        println!("Energy left in the strings at the end of the release render: {}", top.join(", "));
    }
    let model_after = analysis::rms(&model_rel[off_idx..], fs, 0.2, 0.6);
    let sfz_after = analysis::rms(&sfz_rel[off_idx..], fs, 0.2, 0.6);
    let before = analysis::rms(&model_rel, fs, t_off - 0.4, t_off);
    println!(
        "RMS 0.2-0.6 s after key up, relative to the 0.4 s before: model {:+.1} dB, SFZ reference {:+.1} dB",
        20.0 * (model_after / before).log10(),
        20.0 * (sfz_after / before).log10()
    );

    // --- ペダル: 踏んだまま打弦（共鳴の有無の聴き比べ）--------------------
    let r = scene.render(args.seconds, &[(0.0, Event::Pedal(true)), (0.0, note_on(*ref_vel))], pre);
    let model_pedal = post(&r.dry, eq.as_ref(), thump.as_ref(), thump_gain(r.peak_force), pre);
    write_norm(args.out_dir.join(format!("model_C4_v{ref_layer}_pedal.wav")), fs, &model_pedal);
    let plain = post(&strikes[ref_idx].3.dry, eq.as_ref(), thump.as_ref(), thump_gain(strikes[ref_idx].3.peak_force), pre);
    write_ab(args.out_dir.join("ab_C4_pedal_vs_plain.wav"), fs, &plain, &model_pedal, 4.0);

    // --- ペダル共鳴: 踏んだまま打弦し、1 秒後に打鍵した弦だけ手で止める ---
    for (name, vel) in [("loud", 60u32), ("soft", 40u32)] {
        let harm_path = args.sal_dir.join(format!("C4_pedal_resonance_{name}.wav"));
        if !harm_path.exists() {
            continue;
        }
        let (_, harm) = load_mono(&harm_path);
        let harm_len = harm.len() as f64 / fs;
        let t_damp = 1.0;
        let r = scene.render(
            t_damp + harm_len + 0.1,
            &[(0.0, Event::Pedal(true)), (0.0, note_on(vel)), (t_damp, Event::HandDamp { key: played_key })],
            pre,
        );
        let wet = post(&r.dry, eq.as_ref(), thump.as_ref(), thump_gain(r.peak_force), pre);
        let damp_idx = pre + (t_damp * fs) as usize;
        let model_res: Vec<f64> = wet[damp_idx..].to_vec();
        write_norm(args.out_dir.join(format!("model_C4_pedal_resonance_{name}.wav")), fs, &model_res);
        write_ab(args.out_dir.join(format!("ab_C4_pedal_resonance_{name}.wav")), fs, &harm, &model_res, harm_len);
        let harm_an = analysis::analyze(&harm, fs, f0, 16, 4000.0);
        let model_an = analysis::analyze(&model_res, fs, f0, 16, 4000.0);
        let before = analysis::rms(&wet, fs, 0.0, 0.5);
        println!(
            "\n=== pedal resonance ({name}): pedal down, note at MIDI velocity {vel}, played strings stopped at {t_damp:.1} s; RMS of the first 0.5 s after stopping: model {:+.1} dB, Salamander harm {:+.1} dB relative to the model's strike ===",
            20.0 * (analysis::rms(&model_res, fs, 0.0, 0.5) / before).log10(),
            20.0 * (analysis::rms(&harm, fs, 0.0, 0.5) / before).log10()
        );
        print_analysis("Model resonance", &model_an, Some(&harm_an));
        for n in 1..=2 {
            let p = &harm_an.partials[n - 1];
            println!("Partial {n} ({:.0} Hz) dB every 0.1 s:", p.freq);
            println!("  Salamander: {}", envelope_line(&harm, fs, p.freq, 0.1, 2.5));
            println!("  model:      {}", envelope_line(&model_res, fs, p.freq, 0.1, 2.5));
        }
    }

    println!(
        "\nWrote to {0}/: ab_C4_v*.wav (strike), ab_C4_release.wav (SFZ release vs model damper), ab_C4_pedal_resonance_{{loud,soft}}.wav (Salamander harm sample vs model), ab_C4_pedal_vs_plain.wav (model without / with pedal).\nListen: afplay {0}/ab_C4_release.wav",
        args.out_dir.display()
    );
}
