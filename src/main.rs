mod ui;

use std::{
    error::Error,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use jner_daw::{
    audio::{AudioOutput, Snapshot, output_devices},
    engine::Command,
    mixer::MixerState,
    project::{self, Project},
    samples::starter_kit,
    sequencer::{Bpm, Pattern},
    timeline::{Arrangement, TICKS_PER_BEAT},
};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => ui::run().map_err(Into::into),
        [arg, path] if arg == "--open" => {
            let path = std::path::PathBuf::from(path);
            let project = project::load(&path)?;
            ui::run_with_project(Some((path, project))).map_err(Into::into)
        }
        [arg] if arg == "--smoke-project" => smoke_project(),
        [arg] if arg == "--smoke-recording" => smoke_recording(),
        [arg] if arg == "--smoke-export" => smoke_export(None),
        [arg, path] if arg == "--smoke-export" => smoke_export(Some(std::path::Path::new(path))),
        [arg, input, output, rate, beats] if arg == "--export" => {
            let project = project::load(std::path::Path::new(input))?;
            let options = jner_daw::export::ExportOptions {
                mode: jner_daw::export::RenderMode::from_project(&project),
                sample_rate: rate.parse()?,
                beats: beats.parse()?,
            };
            let frames = jner_daw::export::export_wav(
                std::path::Path::new(output),
                &project,
                options,
                &jner_daw::export::ExportControl::default(),
            )?;
            println!(
                "WAV exportado: {frames} frames estéreo a {} Hz · {}",
                options.sample_rate,
                options.mode.name()
            );
            Ok(())
        }
        [arg] if arg == "--list-inputs" => {
            let devices = jner_daw::recording::input_devices()?;
            if devices.is_empty() {
                return Err("No hay entradas de audio".into());
            }
            for (index, device) in devices.iter().enumerate() {
                println!(
                    "{index}: {}{}",
                    device.name,
                    if device.is_default {
                        " [predeterminada]"
                    } else {
                        ""
                    }
                );
            }
            println!("Solo enumeración; no se abrió ninguna entrada ni se capturó audio.");
            Ok(())
        }
        [arg] if arg == "--smoke-midi" => smoke_midi(None),
        [arg, index] if arg == "--smoke-midi" => smoke_midi(Some(index.parse()?)),
        [arg] if arg == "--smoke-synth" => smoke_synth(None),
        [arg, index] if arg == "--smoke-synth" => smoke_synth(Some(index.parse()?)),
        [arg] if arg == "--smoke-effects" => smoke_effects(None),
        [arg, index] if arg == "--smoke-effects" => smoke_effects(Some(index.parse()?)),
        [arg] if arg == "--smoke-automation" => smoke_automation(None),
        [arg, index] if arg == "--smoke-automation" => smoke_automation(Some(index.parse()?)),
        [arg] if arg == "--list-devices" => {
            let devices = output_devices()?;
            if devices.is_empty() {
                return Err("No hay dispositivos de salida".into());
            }
            for (index, device) in devices.iter().enumerate() {
                println!(
                    "{index}: {}{}",
                    device.name,
                    if device.is_default {
                        " [predeterminado]"
                    } else {
                        ""
                    }
                );
            }
            Ok(())
        }
        [arg] if arg == "--smoke-test" => smoke_test(None),
        [arg, index] if arg == "--smoke-test" => smoke_test(Some(index.parse()?)),
        [arg] if arg == "--smoke-beat" => smoke_beat(None),
        [arg, index] if arg == "--smoke-beat" => smoke_beat(Some(index.parse()?)),
        [arg] if arg == "--smoke-mixer" => smoke_mixer(None),
        [arg, index] if arg == "--smoke-mixer" => smoke_mixer(Some(index.parse()?)),
        [arg] if arg == "--smoke-timeline" => smoke_timeline(None),
        [arg, index] if arg == "--smoke-timeline" => smoke_timeline(Some(index.parse()?)),
        [arg] if arg == "--help" || arg == "-h" => {
            println!(
                "M10: --export entrada.jner salida.wav sample_rate beats\n     --smoke-export [salida.wav]  Render/lectura de ejemplo sin dispositivo"
            );
            println!(
                "Jner DAW\n  cargo run --locked                             Ventana nativa\n  cargo run --locked -- --open archivo.jner       Abrir proyecto\n  cargo run --locked -- --smoke-project           Guardado/reapertura y audio offline\n  cargo run --locked -- --smoke-automation [índice] Curvas M8, BPM y reinicio\n  cargo run --locked -- --smoke-effects [índice]   Delay y distorsión M7, edición en vivo\n  cargo run --locked -- --smoke-synth [índice]     Instrumento M6, cuatro ondas y ADSR\n  cargo run --locked -- --smoke-midi [índice]      Eventos M5 y reloj, sin instrumento sonoro\n  cargo run --locked -- --list-devices            Enumerar salidas\n  cargo run --locked -- --smoke-test [índice]      Tono M0, Play/Stop/reinicio\n  cargo run --locked -- --smoke-beat [índice]      Beat M1, loop y edición en vivo\n  cargo run --locked -- --smoke-mixer [índice]     Mixer M2, pan, gain, mute y solo\n  cargo run --locked -- --smoke-timeline [índice]  Clips M3, reloj musical, recorte y reinicio"
            );
            println!(
                "  cargo run --locked -- --smoke-recording         Captura sintética M9; sin micrófono\n  cargo run --locked -- --list-inputs             Enumerar entradas sin abrirlas"
            );
            Ok(())
        }
        _ => Err("Argumentos no válidos; utiliza --help".into()),
    }
}

