//! Edge TTS: Microsoft's neural voices (Edge's Read Aloud service), over a WebSocket.
//! Ported from the edge-tts Python package.

use super::edge_drm::sec_ms_gec;
use super::{Build, Cancelled, Engine, Voice};
use crate::settings::data_dir;
use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use std::io::Write;
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tungstenite::client::IntoClientRequest;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::Message;

const BASE: &str = "speech.platform.bing.com/consumer/speech/synthesize/readaloud";
const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const CHROMIUM_FULL_VERSION: &str = "143.0.3650.75";
const CHROMIUM_MAJOR: &str = "143";

/// Used until the online list has been fetched (or if it can't be)
const FALLBACK: [(&str, &str); 11] = [
    ("en-US-AriaNeural", "Aria (US, female)"),
    ("en-US-JennyNeural", "Jenny (US, female)"),
    ("en-US-GuyNeural", "Guy (US, male)"),
    ("en-US-ChristopherNeural", "Christopher (US, male)"),
    ("en-GB-SoniaNeural", "Sonia (UK, female)"),
    ("en-GB-RyanNeural", "Ryan (UK, male)"),
    ("en-AU-NatashaNeural", "Natasha (Australia, female)"),
    ("en-AU-WilliamNeural", "William (Australia, male)"),
    ("en-CA-ClaraNeural", "Clara (Canada, female)"),
    ("en-IE-EmilyNeural", "Emily (Ireland, female)"),
    ("en-IN-NeerjaNeural", "Neerja (India, female)"),
];

const REGIONS: [(&str, &str); 14] = [
    ("US", "US"),
    ("GB", "UK"),
    ("AU", "Australia"),
    ("CA", "Canada"),
    ("IE", "Ireland"),
    ("IN", "India"),
    ("NZ", "New Zealand"),
    ("ZA", "South Africa"),
    ("SG", "Singapore"),
    ("PH", "Philippines"),
    ("NG", "Nigeria"),
    ("KE", "Kenya"),
    ("TZ", "Tanzania"),
    ("HK", "Hong Kong"),
];

fn user_agent() -> String {
    format!(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{m}.0.0.0 Safari/537.36 Edg/{m}.0.0.0",
        m = CHROMIUM_MAJOR
    )
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn token_query() -> String {
    format!("&Sec-MS-GEC={}&Sec-MS-GEC-Version=2-{CHROMIUM_FULL_VERSION}", sec_ms_gec(now()))
}

fn muid() -> String {
    uuid::Uuid::new_v4().simple().to_string().to_uppercase()
}

/// "en-US-AndrewMultilingualNeural" -> "Andrew Multilingual (US, male)"
fn label(short_name: &str, gender: &str) -> String {
    let mut parts = short_name.splitn(3, '-');
    let (_, region, name) = (parts.next(), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let mut name = name.replace("Neural", "");
    for word in ["Multilingual", "Expressive"] {
        name = name.replace(word, &format!(" {word}"));
    }
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let region = REGIONS.iter().find(|(code, _)| *code == region).map(|(_, r)| *r).unwrap_or(region);
    format!("{name} ({region}, {})", gender.to_lowercase())
}

fn sort_key(voice: &Voice) -> (usize, usize, String) {
    if let Some(i) = FALLBACK.iter().position(|(id, _)| *id == voice.0) {
        return (0, i, String::new());
    }
    let order = ["en-US", "en-GB", "en-AU", "en-CA", "en-IE"];
    let prefix = &voice.0[..voice.0.len().min(5)];
    (1, order.iter().position(|p| *p == prefix).unwrap_or(order.len()), voice.1.clone())
}

/// Microsoft's English voice list. Shares the Python app's weekly cache file.
fn fetch_voice_list() -> Option<Vec<Value>> {
    let cache = data_dir().join("edge_voices_v2.json");
    let fresh = std::fs::metadata(&cache)
        .and_then(|m| m.modified())
        .map(|t| t.elapsed().unwrap_or_default() < Duration::from_secs(7 * 24 * 3600))
        .unwrap_or(false);
    if fresh {
        if let Some(list) = std::fs::read_to_string(&cache).ok().and_then(|t| serde_json::from_str::<Vec<Value>>(&t).ok()) {
            if !list.is_empty() {
                return Some(list);
            }
        }
    }
    let url = format!("https://{BASE}/voices/list?trustedclienttoken={TRUSTED_CLIENT_TOKEN}{}", token_query());
    let mut response = ureq::get(&url)
        .header("User-Agent", user_agent())
        .header("Cookie", format!("muid={};", muid()))
        .config()
        .timeout_global(Some(Duration::from_secs(8)))
        .build()
        .call()
        .ok()?;
    let all: Vec<Value> = serde_json::from_str(&response.body_mut().read_to_string().ok()?).ok()?;
    let english: Vec<Value> =
        all.into_iter().filter(|v| v["Locale"].as_str().unwrap_or("").starts_with("en-")).collect();
    if !english.is_empty() {
        let _ = std::fs::write(&cache, serde_json::to_string(&english).unwrap_or_default());
    }
    Some(english)
}

fn xml_escape(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            // The service rejects most control characters (e.g. vertical tabs in OCR-ed PDFs)
            c if (c as u32) < 32 && !matches!(c, '\t' | '\n' | '\r') => " ".to_string(),
            c => c.to_string(),
        })
        .collect()
}

