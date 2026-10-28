// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * dsp.rs
 *
 * Version: @(#)dsp.rs 0.2.0 2026
 *
 * Description: The Whole Signal Department.
 *              Two pure functions. Downmix: average the channels. Resample:
 *              boxcar when shrinking, straight line when growing. Old way kept
 *              every third sample and prayed. Prayer is not DSP.
 *
 * Author: Magnus Leatti (layer07)
 * URL: https://kernelriot.com
 * Github: /layer07
 *
 *        ██▓    ▄▄▄     ▓██   ██▓▓█████  ██▀███
 *       ▓██▒   ▒████▄    ▒██  ██▒▓█   ▀ ▓██ ▒ ██▒
 *       ▒██░   ▒██  ▀█▄   ▒██ ██░▒███   ▓██ ░▄█ ▒
 *       ▒██░   ░██▄▄▄▄██  ░ ▐██▓░▒▓█ ▄ ▒██▀▀█▄
 *       ░██████▒▓█   ▓██▒ ░ ██▒▓░░▒████▒░██▓ ▒██▒
 *       ░ ▒░▓  ░▒▒   ▓▒█░  ██▒▒▒ ░░ ▒░ ░░ ▒▓ ░▒▓░
 *       ░ ░ ▒  ░ ▒   ▒▒ ░▓██ ░▒░  ░ ░  ░  ░▒ ░ ▒░
 *         ░ ░    ░   ▒   ▒ ▒ ░░     ░     ░░   ░
 *           ░  ░     ░  ░░ ░        ░  ░   ░
 */

// Parakeet wants 16 kHz mono f32. Mics hand us 44.1/48 kHz, 1–8 channels, in
// whatever sample format the driver felt like that morning. The audio callback
// only downmixes (cheap, allocation-free); the resample runs ONCE, after you
// let go of the hotkey, over the whole clip. Nothing clever happens in the
// realtime thread, which is the only correct amount of clever for a realtime
// thread.

/// Interleaved frames → mono by averaging channels. A trailing partial frame
/// (shouldn't happen, drivers are weird) is dropped.
pub fn downmix<'a>(interleaved: &'a [f32], channels: usize) -> impl Iterator<Item = f32> + 'a {
    let ch = channels.max(1);
    interleaved
        .chunks_exact(ch)
        .map(move |frame| frame.iter().sum::<f32>() / ch as f32)
}

/// Resample mono `x` from `from` Hz to `to` Hz.
///
/// Decimating (48k → 16k): each output sample is the MEAN of the input
/// samples its window covers — integrate-and-dump, i.e. a boxcar low-pass
/// fused with the decimator. Not a brick-wall filter, but it kills the worst
/// aliasing for free, and it beats the previous "keep every 3rd sample and
/// pray" by a mile.
///
/// Interpolating (8k Bluetooth headset → 16k): plain linear. There's no
/// information up there to invent anyway.
pub fn resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || from == 0 || to == 0 || x.is_empty() {
        return x.to_vec();
    }
    let ratio = from as f64 / to as f64;
    // Integer math for the length: 44100 / 2.75625 in floats is 15999.999…
    let n_out = (x.len() as u64 * to as u64 / from as u64) as usize;
    let last  = x.len() - 1;

    (0..n_out)
        .map(|i| {
            let pos = i as f64 * ratio;
            if ratio > 1.0 {
                let lo = (pos as usize).min(last);
                let hi = ((pos + ratio) as usize).clamp(lo + 1, x.len());
                x[lo..hi].iter().sum::<f32>() / (hi - lo) as f32
            } else {
                let j = pos as usize;
                let t = (pos - j as f64) as f32;
                x[j] * (1.0 - t) + x[(j + 1).min(last)] * t
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_averages_channels() {
        let v: Vec<f32> = downmix(&[1.0, 3.0, -2.0, 2.0, 9.0], 2).collect();
        assert_eq!(v, vec![2.0, 0.0]); // trailing half-frame dropped
    }

    #[test]
    fn identity_and_empty() {
        assert_eq!(resample(&[1.0, 2.0], 16_000, 16_000), vec![1.0, 2.0]);
        assert!(resample(&[], 48_000, 16_000).is_empty());
    }

    #[test]
    fn decimate_48k_keeps_dc_and_length() {
        let y = resample(&vec![0.5; 48_000], 48_000, 16_000);
        assert_eq!(y.len(), 16_000);
        assert!(y.iter().all(|&s| (s - 0.5).abs() < 1e-6));
    }

    #[test]
    fn decimate_kills_nyquist_alias() {
        // A tone at 24 kHz (+1,-1,+1,...) is pure alias at 16 kHz. Picking every
        // 3rd sample turns it into a full-scale ±1 square wave of garbage; the
        // boxcar crushes it to ±1/3.
        let x: Vec<f32> = (0..4800).map(|i| if i % 2 == 0 { 1.0 } else { -1.0 }).collect();
        let peak = resample(&x, 48_000, 16_000).iter().fold(0f32, |m, s| m.max(s.abs()));
        assert!(peak < 0.34, "alias leaked through at {peak}");
    }

    #[test]
    fn interpolate_8k_doubles() {
        let y = resample(&[0.0, 1.0, 0.0, 1.0], 8_000, 16_000);
        assert_eq!(y.len(), 8);
        assert!((y[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn odd_ratio_44k1() {
        let y = resample(&vec![0.25; 44_100], 44_100, 16_000);
        assert_eq!(y.len(), 16_000);
        assert!(y.iter().all(|&s| (s - 0.25).abs() < 1e-6));
    }
}