fn smoke_export(destination: Option<&std::path::Path>) -> Result<(), Box<dyn Error>> {
    let mut project = Project::empty();
    project.pattern = Pattern::demo();
    project.effects.delay_enabled = true;
    let temp = tempfile::tempdir()?;
    let temporary_path = temp.path().join("export.wav");
    let path = destination.unwrap_or(&temporary_path);
    let options = jner_daw::export::ExportOptions::default();
    let frames = jner_daw::export::export_wav(
        path,
        &project,
        options,
        &jner_daw::export::ExportControl::default(),
    )?;
    let mut wav = hound::WavReader::open(path)?;
    if wav.spec().channels != 2
        || wav.spec().sample_rate != 48_000
        || wav.spec().bits_per_sample != 16
        || u64::from(wav.duration()) != frames
        || frames != 96_000
    {
        return Err("Cabecera o duración WAV incorrectas".into());
    }
    let mut audible = false;
    for value in wav.samples::<i16>() {
        audible |= value? != 0;
    }
    if !audible {
        return Err("El WAV de prueba quedó en silencio".into());
    }
    println!(
        "OK: render offline sin CPAL; WAV estéreo PCM16, 48 kHz, 96 000 frames (2 s), lectura completa y señal no nula. Sin emisión de sonido."
    );
    if destination.is_some() {
        println!("Ejemplo disponible en {}", path.display());
    }
    Ok(())
}

fn smoke_recording() -> Result<(), Box<dyn Error>> {
    use jner_daw::{engine::Engine, timeline::TimelinePlayer};
    use std::num::NonZeroU32;
    let source = Arc::new(jner_daw::recording::synthetic_take()?);
    if source.frame_count() != 11_025 || source.sample_rate() != 44_100 {
        return Err("Duración de captura incorrecta".into());
    }
    let mut original = Project::empty();
    original.timeline_sources.push(source);
    original.arrangement.add(2, 0, 0, 11_025)?;
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("recording.jner");
    project::save(&path, &original)?;
    let reopened = project::load(&path)?;
    if original.arrangement.clips() != reopened.arrangement.clips() {
        return Err("El clip grabado cambió al reabrir".into());
    }
    let rate = NonZeroU32::new(48_000).ok_or("Sample rate cero")?;
    let mut before = Engine::with_timeline(
        rate,
        TimelinePlayer::prepare(&original.arrangement, &original.timeline_sources, rate)?,
        original.bpm,
    );
    let mut after = Engine::with_timeline(
        rate,
        TimelinePlayer::prepare(&reopened.arrangement, &reopened.timeline_sources, rate)?,
        reopened.bpm,
    );
    before.apply(Command::Play);
    after.apply(Command::Play);
    let mut audible = false;
    for _ in 0..24_000 {
        let a = before.next_frame();
        let b = after.next_frame();
        if a != b {
            return Err("El audio grabado cambió al guardar y reabrir".into());
        }
        audible |= a.iter().any(|v| v.abs() > 0.001);
    }
    if !audible {
        return Err("La captura sintética quedó en silencio".into());
    }
    println!(
        "OK: captura sintética estéreo → mono, gate de 250 ms (11 025 frames a 44.1 kHz), clip en Pista 3/beat 1, guardado/reapertura y 24 000 frames estéreo a 48 kHz idénticos. Sin abrir micrófono ni emitir sonido."
    );
    Ok(())
}