/// Split escaped text into pieces of at most `max` bytes, at spaces where possible,
/// never inside an "&amp;"-style entity.
fn split_bytes(text: &str, max: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut rest = text.trim();
    while rest.len() > max {
        let mut cut = max;
        while !rest.is_char_boundary(cut) {
            cut -= 1;
        }
        if let Some(space) = rest[..cut].rfind([' ', '\n']) {
            if space > 0 {
                cut = space;
            }
        }
        if let Some(amp) = rest[..cut].rfind('&') {
            if !rest[amp..cut].contains(';') && amp > 0 {
                cut = amp;
            }
        }
        pieces.push(rest[..cut].trim().to_string());
        rest = rest[cut..].trim_start();
    }
    if !rest.is_empty() {
        pieces.push(rest.to_string());
    }
    pieces.into_iter().filter(|p| !p.is_empty()).collect()
}

/// "Thu Nov 19 2026 03:40:00 GMT+0000 (Coordinated Universal Time)"
fn js_date() -> String {
    let secs = now() as i64;
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    // Civil date from days since 1970 (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    let weekday = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][days.rem_euclid(7) as usize];
    let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!(
        "{weekday} {} {day:02} {year} {:02}:{:02}:{:02} GMT+0000 (Coordinated Universal Time)",
        months[(month - 1) as usize],
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// "en-US-AriaNeural" -> "Microsoft Server Speech Text to Speech Voice (en-US, AriaNeural)"
fn full_voice_name(voice: &str) -> String {
    let parts: Vec<&str> = voice.splitn(3, '-').collect();
    if parts.len() == 3 && voice.ends_with("Neural") {
        let (lang, mut region, mut name) = (parts[0], parts[1].to_string(), parts[2].to_string());
        if let Some(dash) = name.find('-') {
            region = format!("{region}-{}", &name[..dash]);
            name = name[dash + 1..].to_string();
        }
        return format!("Microsoft Server Speech Text to Speech Voice ({lang}-{region}, {name})");
    }
    voice.to_string()
}

fn speak_piece(piece: &str, voice: &str, out: &mut std::fs::File, build: &Build) -> Result<usize> {
    let url = format!(
        "wss://{BASE}/edge/v1?TrustedClientToken={TRUSTED_CLIENT_TOKEN}&ConnectionId={}{}",
        uuid::Uuid::new_v4().simple(),
        token_query()
    );
    let mut request = url.into_client_request()?;
    let headers = request.headers_mut();
    for (name, value) in [
        ("Pragma", "no-cache".to_string()),
        ("Cache-Control", "no-cache".to_string()),
        ("Origin", "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold".to_string()),
        ("User-Agent", user_agent()),
        ("Accept-Encoding", "gzip, deflate, br, zstd".to_string()),
        ("Accept-Language", "en-US,en;q=0.9".to_string()),
        ("Cookie", format!("muid={};", muid())),
    ] {
        headers.insert(name, value.parse()?);
    }
    let (mut socket, _) = tungstenite::connect(request).context("couldn't connect to Microsoft's speech service")?;
    // Wake up regularly while waiting, to notice Stop
    let timeout = Some(Duration::from_millis(500));
    match socket.get_mut() {
        MaybeTlsStream::Plain(s) => s.set_read_timeout(timeout)?,
        MaybeTlsStream::Rustls(s) => s.sock.set_read_timeout(timeout)?,
        _ => {}
    }
    socket.send(Message::text(format!(
        "X-Timestamp:{}\r\nContent-Type:application/json; charset=utf-8\r\nPath:speech.config\r\n\r\n\
         {{\"context\":{{\"synthesis\":{{\"audio\":{{\"metadataoptions\":{{\
         \"sentenceBoundaryEnabled\":\"true\",\"wordBoundaryEnabled\":\"false\"}},\
         \"outputFormat\":\"audio-24khz-48kbitrate-mono-mp3\"}}}}}}}}\r\n",
        js_date()
    )))?;
    let ssml = format!(
        "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='en-US'>\
         <voice name='{}'><prosody pitch='+0Hz' rate='+0%' volume='+0%'>{piece}</prosody></voice></speak>",
        full_voice_name(voice)
    );
    socket.send(Message::text(format!(
        "X-RequestId:{}\r\nContent-Type:application/ssml+xml\r\nX-Timestamp:{}Z\r\nPath:ssml\r\n\r\n{ssml}",
        uuid::Uuid::new_v4().simple(),
        js_date()
    )))?;
    let started = std::time::Instant::now();
    let mut written = 0;
    loop {
        if build.cancelled() {
            let _ = socket.close(None);
            return Err(anyhow!(Cancelled));
        }
        if started.elapsed() > Duration::from_secs(60) {
            return Err(anyhow!("Microsoft's speech service stopped answering"));
        }
        let message = match socket.read() {
            Ok(m) => m,
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) =>
            {
                continue
            }
            Err(e) => return Err(anyhow!("speech service connection failed: {e}")),
        };
        match message {
            Message::Text(text) => {
                if text.contains("Path:turn.end") {
                    let _ = socket.close(None);
                    return Ok(written);
                }
            }
            Message::Binary(data) => {
                // 2-byte header length, the headers, then the audio
                if data.len() < 2 {
                    continue;
                }
                let header_len = u16::from_be_bytes([data[0], data[1]]) as usize;
                if data.len() < 2 + header_len {
                    continue;
                }
                let headers = String::from_utf8_lossy(&data[2..2 + header_len]);
                if headers.contains("Path:audio") && headers.contains("Content-Type:audio/mpeg") {
                    out.write_all(&data[2 + header_len..])?;
                    written += data.len() - 2 - header_len;
                }
            }
            Message::Close(_) => return Ok(written),
            _ => {}
        }
    }
}

#[derive(Default)]
pub struct Edge {
    voices: OnceLock<Vec<Voice>>,
}

impl Engine for Edge {
    fn name(&self) -> &'static str {
        "Edge TTS"
    }
    fn ext(&self) -> &'static str {
        "mp3"
    }
    fn missing(&self) -> Option<String> {
        None
    }
    fn online(&self) -> Option<&'static str> {
        Some("Microsoft")
    }
    fn quality(&self) -> u8 {
        5
    }
    fn about(&self) -> &'static str {
        "Microsoft's neural voices, the ones behind Edge's Read Aloud. The most natural-sounding option, with many accents."
    }
    fn voices(&self) -> Vec<Voice> {
        self.voices
            .get_or_init(|| match fetch_voice_list() {
                Some(list) if !list.is_empty() => {
                    let mut voices: Vec<Voice> = list
                        .iter()
                        .filter_map(|v| {
                            let id = v["ShortName"].as_str()?;
                            Some((id.to_string(), label(id, v["Gender"].as_str().unwrap_or(""))))
                        })
                        .collect();
                    voices.sort_by_key(sort_key);
                    voices
                }
                _ => FALLBACK.iter().map(|(id, l)| (id.to_string(), l.to_string())).collect(),
            })
            .clone()
    }
    fn synthesize(&self, text: &str, voice: &str, out: &Path, build: &Build) -> Result<()> {
        let voice = if voice.is_empty() { FALLBACK[0].0 } else { voice };
        let mut file = std::fs::File::create(out)?;
        let mut total = 0;
        for piece in split_bytes(&xml_escape(text), 4096) {
            build.check()?;
            total += speak_piece(&piece, voice, &mut file, build)?;
        }
        if total == 0 {
            return Err(anyhow!("Microsoft's speech service sent no audio"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_names() {
        assert_eq!(full_voice_name("en-US-AriaNeural"), "Microsoft Server Speech Text to Speech Voice (en-US, AriaNeural)");
        assert_eq!(label("en-US-AndrewMultilingualNeural", "Male"), "Andrew Multilingual (US, male)");
    }

    #[test]
    fn splitting_respects_limit_and_entities() {
        let text = xml_escape(&"fish & chips ".repeat(800));
        let pieces = split_bytes(&text, 4096);
        assert!(pieces.len() > 1);
        for p in &pieces {
            assert!(p.len() <= 4096);
            assert!(!p.ends_with('&') && !p.contains("&am ") && p.matches('&').count() == p.matches("&amp;").count());
        }
    }

    #[test]
    fn date_format() {
        let d = js_date();
        assert!(d.ends_with("GMT+0000 (Coordinated Universal Time)"), "{d}");
        assert_eq!(d.split(' ').count(), 9, "{d}");
    }
}
