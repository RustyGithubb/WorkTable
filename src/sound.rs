//! Petits sons de notification, synthétisés au démarrage (aucun fichier audio) :
//! mail = carillon de deux notes, message = « blip » plus court et plus aigu.
//! Joués sans bloquer l'interface (Windows : `PlaySound` en mémoire ; ailleurs : rien).

use std::sync::OnceLock;

const RATE: u32 = 44_100;

#[derive(Clone, Copy)]
pub enum Sound {
    Mail,
    Message,
}

/// Note : fréquence (Hz), départ (s), durée (s), volume (0–1).
struct Note(f32, f32, f32, f32);

/// Son de cloche douce : fondamentale + harmoniques, attaque courte, extinction exponentielle.
fn render(notes: &[Note], length: f32) -> Vec<i16> {
    let n = (length * RATE as f32) as usize;
    let mut out = vec![0f32; n];
    for &Note(f, start, dur, vol) in notes {
        let s0 = (start * RATE as f32) as usize;
        let len = ((dur * RATE as f32) as usize).min(n.saturating_sub(s0));
        for i in 0..len {
            let t = i as f32 / RATE as f32;
            let attack = (t / 0.006).min(1.0);
            let decay = (-t * 5.5 / dur).exp();
            let w = std::f32::consts::TAU * f * t;
            let v = w.sin() + 0.32 * (2.0 * w).sin() * (-t * 9.0 / dur).exp() + 0.12 * (3.0 * w).sin() * (-t * 14.0 / dur).exp();
            out[s0 + i] += v * attack * decay * vol;
        }
    }
    // Fondu de fin pour éviter un clic.
    let fade = (0.01 * RATE as f32) as usize;
    for i in 0..fade.min(n) {
        out[n - 1 - i] *= i as f32 / fade as f32;
    }
    out.iter().map(|v| (v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).collect()
}

/// Fichier WAV (PCM 16 bits mono) en mémoire.
fn wav(samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + data_len as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&RATE.to_le_bytes());
    w.extend_from_slice(&(RATE * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        w.extend_from_slice(&s.to_le_bytes());
    }
    w
}

fn bytes(s: Sound) -> &'static [u8] {
    static MAIL: OnceLock<Vec<u8>> = OnceLock::new();
    static MESSAGE: OnceLock<Vec<u8>> = OnceLock::new();
    match s {
        // Deux notes montantes (la → mi), posées.
        Sound::Mail => MAIL.get_or_init(|| wav(&render(&[Note(880.0, 0.0, 0.45, 0.30), Note(1318.5, 0.12, 0.55, 0.28)], 0.70))),
        // Deux notes rapprochées plus aiguës (do → sol), plus brèves.
        Sound::Message => MESSAGE.get_or_init(|| wav(&render(&[Note(1046.5, 0.0, 0.22, 0.26), Note(1568.0, 0.06, 0.30, 0.24)], 0.40))),
    }
}

/// Joue le son sans attendre (un nouveau son remplace celui en cours).
pub fn play(s: Sound) {
    let data = bytes(s);
    #[cfg(windows)]
    {
        use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
        // SAFETY : `data` est un WAV valide, statique (il reste en mémoire pendant la lecture).
        unsafe {
            PlaySoundW(data.as_ptr().cast(), std::ptr::null_mut(), SND_ASYNC | SND_MEMORY | SND_NODEFAULT);
        }
    }
    #[cfg(not(windows))]
    let _ = data;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sounds_are_valid_and_distinct() {
        let (m, c) = (bytes(Sound::Mail), bytes(Sound::Message));
        for b in [m, c] {
            assert_eq!(&b[0..4], b"RIFF");
            assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize, b.len() - 8);
            assert!(b.len() > 44 + 10_000);
        }
        assert_ne!(m.len(), c.len());
    }
}