fn wait_for(
    output: &AudioOutput,
    condition: impl Fn(Snapshot) -> bool,
) -> Result<Snapshot, Box<dyn Error>> {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let snapshot = output.snapshot();
        if snapshot.errors > 0 {
            return Err("CPAL notificó un error de stream".into());
        }
        if condition(snapshot) {
            return Ok(snapshot);
        }
        if Instant::now() >= deadline {
            return Err("El callback de audio no respondió dentro del plazo".into());
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn smoke_test(index: Option<usize>) -> Result<(), Box<dyn Error>> {
    let devices = output_devices()?;
    let device = match index {
        Some(index) => devices.get(index).ok_or("Índice de salida no válido")?,
        None => devices
            .iter()
            .find(|d| d.is_default)
            .or(devices.first())
            .ok_or("No hay salida disponible")?,
    };
    println!("Salida: {}", device.name);
    let mut output = AudioOutput::open(&device.device)?;
    println!("Configuración: {:?}; buffer del sistema", output.info);
    let initial = wait_for(&output, |s| s.callbacks > 0)?;
    if initial.playing || initial.position != 0 || initial.peak != 0.0 {
        return Err("El stream no comenzó en silencio".into());
    }
    println!("Reproduciendo tono de 440 Hz a nivel 8% durante dos segundos de audio...");
    output.send(Command::Play)?;
    let rate = u64::from(output.info.sample_rate);
    let playing = wait_for(&output, |s| s.playing && s.position >= rate * 2)?;
    println!(
        "Play: {} frames de transport; {} callbacks; último buffer {} frames; errores {}",
        playing.position, playing.callbacks, playing.buffer_frames, playing.errors
    );
    output.send(Command::Stop)?;
    let stopped = wait_for(&output, |s| !s.playing && s.position == 0 && s.peak == 0.0)?;
    let later = wait_for(&output, |s| s.callbacks > stopped.callbacks + 2)?;
    if later.position != 0 || later.peak != 0.0 {
        return Err("Stop no mantuvo el reloj en cero y la salida en silencio".into());
    }
    output.send(Command::Play)?;
    wait_for(&output, |s| {
        s.playing && s.position > 0 && s.position < rate
    })?;
    output.send(Command::Stop)?;
    let final_state = wait_for(&output, |s| !s.playing && s.position == 0 && s.peak == 0.0)?;
    println!(
        "OK: silencio inicial, reloj, Play, Stop y reinicio; {} frames procesados; errores {}.",
        final_state.frames, final_state.errors
    );
    println!(
        "Esta prueba no verifica escucha, latencia de extremo a extremo ni ausencia de glitches."
    );
    Ok(())
}

fn smoke_beat(index: Option<usize>) -> Result<(), Box<dyn Error>> {
    let devices = output_devices()?;
    let device = match index {
        Some(index) => devices.get(index).ok_or("Índice de salida no válido")?,
        None => devices
            .iter()
            .find(|d| d.is_default)
            .or(devices.first())
            .ok_or("No hay salida disponible")?,
    };
    println!("Salida: {}", device.name);
    let sources = starter_kit();
    let mut output = AudioOutput::open_drums(
        &device.device,
        &sources,
        Pattern::demo(),
        Bpm::default(),
        MixerState::default(),
    )?;
    println!("Configuración: {:?}; buffer del sistema", output.info);
    let initial = wait_for(&output, |s| s.callbacks > 0)?;
    if initial.playing || initial.position != 0 || initial.peak != 0.0 {
        return Err("El stream no comenzó en silencio".into());
    }
    println!("Reproduciendo dos loops de cuatro sonidos a 120 BPM...");
    output.send(Command::Play)?;
    wait_for(&output, |s| s.playing && s.peak > 0.01)?;
    let looped = wait_for(&output, |s| s.step_events >= 33)?;
    println!(
        "Loop: {} eventos de step, {} frames, errores {}",
        looped.step_events, looped.position, looped.errors
    );
    output.send(Command::SetBpm(Bpm::new(137)?))?;
    output.send(Command::SetGain(0.06))?;
    output.send(Command::SetPattern(Pattern::default()))?;
    let rate = u64::from(output.info.sample_rate);
    let edited = wait_for(&output, |s| {
        s.position > looped.position + rate && s.peak == 0.0
    })?;
    if edited.step_events <= looped.step_events {
        return Err("El reloj se detuvo al editar el patrón".into());
    }
    output.send(Command::SetPattern(Pattern::demo()))?;
    wait_for(&output, |s| s.position > edited.position && s.peak > 0.01)?;
    output.send(Command::Stop)?;
    wait_for(&output, |s| {
        !s.playing && s.position == 0 && s.peak == 0.0 && s.step.is_none()
    })?;
    output.send(Command::Play)?;
    wait_for(&output, |s| s.playing && s.step == Some(0) && s.peak > 0.01)?;
    output.send(Command::Stop)?;
    let stopped = wait_for(&output, |s| !s.playing && s.position == 0 && s.peak == 0.0)?;
    println!(
        "OK: loop, BPM/master/patrón durante reproducción, Stop y reinicio; último buffer {} frames; errores {}.",
        stopped.buffer_frames, stopped.errors
    );
    println!(
        "Pendiente escucha humana: esta prueba no demuestra ausencia de glitches ni mide latencia."
    );
    Ok(())
}

fn smoke_mixer(index: Option<usize>) -> Result<(), Box<dyn Error>> {
    let devices = output_devices()?;
    let device = match index {
        Some(index) => devices.get(index).ok_or("Índice de salida no válido")?,
        None => devices
            .iter()
            .find(|d| d.is_default)
            .or(devices.first())
            .ok_or("No hay salida disponible")?,
    };
    let sources = starter_kit();
    let mut mixer = MixerState::default();
    mixer.tracks[0].solo = true;
    mixer.tracks[0].pan = -1.0;
    let mut output = AudioOutput::open_drums(
        &device.device,
        &sources,
        Pattern::demo(),
        Bpm::default(),
        mixer,
    )?;
    if output.info.channels < 2 {
        return Err("La prueba de panorama requiere una salida de al menos dos canales".into());
    }
    println!("Salida: {} · {:?}", device.name, output.info);
    wait_for(&output, |s| s.callbacks >= 2)?;
    let rate = u64::from(output.info.sample_rate);
    output.send(Command::Play)?;
    let left = wait_for(&output, |s| {
        s.stereo_peaks[0] > 0.01 && s.stereo_peaks[1] == 0.0
    })?;
    println!("Pan L: {:?}", left.stereo_peaks);
    mixer.tracks[0].pan = 1.0;
    output.send(Command::SetMixer(mixer))?;
    let right = wait_for(&output, |s| {
        s.position > left.position + rate / 50
            && s.stereo_peaks[1] > 0.01
            && s.stereo_peaks[0] == 0.0
    })?;
    println!("Pan R: {:?}", right.stereo_peaks);
    mixer.tracks[0].gain = 0.0;
    output.send(Command::SetMixer(mixer))?;
    let zero = wait_for(&output, |s| {
        s.position > right.position + rate / 50 && s.peak == 0.0
    })?;
    mixer.tracks[0].gain = 0.5;
    mixer.tracks[0].pan = -1.0;
    mixer.tracks[1].solo = true;
    mixer.tracks[1].pan = 1.0;
    output.send(Command::SetMixer(mixer))?;
    let solos = wait_for(&output, |s| {
        s.position > zero.position
            && s.track_peaks[0] > 0.01
            && s.track_peaks[1] > 0.01
            && s.track_peaks[2] == 0.0
            && s.track_peaks[3] == 0.0
    })?;
    println!("Dos Solo, picos por pista: {:?}", solos.track_peaks);
    mixer.tracks[0].muted = true;
    output.send(Command::SetMixer(mixer))?;
    let muted = wait_for(&output, |s| {
        s.position > solos.position + rate / 50
            && s.track_peaks[0] == 0.0
            && s.track_peaks[1] > 0.01
    })?;
    mixer.tracks[1].muted = true;
    output.send(Command::SetMixer(mixer))?;
    let silent = wait_for(&output, |s| {
        s.position > muted.position + rate / 50 && s.peak == 0.0
    })?;
    mixer = MixerState::default();
    mixer.tracks[0].solo = true;
    mixer.tracks[0].pan = 1.0;
    output.send(Command::SetMixer(mixer))?;
    wait_for(&output, |s| {
        s.position > silent.position && s.stereo_peaks[1] > 0.01 && s.stereo_peaks[0] == 0.0
    })?;
    output.send(Command::Stop)?;
    wait_for(&output, |s| !s.playing && s.position == 0 && s.peak == 0.0)?;
    output.send(Command::Play)?;
    wait_for(&output, |s| {
        s.playing && s.stereo_peaks[1] > 0.01 && s.stereo_peaks[0] == 0.0
    })?;
    output.send(Command::Stop)?;
    let stopped = wait_for(&output, |s| !s.playing && s.position == 0 && s.peak == 0.0)?;
    println!(
        "OK: pan L/R, gain cero, Solo múltiple, Mute sobre Solo, reloj continuo y ajustes conservados tras Stop/Play; buffer {} frames; errores {}.",
        stopped.buffer_frames, stopped.errors
    );
    println!(
        "Pendiente escucha humana: los picos del buffer no prueban la reproducción física ni la ausencia de clicks."
    );
    Ok(())
}

fn smoke_timeline(index: Option<usize>) -> Result<(), Box<dyn Error>> {
    let devices = output_devices()?;
    let device = match index {
        Some(index) => devices.get(index).ok_or("Índice de salida no válido")?,
        None => devices
            .iter()
            .find(|d| d.is_default)
            .or(devices.first())
            .ok_or("No hay salida disponible")?,
    };
    let sources: Vec<_> = starter_kit().into_iter().map(Arc::new).collect();
    let bpm = Bpm::default();
    let mut arrangement = Arrangement::default();
    let id = arrangement.add(0, 0, 0, sources[0].frame_count())?;
    let clip = arrangement
        .get(id)
        .ok_or("Clip ausente")?
        .moved(480.0, 0, true)
        .trim_right(
            -480.0,
            sources[0].sample_rate(),
            sources[0].frame_count(),
            bpm,
            true,
        )
        .trim_left(240.0, sources[0].sample_rate(), bpm, true);
    arrangement.replace(clip)?;
    arrangement.add(1, 1, TICKS_PER_BEAT * 2, sources[1].frame_count())?;
    arrangement.add(2, 2, TICKS_PER_BEAT * 3, sources[2].frame_count())?;
    let mut output = AudioOutput::open_timeline(
        &device.device,
        &sources,
        &arrangement,
        bpm,
        MixerState::default(),
    )?;
    println!("Salida: {} · {:?}", device.name, output.info);
    println!(
        "Clip editado: inicio {} ticks, fuente {}–{} frames",
        clip.start_tick, clip.in_frame, clip.out_frame
    );
    let initial = wait_for(&output, |s| s.callbacks >= 2)?;
    if initial.playing || initial.peak != 0.0 || initial.musical_ticks != 0 {
        return Err("El timeline no comenzó en silencio y posición cero".into());
    }
    output.send(Command::Play)?;
    wait_for(&output, |s| s.peak > 0.001)?;
    let rate = u64::from(output.info.sample_rate);
    let finished = wait_for(&output, |s| {
        s.position >= rate * 2 && s.clip_triggers == 3 && s.peak == 0.0
    })?;
    if finished.musical_ticks < TICKS_PER_BEAT * 4 {
        return Err("El reloj musical no siguió al reloj de muestras".into());
    }
    output.send(Command::SetBpm(Bpm::new(137)?))?;
    wait_for(&output, |s| {
        s.musical_ticks > finished.musical_ticks + 200 && s.clip_triggers == 3 && s.peak == 0.0
    })?;
    output.send(Command::Stop)?;
    wait_for(&output, |s| {
        !s.playing && s.position == 0 && s.musical_ticks == 0 && s.peak == 0.0
    })?;
    output.send(Command::Play)?;
    wait_for(&output, |s| s.clip_triggers > 0 && s.peak > 0.001)?;
    output.send(Command::Stop)?;
    let stopped = wait_for(&output, |s| {
        !s.playing && s.position == 0 && s.musical_ticks == 0 && s.peak == 0.0
    })?;
    println!(
        "OK: tres clips, mover/recortar/snap, reloj musical, silencio tras los clips sin loop y Stop/Play; buffer {} frames; errores {}.",
        stopped.buffer_frames, stopped.errors
    );
    println!("Pendiente escucha humana; la prueba no mide latencia ni captura la salida física.");
    Ok(())
}

fn smoke_project() -> Result<(), Box<dyn Error>> {
    use jner_daw::{
        engine::Engine, midi::MidiPlayer, project::ProjectView, synth::Waveform,
        timeline::TimelinePlayer,
    };
    use std::num::NonZeroU32;
    let directory = tempfile::tempdir()?;
    let first_path = directory.path().join("first.jner");
    let second_path = directory.path().join("save-as.jner");
    let mut original = Project::empty();
    original.bpm = Bpm::new(137)?;
    original.master_gain = 0.13;
    original.pattern = Pattern::demo();
    original.mixer.tracks[0].pan = -0.8;
    original.mixer.tracks[1].gain = 0.4;
    original.timeline_sources = original.sources.to_vec();
    let id = original
        .arrangement
        .add(0, 0, 240, original.sources[0].frame_count())?;
    let edited = original
        .arrangement
        .get(id)
        .ok_or("Falta el clip")?
        .trim_left(240.0, original.sources[0].sample_rate(), original.bpm, true);
    original.arrangement.replace(edited)?;
    original
        .arrangement
        .add(1, 1, 960, original.sources[1].frame_count())?;
    original.workspace.view = ProjectView::Timeline;
    original.workspace.timeline_playback = true;
    original.notes.add(0, 60, 100, 0, 480)?;
    original.notes.add(2, 67, 77, 480, 960)?;
    original.synth.tracks[0].waveform = Waveform::Saw;
    original.synth.tracks[0].cutoff_hz = 3200.0;
    original.synth.tracks[2].waveform = Waveform::Triangle;
    original.synth.tracks[2].release_ms = 350.0;
    original.effects = jner_daw::effects::EffectsState {
        distortion_enabled: true,
        drive: 6.0,
        delay_enabled: true,
        delay_ms: 175.0,
        feedback: 0.6,
        ..Default::default()
    };
    use jner_daw::automation::Target;
    for (target, start, end) in [
        (Target::TrackGain(0), 0.2, 1.6),
        (Target::TrackPan(0), -1.0, 0.8),
        (Target::DelayMix, 0.0, 0.6),
    ] {
        original.automation.add(target, 0, start)?;
        original.automation.add(target, 1920, end)?;
    }
    project::save(&first_path, &original)?;
    let loaded = project::load(&first_path)?;
    project::save(&second_path, &loaded)?;
    let reopened = project::load(&second_path)?;
    if original.bpm != reopened.bpm
        || original.pattern != reopened.pattern
        || original.mixer != reopened.mixer
        || original.workspace != reopened.workspace
        || original.arrangement.clips() != reopened.arrangement.clips()
        || original.notes != reopened.notes
        || original.synth != reopened.synth
        || original.master_gain != reopened.master_gain
        || original.effects != reopened.effects
        || original.automation != reopened.automation
    {
        return Err("La reapertura no conservó los parámetros".into());
    }
    let rate = NonZeroU32::new(48_000).ok_or("Sample rate cero")?;
    let mut a = Engine::with_timeline(
        rate,
        TimelinePlayer::prepare(&original.arrangement, &original.timeline_sources, rate)?,
        original.bpm,
    );
    let mut b = Engine::with_timeline(
        rate,
        TimelinePlayer::prepare(&reopened.arrangement, &reopened.timeline_sources, rate)?,
        reopened.bpm,
    );
    a.configure_effects(original.effects)?;
    b.configure_effects(reopened.effects)?;
    a.configure_automation(&original.automation)?;
    b.configure_automation(&reopened.automation)?;
    for engine in [&mut a, &mut b] {
        engine.apply(Command::SetMixer(original.mixer));
        engine.apply(Command::SetGain(original.master_gain));
        engine.apply(Command::Play);
    }
    let mut audible = false;
    for _ in 0..96_000 {
        let before = a.next_frame();
        let after = b.next_frame();
        if before != after {
            return Err("El audio cambió al guardar y reabrir el proyecto".into());
        }
        audible |= before.iter().any(|v| v.abs() > 0.001);
    }
    if !audible {
        return Err("El proyecto de prueba no generó audio".into());
    }
    let mut a = Engine::with_synth(
        rate,
        MidiPlayer::prepare(&original.notes),
        original.bpm,
        original.synth,
    )?;
    let mut b = Engine::with_synth(
        rate,
        MidiPlayer::prepare(&reopened.notes),
        reopened.bpm,
        reopened.synth,
    )?;
    a.configure_effects(original.effects)?;
    b.configure_effects(reopened.effects)?;
    a.configure_automation(&original.automation)?;
    b.configure_automation(&reopened.automation)?;
    for engine in [&mut a, &mut b] {
        engine.apply(Command::SetMixer(original.mixer));
        engine.apply(Command::SetGain(original.master_gain));
        engine.apply(Command::Play);
    }
    let mut audible = false;
    for _ in 0..96_000 {
        let before = a.next_frame();
        let after = b.next_frame();
        if before != after {
            return Err("El synth cambió al guardar y reabrir".into());
        }
        audible |= before.iter().any(|v| v.abs() > 0.001);
    }
    if !audible {
        return Err("El synth de prueba no generó audio".into());
    }
    println!(
        "OK: guardar, abrir, Guardar como y reabrir; BPM, patrón, mixer, clips, notas, synth, efectos, automatización y fuentes conservados; 96 000 frames estéreo de timeline y 96 000 de synth con efectos/curvas idénticos antes/después."
    );
    println!(
        "Archivo autosuficiente: {} bytes. Prueba sin dispositivo ni emisión de sonido.",
        std::fs::metadata(&second_path)?.len()
    );
    Ok(())
}

fn smoke_midi(index: Option<usize>) -> Result<(), Box<dyn Error>> {
    use jner_daw::midi::NoteSequence;
    let devices = output_devices()?;
    let device = match index {
        Some(i) => devices.get(i).ok_or("Índice de salida no válido")?,
        None => devices
            .iter()
            .find(|d| d.is_default)
            .or(devices.first())
            .ok_or("No hay salida disponible")?,
    };
    let mut notes = NoteSequence::default();
    notes.add(0, 60, 100, 0, 960)?;
    notes.add(1, 64, 80, 480, 960)?;
    notes.add(2, 67, 127, 960, 960)?;
    let mut output = AudioOutput::open_midi(
        &device.device,
        &notes,
        Bpm::default(),
        MixerState::default(),
    )?;
    println!(
        "Salida: {} · {:?}. Eventos internos; no emite sonido.",
        device.name, output.info
    );
    let initial = wait_for(&output, |s| s.callbacks > 0)?;
    if initial.position != 0
        || initial.note_ons != 0
        || initial.active_notes != 0
        || initial.peak != 0.0
    {
        return Err("Estado MIDI inicial incorrecto".into());
    }
    output.send(Command::Play)?;
    wait_for(&output, |s| s.note_ons == 1 && s.active_notes == 1)?;
    output.send(Command::SetBpm(Bpm::new(137)?))?;
    let end = wait_for(&output, |s| s.note_offs == 3)?;
    if end.note_ons != 3 || end.active_notes != 0 || end.musical_ticks < 1920 || end.peak != 0.0 {
        return Err("Eventos MIDI o silencio incorrectos".into());
    }
    output.send(Command::Stop)?;
    wait_for(&output, |s| {
        !s.playing
            && s.position == 0
            && s.musical_ticks == 0
            && s.note_ons == 0
            && s.note_offs == 0
            && s.active_notes == 0
    })?;
    output.send(Command::Play)?;
    wait_for(&output, |s| s.note_ons == 1 && s.active_notes == 1)?;
    output.send(Command::Stop)?;
    let stopped = wait_for(&output, |s| !s.playing && s.active_notes == 0)?;
    println!(
        "OK: tres Note On/Off, cambio de BPM, Stop sin notas activas y reinicio; buffer {} frames; errores CPAL {}.",
        stopped.buffer_frames, stopped.errors
    );
    Ok(())
}

fn smoke_synth(index: Option<usize>) -> Result<(), Box<dyn Error>> {
    use jner_daw::{
        midi::NoteSequence,
        synth::{SynthState, Waveform},
    };
    let devices = output_devices()?;
    let device = match index {
        Some(i) => devices.get(i).ok_or("Índice de salida no válido")?,
        None => devices
            .iter()
            .find(|d| d.is_default)
            .or(devices.first())
            .ok_or("No hay salida disponible")?,
    };
    let mut notes = NoteSequence::default();
    let mut state = SynthState::default();
    for (track, pitch) in [60, 64, 67, 72].into_iter().enumerate() {
        notes.add(track, pitch, 100, 0, 960)?;
        state.tracks[track].waveform = Waveform::ALL[track];
    }
    let mut output = AudioOutput::open_synth(
        &device.device,
        &notes,
        Bpm::default(),
        MixerState::default(),
        state,
    )?;
    println!(
        "Salida: {} · {:?}. Acorde de cuatro ondas a master 8%.",
        device.name, output.info
    );
    let initial = wait_for(&output, |s| s.callbacks > 0)?;
    if initial.peak != 0.0 || initial.synth_voices != 0 || initial.position != 0 {
        return Err("El synth no comenzó en silencio".into());
    }
    output.send(Command::Play)?;
    wait_for(&output, |s| {
        s.note_ons == 4 && s.synth_voices == 4 && s.peak > 0.001
    })?;
    let end = wait_for(&output, |s| {
        s.note_offs == 4 && s.synth_voices == 0 && s.peak == 0.0
    })?;
    if end.active_notes != 0 {
        return Err("Quedaron notas activas después de Note Off".into());
    }
    state.tracks[0].waveform = Waveform::Square;
    state.tracks[0].cutoff_hz = 500.0;
    let rate = std::num::NonZeroU32::new(output.info.sample_rate).ok_or("Sample rate cero")?;
    output.send(Command::SetSynth(state.prepare(rate)?))?;
    output.send(Command::Stop)?;
    wait_for(&output, |s| {
        !s.playing && s.synth_voices == 0 && s.position == 0
    })?;
    output.send(Command::Play)?;
    wait_for(&output, |s| s.note_ons == 4 && s.peak > 0.001)?;
    output.send(Command::Stop)?;
    let stopped = wait_for(&output, |s| {
        !s.playing && s.synth_voices == 0 && s.peak == 0.0
    })?;
    println!(
        "OK: cuatro ondas/voces, ADSR con cola hasta silencio, cambio de patch, Stop y reinicio; buffer {} frames; errores CPAL {}. Pendiente escucha humana.",
        stopped.buffer_frames, stopped.errors
    );
    Ok(())
}

fn smoke_effects(index: Option<usize>) -> Result<(), Box<dyn Error>> {
    use jner_daw::effects::EffectsState;
    let devices = output_devices()?;
    let device = match index {
        Some(i) => devices.get(i).ok_or("Índice de salida no válido")?,
        None => devices
            .iter()
            .find(|d| d.is_default)
            .or(devices.first())
            .ok_or("No hay salida disponible")?,
    };
    let sources = starter_kit();
    let mut output = AudioOutput::open_drums_with_effects(
        &device.device,
        &sources,
        Pattern::demo(),
        Bpm::default(),
        MixerState::default(),
        EffectsState::default(),
    )?;
    println!(
        "Salida: {} · {:?}. Beat con efectos, master 8%.",
        device.name, output.info
    );
    let initial = wait_for(&output, |s| s.callbacks > 0)?;
    if initial.peak != 0.0 || initial.position != 0 {
        return Err("Los efectos no comenzaron en silencio".into());
    }
    let rate = std::num::NonZeroU32::new(output.info.sample_rate).ok_or("Sample rate cero")?;
    output.send(Command::Play)?;
    let mut previous = wait_for(&output, |s| {
        s.position > u64::from(rate.get()) / 10 && s.peak > 0.001
    })?;
    let mut state = EffectsState {
        distortion_enabled: true,
        drive: 8.0,
        distortion_mix: 0.7,
        delay_enabled: true,
        delay_ms: 120.0,
        feedback: 0.75,
        delay_mix: 0.6,
    };
    for delay in [120.0, 500.0, 80.5, 180.0] {
        state.delay_ms = delay;
        state.drive = if delay < 200.0 { 12.0 } else { 2.0 };
        output.send(Command::SetEffects(state.prepare(rate)?))?;
        let start = previous;
        previous = wait_for(&output, |s| {
            s.position > start.position + u64::from(rate.get()) / 4
        })?;
        if !previous.playing
            || previous.callbacks <= start.callbacks
            || !previous.peak.is_finite()
            || previous.peak > 1.0
        {
            return Err(
                "La edición de efectos interrumpió el stream o produjo un buffer no válido".into(),
            );
        }
    }
    output.send(Command::SetPattern(Pattern::default()))?;
    let clear = previous.position;
    wait_for(&output, |s| {
        s.position > clear + u64::from(rate.get()) && s.peak > 1e-6
    })?;
    output.send(Command::SetEffects(EffectsState::default().prepare(rate)?))?;
    let bypassed = wait_for(&output, |s| {
        s.position > clear + u64::from(rate.get()) + u64::from(rate.get()) / 10 && s.peak == 0.0
    })?;
    if !bypassed.playing {
        return Err("El bypass detuvo el transport".into());
    }
    output.send(Command::Stop)?;
    wait_for(&output, |s| !s.playing && s.position == 0 && s.peak == 0.0)?;
    output.send(Command::SetEffects(state.prepare(rate)?))?;
    output.send(Command::Play)?;
    let restarted = wait_for(&output, |s| s.position > u64::from(rate.get()) / 2)?;
    if restarted.peak != 0.0 {
        return Err("El delay conservó audio anterior tras Stop/Play".into());
    }
    output.send(Command::Stop)?;
    let stopped = wait_for(&output, |s| !s.playing && s.position == 0 && s.peak == 0.0)?;
    println!(
        "OK: delay/distorsión, cuatro cambios en vivo sin reiniciar reloj/callbacks, cola tras vaciar patrón, bypass y Stop sin eco residual; buffer {} frames; errores CPAL {}. Pendiente escucha humana.",
        stopped.buffer_frames, stopped.errors
    );
    Ok(())
}

fn smoke_automation(index: Option<usize>) -> Result<(), Box<dyn Error>> {
    use jner_daw::{
        audio::PlaybackSettings,
        automation::{Automation, Target},
        effects::EffectsState,
        synth::SynthState,
    };
    let devices = output_devices()?;
    let device = match index {
        Some(i) => devices.get(i).ok_or("Índice de salida no válido")?,
        None => devices
            .iter()
            .find(|d| d.is_default)
            .or(devices.first())
            .ok_or("No hay salida disponible")?,
    };
    let mut curves = Automation::default();
    curves.add(Target::TrackGain(0), 0, 0.0)?;
    curves.add(Target::TrackGain(0), 960, 1.5)?;
    curves.add(Target::TrackGain(0), 1920, 0.0)?;
    for (target, start, end) in [
        (Target::TrackPan(0), -1.0, 1.0),
        (Target::DelayMix, 0.0, 0.6),
    ] {
        curves.add(target, 0, start)?;
        curves.add(target, 1920, end)?;
    }
    let settings = PlaybackSettings {
        bpm: Bpm::default(),
        mixer: MixerState::default(),
        synth: SynthState::default(),
        effects: EffectsState {
            delay_enabled: true,
            ..Default::default()
        },
        automation: &curves,
        record_clock: None,
    };
    let sources = starter_kit();
    let mut output = AudioOutput::open_drums_with_settings(
        &device.device,
        &sources,
        Pattern::demo(),
        &settings,
    )?;
    println!(
        "Salida: {} · {:?}. Beat automatizado, master 8%.",
        device.name, output.info
    );
    let initial = wait_for(&output, |s| s.callbacks > 0)?;
    if initial.peak != 0.0 || initial.automation_values.iter().any(Option::is_some) {
        return Err("Estado inicial de automatización incorrecto".into());
    }
    let rate = u64::from(output.info.sample_rate);
    output.send(Command::Play)?;
    let first = wait_for(&output, |s| {
        s.position > rate / 5 && s.automation_values[0].is_some_and(|v| v > 0.0 && v < 1.5)
    })?;
    if first.automation_values.iter().flatten().count() != 3
        || !first.automation_values[4].is_some_and(|v| v < 0.0)
        || !first.automation_values[8].is_some_and(|v| v > 0.0)
    {
        return Err("Las tres curvas no avanzaron juntas".into());
    }
    output.send(Command::SetBpm(Bpm::new(240)?))?;
    let end = wait_for(&output, |s| s.musical_ticks > 1920)?;
    if end.automation_values[0] != Some(0.0)
        || end.automation_values[4] != Some(1.0)
        || end.automation_values[8] != Some(0.6)
        || end.callbacks <= first.callbacks
        || end.position <= first.position
    {
        return Err("Curvas, BPM o continuidad de callbacks incorrectos".into());
    }
    output.send(Command::Stop)?;
    wait_for(&output, |s| {
        !s.playing
            && s.position == 0
            && s.peak == 0.0
            && s.automation_values.iter().all(Option::is_none)
    })?;
    output.send(Command::Play)?;
    let restart = wait_for(&output, |s| {
        s.position > rate / 10 && s.automation_values[0].is_some_and(|v| v > 0.0 && v < 1.5)
    })?;
    if !restart.automation_values[4].is_some_and(|v| v < 0.0) {
        return Err("La automatización no reinició desde cero".into());
    }
    output.send(Command::Stop)?;
    let stopped = wait_for(&output, |s| !s.playing && s.position == 0 && s.peak == 0.0)?;
    println!(
        "OK: volumen, pan y mezcla DSP; interpolación, cambio de BPM, extremos, Stop/manual y reinicio; buffer {} frames; errores CPAL {}. Pendiente escucha humana.",
        stopped.buffer_frames, stopped.errors
    );
    Ok(())
}
