//! Piper: natural neural voices that run offline (ONNX models).
//!
//! Voices are the same files the Python app uses, in ~/.textReader/voices/piper,
//! downloaded from the official Piper voice repository on first use.

use super::{Build, Engine, Voice, espeak};
use crate::settings::data_dir;
use anyhow::{Context, Result, anyhow};
use ort::session::Session;
use ort::value::Tensor;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

const VOICES: [(&str, &str); 15] = [
    ("en_US-lessac-medium", "Lessac (US, female)"),
    ("en_US-amy-medium", "Amy (US, female)"),
    ("en_US-kristin-medium", "Kristin (US, female)"),
    ("en_US-kathleen-medium", "Kathleen (US, female)"),
    ("en_US-hfc_female-medium", "HFC (US, female)"),
    ("en_US-ryan-medium", "Ryan (US, male)"),
    ("en_US-joe-medium", "Joe (US, male)"),
    ("en_US-john-medium", "John (US, male)"),
    ("en_US-bryce-medium", "Bryce (US, male)"),
    ("en_US-hfc_male-medium", "HFC (US, male)"),
    ("en_GB-jenny_dioco-medium", "Jenny (UK, female)"),
    ("en_GB-cori-medium", "Cori (UK, female)"),
    ("en_GB-alba-medium", "Alba (Scotland, female)"),
    ("en_GB-alan-medium", "Alan (UK, male)"),
    ("en_GB-northern_english_male-medium", "Northern (UK, male)"),
];

const URL: &str = "https://huggingface.co/rhasspy/piper-voices/resolve/main/en";

fn voice_dir() -> PathBuf {
    data_dir().join("voices").join("piper")
}

fn model_path(name: &str) -> PathBuf {
    voice_dir().join(format!("{name}.onnx"))
}

struct Model {
    session: Session,
    ids: HashMap<String, Vec<i64>>,
    sample_rate: u32,
    espeak_voice: String,
    scales: [f32; 3], // noise, length, noise_w
}

#[derive(Default)]
pub struct Piper {
    models: Mutex<HashMap<String, Arc<Mutex<Model>>>>,
}

static ENV: OnceLock<std::result::Result<ort::environment::Environment, String>> = OnceLock::new();

fn environment() -> Result<&'static ort::environment::Environment> {
    ENV.get_or_init(|| ort::init().with_name("clipboard-reader").build().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| anyhow!("ONNX Runtime didn't start: {e}"))
}

fn download(name: &str, build: &Build) -> Result<()> {
    let mut parts = name.splitn(3, '-');
    let (locale, speaker, quality) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    std::fs::create_dir_all(voice_dir())?;
    for ext in [".onnx.json", ".onnx"] {
        let dest = voice_dir().join(format!("{name}{ext}"));
        if dest.exists() {
            continue;
        }
        let url = format!("{URL}/{locale}/{speaker}/{quality}/{name}{ext}?download=true");
        let response = ureq::get(&url).header("User-Agent", "textReader").call().context("voice download failed")?;
        let total: u64 = response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(63_200_000);
        let part = dest.with_extension(format!("{}.part", dest.extension().unwrap().to_string_lossy()));
        let mut out = std::fs::File::create(&part)?;
        let mut reader = response.into_body().into_reader();
        let mut buf = vec![0u8; 256 * 1024];
        let mut done: u64 = 0;
        loop {
            if build.cancelled() {
                drop(out);
                let _ = std::fs::remove_file(&part);
                if !model_path(name).exists() {
                    let _ = std::fs::remove_file(voice_dir().join(format!("{name}.onnx.json")));
                }
                return Err(anyhow!(super::Cancelled));
            }
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            done += n as u64;
            if ext == ".onnx" {
                (build.progress)(format!("Downloading voice: {:.1} of {:.0} MB", done as f64 / 1e6, total.max(done) as f64 / 1e6));
            }
        }
        drop(out);
        std::fs::rename(&part, &dest)?;
    }
    Ok(())
}

fn load_model(name: &str) -> Result<Model> {
    let config: Value = serde_json::from_str(&std::fs::read_to_string(voice_dir().join(format!("{name}.onnx.json")))?)?;
    let ids = config["phoneme_id_map"]
        .as_object()
        .context("voice config has no phoneme_id_map")?
        .iter()
        .map(|(k, v)| (k.clone(), v.as_array().map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default()))
        .collect();
    let inference = &config["inference"];
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let session = Session::builder(environment()?)?
        .with_intra_threads(threads)
        .map_err(|e| anyhow!("{e}"))?
        .commit_from_file(model_path(name))
        .map_err(|e| anyhow!("couldn't load the voice: {e}"))?;
    Ok(Model {
        session,
        ids,
        sample_rate: config["audio"]["sample_rate"].as_u64().unwrap_or(22050) as u32,
        espeak_voice: config["espeak"]["voice"].as_str().unwrap_or("en-us").to_string(),
        scales: [
            inference["noise_scale"].as_f64().unwrap_or(0.667) as f32,
            inference["length_scale"].as_f64().unwrap_or(1.0) as f32,
            inference["noise_w"].as_f64().unwrap_or(0.8) as f32,
        ],
    })
}

