use std::io::Read;

use tetris_audio::{Cue, Mixer, SilentSink, Sink, SoundBank, WavSink, SAMPLE_RATE};

const ALL_CUES: &[Cue] = &[
    Cue::Move,
    Cue::Rotate,
    Cue::Lock,
    Cue::Hold,
    Cue::HoldDenied,
    Cue::Clear(1),
    Cue::Clear(2),
    Cue::Clear(3),
    Cue::Tetris,
    Cue::Spin,
    Cue::LevelUp,
    Cue::Combo,
    Cue::BackToBack,
    Cue::Danger,
    Cue::Pause,
    Cue::Resume,
    Cue::MenuMove,
    Cue::MenuSelect,
    Cue::GameOver,
];

#[test]
fn every_cue_renders_finite_bounded_audio() {
    let bank = SoundBank::default();
    for cue in ALL_CUES {
        let samples = bank.render(*cue);
        assert!(!samples.is_empty(), "{cue:?} rendered nothing");
        assert!(
            samples.iter().all(|s| s.is_finite()),
            "{cue:?} produced NaN or infinity"
        );
        assert!(
            samples.iter().all(|s| s.abs() <= 1.0),
            "{cue:?} exceeds full scale"
        );
        let seconds = samples.len() as f32 / SAMPLE_RATE as f32;
        assert!(
            seconds <= cue.duration() + 0.01,
            "{cue:?} is {seconds}s, expected at most {}s",
            cue.duration()
        );
        assert!(seconds > 0.02, "{cue:?} is inaudibly short");
    }
}

#[test]
fn cues_are_peak_normalised_so_the_mix_is_even() {
    let bank = SoundBank::default();
    for cue in ALL_CUES {
        let samples = bank.render(*cue);
        let peak = samples.iter().fold(0.0f32, |a, s| a.max(s.abs()));
        assert!(
            (peak - 0.8).abs() < 0.02,
            "{cue:?} peaks at {peak}, expected ~0.8"
        );
    }
}

#[test]
fn rendering_is_deterministic() {
    let bank = SoundBank::default();
    for cue in ALL_CUES {
        assert_eq!(bank.render(*cue), bank.render(*cue), "{cue:?} differs");
    }
}

#[test]
fn different_cues_sound_different() {
    let bank = SoundBank::default();
    let tetris = bank.render(Cue::Tetris);
    let mv = bank.render(Cue::Move);
    assert_ne!(tetris.len(), mv.len(), "lengths should differ");
    assert_ne!(bank.render(Cue::Clear(1)), bank.render(Cue::Clear(3)));
}

#[test]
fn mixer_produces_sound_then_silence() {
    let mut mixer = Mixer::new(SoundBank::default());
    mixer.push(Cue::Move);
    assert_eq!(mixer.active_voices(), 1);

    let mut buf = vec![0i16; 256 * 2];
    mixer.fill(&mut buf);
    assert!(
        buf.iter().any(|s| *s != 0),
        "a fresh cue should be audible immediately"
    );

    let mut frames = 256;
    while mixer.active_voices() > 0 && frames < 20_000 {
        buf.iter_mut().for_each(|s| *s = 0);
        mixer.fill(&mut buf);
        frames += 256;
    }
    assert!(frames < 6000, "cue lasted {frames} frames");
    assert_eq!(mixer.active_voices(), 0);

    buf.iter_mut().for_each(|s| *s = 0);
    mixer.fill(&mut buf);
    assert!(
        buf.iter().all(|s| *s == 0),
        "silence expected after the tail"
    );
}

#[test]
fn mixer_layers_several_voices_without_clipping() {
    let mut mixer = Mixer::new(SoundBank::default());
    for _ in 0..6 {
        mixer.push(Cue::Tetris);
    }
    assert_eq!(mixer.active_voices(), 6);
    let mut buf = vec![0i16; 512 * 2];
    for _ in 0..40 {
        mixer.fill(&mut buf);
        assert!(
            buf.iter().all(|s| (i32::from(*s)).abs() <= 32767),
            "output must stay inside i16"
        );
    }
}

#[test]
fn voice_cap_drops_the_oldest_instead_of_growing_forever() {
    let mut mixer = Mixer::new(SoundBank::default());
    for _ in 0..200 {
        mixer.push(Cue::GameOver);
    }
    assert!(
        mixer.active_voices() <= 24,
        "{} voices",
        mixer.active_voices()
    );
    assert!(mixer.dropped_voices() > 0);
}

#[test]
fn volume_and_mute_control_the_output() {
    let mut mixer = Mixer::new(SoundBank::default());
    mixer.set_volume(0.0);
    mixer.push(Cue::Tetris);
    let mut buf = vec![0i16; 256 * 2];
    mixer.fill(&mut buf);
    assert!(buf.iter().all(|s| *s == 0), "zero volume means silence");

    let mut mixer = Mixer::new(SoundBank::default());
    mixer.set_muted(true);
    mixer.push(Cue::Tetris);
    assert_eq!(mixer.active_voices(), 0, "muted mixer ignores cues");

    let mut mixer = Mixer::new(SoundBank::default());
    mixer.set_volume(9.0);
    assert_eq!(mixer.volume(), 1.0);
    mixer.set_volume(-3.0);
    assert_eq!(mixer.volume(), 0.0);
}

#[test]
fn silent_sink_records_what_would_have_played() {
    let mut sink = SilentSink::default();
    sink.play(Cue::Lock);
    sink.play(Cue::Tetris);
    sink.set_volume(0.4);
    assert_eq!(sink.heard, vec![Cue::Lock, Cue::Tetris]);
    assert_eq!(sink.volume(), 0.4);

    let mut muted = SilentSink::default();
    muted.set_muted(true);
    muted.play(Cue::Lock);
    assert!(muted.heard.is_empty());
}

#[test]
fn wav_sink_writes_a_playable_file() {
    let dir = std::env::temp_dir().join("tetris-audio-test");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("cues.wav");

    let mut sink = WavSink::new(&path);
    sink.play(Cue::Tetris);
    sink.tick();
    assert!(sink.samples().len() > SAMPLE_RATE as usize / 4);
    sink.finish().expect("write wav");

    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .expect("open wav")
        .read_to_end(&mut bytes)
        .expect("read wav");
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    assert_eq!(&bytes[12..16], b"fmt ");
    let data_at = bytes
        .windows(4)
        .position(|w| w == b"data")
        .expect("data chunk");
    let declared = u32::from_le_bytes(bytes[data_at + 4..data_at + 8].try_into().unwrap()) as usize;
    assert_eq!(declared, bytes.len() - data_at - 8);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stereo_frames_are_interleaved_equal_for_mono_sources() {
    let mut mixer = Mixer::new(SoundBank::default());
    mixer.push(Cue::LevelUp);
    let mut buf = vec![0i16; 128 * 2];
    mixer.fill(&mut buf);
    for frame in buf.chunks(2) {
        assert_eq!(frame[0], frame[1], "mono cue should be centred");
    }
}