impl Model {
    fn phoneme_ids(&self, phonemes: &[char]) -> Vec<i64> {
        let get = |s: &str| self.ids.get(s).cloned().unwrap_or_default();
        let pad = get("_");
        let mut ids = get("^");
        ids.extend(&pad);
        for ch in phonemes {
            if let Some(id) = self.ids.get(&ch.to_string()) {
                ids.extend(id);
                ids.extend(&pad);
            }
        }
        ids.extend(get("$"));
        ids
    }

    fn speak(&mut self, ids: Vec<i64>) -> Result<Vec<f32>> {
        let n = ids.len();
        let outputs = self.session.run(ort::inputs![
            "input" => Tensor::from_array(([1usize, n], ids))?,
            "input_lengths" => Tensor::from_array(([1usize], vec![n as i64]))?,
            "scales" => Tensor::from_array(([3usize], self.scales.to_vec()))?,
        ])?;
        let (_, audio) = outputs[0].try_extract_tensor::<f32>()?;
        Ok(audio.to_vec())
    }
}

impl Engine for Piper {
    fn name(&self) -> &'static str {
        "Piper"
    }
    fn ext(&self) -> &'static str {
        "wav"
    }
    fn quality(&self) -> u8 {
        4
    }
    fn about(&self) -> &'static str {
        "Fast neural voices that run entirely on your computer. Each voice is a separate ~63 MB download."
    }
    fn download_state(&self, voice: &str) -> Option<(bool, u64)> {
        let files = [voice_dir().join(format!("{voice}.onnx")), voice_dir().join(format!("{voice}.onnx.json"))];
        if files[0].exists() {
            Some((true, files.iter().filter_map(|f| std::fs::metadata(f).ok()).map(|m| m.len()).sum()))
        } else {
            Some((false, 63_200_000))
        }
    }
    fn download(&self, voice: &str, build: &Build) -> Result<()> {
        download(voice, build)
    }
    fn delete(&self, voice: &str) -> Result<()> {
        self.models.lock().unwrap().remove(voice);
        for ext in [".onnx", ".onnx.json"] {
            let _ = std::fs::remove_file(voice_dir().join(format!("{voice}{ext}")));
        }
        Ok(())
    }
    fn missing(&self) -> Option<String> {
        espeak::phonemize("en-us", "test").err().map(|e| e.to_string())
    }
    fn voices(&self) -> Vec<Voice> {
        VOICES
            .iter()
            .map(|(id, label)| {
                let label = if model_path(id).exists() { label.to_string() } else { format!("{label} · download") };
                (id.to_string(), label)
            })
            .collect()
    }
    fn synthesize(&self, text: &str, voice: &str, out: &Path, build: &Build) -> Result<()> {
        let name = VOICES.iter().find(|(id, _)| *id == voice).map(|(id, _)| *id).unwrap_or(VOICES[0].0);
        if !model_path(name).exists() {
            download(name, build)?;
        }
        build.check()?;
        let model = {
            let mut models = self.models.lock().unwrap();
            match models.get(name) {
                Some(m) => m.clone(),
                None => {
                    (build.progress)("Loading voice…".into());
                    let m = Arc::new(Mutex::new(load_model(name)?));
                    models.insert(name.to_string(), m.clone());
                    m
                }
            }
        };
        let mut model = model.lock().unwrap();
        let sentences = espeak::phonemize(&model.espeak_voice, text)?;
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: model.sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut wav = hound::WavWriter::create(out, spec)?;
        for phonemes in sentences {
            build.check()?;
            let ids = model.phoneme_ids(&phonemes);
            let audio = model.speak(ids)?;
            // Normalize each sentence to full volume, as Piper does
            let peak = audio.iter().fold(0f32, |m, s| m.max(s.abs()));
            let gain = if peak > 1e-8 { 1.0 / peak } else { 0.0 };
            for sample in audio {
                wav.write_sample(((sample * gain).clamp(-1.0, 1.0) * 32767.0) as i16)?;
            }
        }
        wav.finalize()?;
        Ok(())
    }
}
